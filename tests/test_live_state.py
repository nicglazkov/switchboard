"""Live state (docs/specs/2026-10-14-live-state.md): the server's derive() is the one place a chat's activity, cache
and size warning are decided; the agent only reports raw signals (its cache end, real or measured, and its clock)."""
import json, sys, unittest
from pathlib import Path
from tempfile import mkdtemp

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / 'server'))
import fleet, server  # noqa: E402
from tests.test_rpc import RpcBase, NOW, SID_A  # noqa: E402
from tests.test_server import MACH  # noqa: E402

H = 3600
FIXTURE = Path(__file__).resolve().parents[0] / 'fixtures' / 'live-state.json'


def s(**kw):
    return dict({'sid': SID_A, 'kind': 'background', 'status': 'idle', 'active': NOW - 60, 'mtime': NOW - 60, 'ctx': 40_000}, **kw)


def act(row, **kw):
    return fleet.derive(row, NOW, **kw)['activity']


class DeriveActivityTest(unittest.TestCase):
    def test_each_activity(self):
        self.assertEqual(act(s()), 'quiet')
        self.assertEqual(act(s(status='busy')), 'working')
        self.assertEqual(act(s(state='running', status=None)), 'working')
        self.assertEqual(act(s(status='waiting')), 'needs')
        self.assertEqual(act(s(state='blocked')), 'needs')
        self.assertEqual(act(s(state='blocked', status='busy')), 'working', 'blocked while busy is a tool running')
        self.assertEqual(act(s(), held=True), 'held')
        self.assertEqual(act(s(status='stopped', state='blocked')), 'quiet')

    def test_precedence_pairs(self):
        # held > needs/parked > working/stalled > quiet, every pair
        self.assertEqual(act(s(status='waiting'), held=True), 'held')
        self.assertEqual(act(s(status='waiting', active=NOW - 30 * H), held=True), 'held', 'a held chat is never parked')
        self.assertEqual(act(s(status='busy'), held=True), 'held')
        self.assertEqual(act(s(status='busy', mtime=NOW - 7 * H, active=NOW - 7 * H), held=True), 'held')
        self.assertEqual(act(s(status='waiting', state='running')), 'needs')
        self.assertEqual(act(s(status='waiting', state='running', active=NOW - 30 * H, mtime=NOW - 30 * H)), 'parked')
        self.assertEqual(act(s(state='blocked', status='idle', turn_open=True)), 'needs', 'needs beats an open turn')
        self.assertEqual(act(s(status='busy', mtime=NOW - 7 * H, active=NOW - 7 * H)), 'stalled')
        self.assertEqual(act(s(status='busy')), 'working')

    def test_parked_and_stalled_timers(self):
        self.assertEqual(act(s(status='waiting', active=NOW - 24 * H + 60)), 'needs')
        self.assertEqual(act(s(status='waiting', active=NOW - 24 * H - 60)), 'parked')
        self.assertEqual(act(s(status='busy', active=NOW - 6 * H + 60, mtime=NOW - 6 * H + 60)), 'working')
        self.assertEqual(act(s(status='busy', active=NOW - 6 * H - 60, mtime=NOW - 6 * H - 60)), 'stalled')
        # its subagents writing is progress
        self.assertEqual(act(s(status='busy', active=NOW - 9 * H, mtime=NOW - 9 * H, child_ts=NOW - 60)), 'working')
        # an idle Claude Code touching its transcript (mtime) with nothing new is not
        self.assertEqual(act(s(status='busy', active=NOW - 9 * H, last_ts=NOW - 9 * H, mtime=NOW - 60)), 'stalled')

    def test_turn_open(self):
        # A terminal chat's background shells keep Claude Code saying busy after its turn ended.
        self.assertEqual(act(s(kind='interactive', status='busy', turn_open=False)), 'quiet')
        self.assertEqual(act(s(kind='interactive', status='idle', turn_open=True)), 'working')

    def test_a_background_chat_whose_turn_ended_is_quiet(self):
        # A server it started and left running kept a background chat "Working" for a day (Kitchen remodel, 10-15).
        self.assertEqual(act(s(kind='background', status='busy', state='done', turn_open=False)), 'quiet')
        # its subagents or commands still writing: it is still working, though its own turn is over
        self.assertEqual(act(s(kind='background', status='busy', turn_open=False, child_ts=NOW - 60)), 'working')
        self.assertEqual(act(s(kind='background', status='busy', turn_open=False, child_ts=NOW - 16 * 60)), 'quiet')
        # no turn reading (an older agent): Claude Code's word stands
        self.assertEqual(act(s(kind='background', status='busy')), 'working')

    def test_hook_pings(self):
        self.assertEqual(act(s(), hook={'why': 'prompt', 'at': NOW}), 'working')
        self.assertEqual(act(s(status='busy'), hook={'why': 'stop', 'at': NOW}), 'quiet')
        self.assertEqual(act(s(status='busy'), hook={'why': 'wait', 'at': NOW}), 'needs')
        self.assertEqual(act(s(status='waiting'), hook={'why': 'prompt', 'at': NOW}), 'working', 'the answer went in')
        self.assertEqual(act(s(status='stopped'), hook={'why': 'prompt', 'at': NOW}), 'quiet')
        self.assertEqual(act(s(), held=True, hook={'why': 'prompt', 'at': NOW}), 'held')
        # a prompt is progress: a chat busy for hours is working again
        self.assertEqual(act(s(status='busy', active=NOW - 9 * H, mtime=NOW - 9 * H), hook={'why': 'prompt', 'at': NOW - 5}), 'working')


class DeriveCacheTest(unittest.TestCase):
    def test_warm_cold_unknown(self):
        d = fleet.derive(s(cache_expires_at=NOW + 250), NOW)
        self.assertEqual((d['cache'], d['cold_at']), ('warm', NOW + 250))
        d = fleet.derive(s(cache_expires_at=NOW - 5), NOW)
        self.assertEqual((d['cache'], d['cold_at']), ('cold', NOW - 5))
        d = fleet.derive(s(cache_expires_at=None), NOW)
        self.assertEqual((d['cache'], d['cold_at']), ('unknown', None), 'never a guess')
        # reported for every activity, not only quiet
        self.assertEqual(fleet.derive(s(status='busy', cache_expires_at=NOW + 9), NOW)['cache'], 'warm')

    def test_heavy_is_its_own_fact(self):
        self.assertTrue(fleet.derive(s(ctx=150_000), NOW)['heavy'])
        self.assertFalse(fleet.derive(s(ctx=149_999), NOW)['heavy'])
        d = fleet.derive(s(ctx=312_000, cache_expires_at=NOW - 60), NOW)
        self.assertEqual((d['cache'], d['heavy'], d['activity']), ('cold', True, 'quiet'))

    def test_skew_converts_to_the_server_clock(self):
        # The machine's clock runs 40 s behind: its timestamps are 40 s early on ours.
        d = fleet.derive(s(cache_expires_at=NOW + 10 - 40), NOW, skew=40)
        self.assertEqual((d['cache'], d['cold_at']), ('warm', NOW + 10))
        self.assertEqual(act(s(status='waiting', active=NOW - 24 * H + 20), skew=-60), 'parked')

    def test_older_agents(self):
        # No cache_expires_at: its cache_left counted from when it answered, or its cold flag.
        d = fleet.derive({'sid': SID_A, 'status': 'idle', 'ctx': 9, 'cache_left': 300}, NOW, updated=NOW - 100)
        self.assertEqual((d['cache'], d['cold_at']), ('warm', NOW + 200))
        self.assertEqual(fleet.derive({'sid': SID_A, 'cold': True, 'ctx': 9}, NOW)['cache'], 'cold')
        self.assertEqual(fleet.derive({'sid': SID_A, 'cold': False, 'ctx': 9}, NOW)['cache'], 'unknown')

    def test_a_status_ping_carries_the_new_cache_end(self):
        d = fleet.derive(s(cache_expires_at=NOW - 600), NOW, hook={'why': None, 'at': NOW, 'cold_at': NOW + 3590})
        self.assertEqual((d['cache'], d['cold_at']), ('warm', NOW + 3590))


class MeasuredTtlTest(RpcBase):
    def test_measured_ttl(self):
        m = self.sb.measured_ttl
        self.assertEqual(m(NOW + 3590, NOW), 3600)
        self.assertEqual(m(NOW + 1800, NOW - 60), 3600)
        self.assertEqual(m(NOW + 290, NOW), 300)
        self.assertEqual(m(NOW + 120, NOW - 170), 300, 'a long reply still reads as 5 minutes')
        self.assertIsNone(m(NOW - 5, NOW))
        self.assertIsNone(m(None, NOW))
        self.assertIsNone(m(NOW + 300, None))

    def test_cache_end_is_real_or_measured_never_guessed(self):
        e = self.sb.cache_expiry
        self.assertEqual(e({'cache_expires_at': NOW + 30}, {'last_assistant_ts': NOW - 10}, 300), NOW + 30)
        self.assertEqual(e({}, {'last_assistant_ts': NOW - 10}, 300), NOW + 290)
        self.assertIsNone(e({}, {'last_assistant_ts': NOW - 10}, None), 'no reading ever: unknown')

    def test_list_reports_the_measured_lifetime_after_the_status_line_goes_quiet(self):
        self.transcript(SID_A, str(self.root), NOW - 60)
        last = self.sb.rpc({'verb': 'list'}, NOW, agents=[])['sessions'][0]['active']
        self.sb.write_state({'session_id': SID_A, 'prompt_cache': {'expires_at': last + 295}}, last + 1)
        r = self.sb.rpc({'verb': 'list'}, NOW, agents=[])
        row = r['sessions'][0]
        self.assertEqual((row['cache_expires_at'], row['cache_ttl']), (last + 295, 300))
        self.assertGreaterEqual(r['clock'], r['now'])
        # The status line has not run since a newer reply: the measured lifetime counts from that reply.
        self.sb.write_state({'session_id': SID_A, 'prompt_cache': {'expires_at': last + 295}}, last - 100)
        row = self.sb.rpc({'verb': 'list'}, NOW, agents=[])['sessions'][0]
        self.assertEqual((row['cache_expires_at'], row['cache_ttl']), (last + 300, 300))

    def test_no_reading_ever_is_unknown(self):
        self.transcript(SID_A, str(self.root), NOW - 60)
        row = self.sb.rpc({'verb': 'list'}, NOW, agents=[])['sessions'][0]
        self.assertEqual((row['cache_expires_at'], row['cache_ttl'], row['cache_left'], row['cold']), (None, None, 0, False))

    def test_status_ping_carries_the_cache_end(self):
        sent = []
        data = {'session_id': SID_A, 'context_window': {'current_usage': {'input_tokens': 5000}}, 'model': {'display_name': 'Opus'},
                'prompt_cache': {'expires_at': NOW + 3600}}
        self.assertTrue(self.sb.status_ping(data, {'ctx_tokens': 5000, 'model': 'Opus', 'cache_expires_at': NOW - 100}, NOW, ping=sent.append))
        self.assertEqual(sent[0]['cache_expires_at'], NOW + 3600)


class Fake:
    def __init__(self):
        self.rows, self.clock = [], NOW

    def __call__(self, target, req, timeout):
        if req['verb'] == 'list' and target == 'me@x':
            return {'ok': True, 'sessions': [dict(r) for r in self.rows], 'replaced': [], 'limits': None, 'now': self.clock - 30,
                    'clock': self.clock - 30}
        if req['verb'] == 'list':
            return {'ok': True, 'sessions': [], 'replaced': [], 'limits': None}
        return {'ok': True}


class ServerLiveTest(unittest.TestCase):
    def setUp(self):
        self.f = Fake()
        self.now = [NOW]
        st = Path(mkdtemp())
        self.app = server.App(fleet.Fleet(MACH, self.f, clock=lambda: self.now[0]), state_dir=st, web_dir=st,
                              clock=lambda: self.now[0], cfg_dir=Path(mkdtemp()), retry_waits=(0, 0))
        self.app._poll_soon = lambda *a, **k: None

    def row(self):
        snap = self.app.sessions()
        m = next(m for m in snap['machines'] if m['name'] == 'desk')
        return snap, m, m['sessions'][0]

    def test_api_fields(self):
        # The machine's clock is 30 s behind the server's.
        self.f.rows = [s(cache_expires_at=NOW - 30 + 100, ctx=200_000, cache_left=100, cold=False)]
        self.app.fleet.poll()
        snap, m, row = self.row()
        self.assertEqual(snap['now'], NOW)
        self.assertEqual((m['as_of'], m['skew']), (NOW, 30))
        self.assertEqual((row['activity'], row['cache'], row['cold_at'], row['heavy'], row['ctx']), ('quiet', 'warm', NOW + 100, True, 200_000))
        for old in ('status', 'cache_left', 'cold', 'stalled', 'held'):
            self.assertIn(old, row, 'old fields stay for one release')
        self.now[0] = NOW + 101
        self.assertEqual(self.row()[2]['cache'], 'cold', 'warm flips to cold when cold_at passes, between polls too')

    def test_hook_overlay_then_poll(self):
        self.f.rows = [s(cache_expires_at=None)]
        self.app.fleet.poll()
        self.now[0] = NOW + 5
        self.app.ping({'machine': 'desk', 'session_id': SID_A, 'why': 'prompt'})
        self.assertEqual(self.row()[2]['activity'], 'working', 'a ping re-runs derive at once')
        self.app.ping({'machine': 'desk', 'session_id': SID_A, 'why': 'status', 'ctx': 41_000, 'ts': NOW + 5 - 30,
                       'cache_expires_at': NOW + 5 - 30 + 3600})
        _, _, row = self.row()
        self.assertEqual((row['activity'], row['ctx'], row['cache'], row['cold_at']), ('working', 41_000, 'warm', NOW + 3605))
        self.now[0] = NOW + 10
        self.app.ping({'machine': 'desk', 'session_id': SID_A, 'why': 'stop'})
        self.assertEqual(self.row()[2]['activity'], 'quiet')
        # The machine answers after the pings: it knows best, and it agrees.
        self.now[0] = NOW + 12
        self.f.clock = NOW + 12
        self.f.rows = [s(cache_expires_at=NOW + 12 - 30 + 3590)]
        self.app.fleet.poll()
        _, _, row = self.row()
        self.assertEqual((row['activity'], row['cache'], row['cold_at']), ('quiet', 'warm', NOW + 3602))
        # a wait ping, then a poll that has not seen it yet is older than the ping: the ping stands
        self.now[0] = NOW + 20
        self.app.ping({'machine': 'desk', 'session_id': SID_A, 'why': 'wait', 'kind': 'permission_prompt'})
        self.assertEqual(self.row()[2]['activity'], 'needs')

    def test_held_comes_from_the_guard(self):
        self.f.rows = [s(status='waiting')]
        self.app.fleet.poll()
        self.app.guard.held_set[('desk', SID_A)] = self.app.clock()
        _, _, row = self.row()
        self.assertEqual((row['held'], row['activity']), (True, 'held'))

    def test_summary_counts_from_derive(self):
        self.f.rows = [s(status='busy', sid=SID_A), s(sid='b' * 8 + SID_A[8:], ctx=300_000, cache_expires_at=NOW - 30 - 10)]
        self.app.fleet.poll()
        sm = self.app.fleet.summary()
        self.assertEqual((sm['running'], sm['cold']), (1, 1))


class FixtureTest(unittest.TestCase):
    """The table the Mac and iPhone tests share: every payload in it is one derive() could send."""

    def test_fixture_rows_are_derive_outputs(self):
        cases = json.loads(FIXTURE.read_text())['cases']
        self.assertGreater(len(cases), 10)
        for c in cases:
            r = c['session']
            if 'activity' not in r:
                continue  # an older server's row
            self.assertIn(r['activity'], fleet.ACTIVITIES, c['name'])
            self.assertIn(r['cache'], fleet.CACHES, c['name'])
            self.assertEqual(r['heavy'], (r.get('ctx') or 0) >= fleet.GUARD_CTX, c['name'])
            self.assertEqual(r['cache'] == 'unknown', r.get('cold_at') is None, c['name'])


if __name__ == '__main__':
    unittest.main()
