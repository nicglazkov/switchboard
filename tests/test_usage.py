"""GET /api/usage (this week by day, past weeks) and what a blocked chat needs from you (spec 2026-10-13, 1 and 4)."""
import json, sys, unittest
from pathlib import Path
from tempfile import mkdtemp
from tests.test_rpc import RpcBase, NOW as RPC_NOW, SID_A
from tests.test_week_history import LA, MIN, HOUR, DAY, at, RESET, START, NOW, MACH, Machines
sys.path.insert(0, str(Path(__file__).resolve().parents[1] / 'server'))
import fleet, server  # noqa: E402


class DayUsedTest(unittest.TestCase):
    def test_each_local_day_is_its_last_sample_minus_its_first(self):
        samples = [(START + HOUR, 2), (at(2026, 10, 8, 23, 50), 6),              # Thu, from 0 at the 9 AM reset
                   (at(2026, 10, 9, 0, 5), 6.5), (at(2026, 10, 9, 20), 16.5),     # Fri
                   (at(2026, 10, 10, 10), 17), (at(2026, 10, 10, 11), 15),       # Sat: went down, never negative
                   (at(2026, 10, 11, 9), 20)]                                    # Sun: one sample, cannot tell
        days = fleet.day_used(samples, START, NOW, LA)
        self.assertEqual(days, [{'day': '2026-10-08', 'used': 6.0}, {'day': '2026-10-09', 'used': 10.0},
                                {'day': '2026-10-10', 'used': 0.0}, {'day': '2026-10-11', 'used': None}])

    def test_days_with_no_samples_are_unknown_and_the_first_day_counts_from_zero(self):
        days = fleet.day_used([(at(2026, 10, 10, 12), 9), (at(2026, 10, 10, 13), 12)], START, at(2026, 10, 10, 14), LA)
        self.assertEqual([d['used'] for d in days], [0.0, None, 3.0])

    def test_a_reset_starts_the_days_again(self):
        h = fleet.WeekHistory(Path(mkdtemp()) / 'week_history.json', tz=LA)
        h.record(40, RESET, RESET - 2 * HOUR)
        h.record(45, RESET, RESET - HOUR)                  # Thu Oct 15, 8 AM: the last hour of the week
        nxt = RESET + fleet.WEEK
        h.record(1, nxt, RESET + HOUR)                     # 10 AM, a new week
        h.record(3, nxt, RESET + 3 * HOUR)
        days, weeks = h.usage(4, nxt, RESET + 4 * HOUR)
        self.assertEqual(days, [{'day': '2026-10-15', 'used': 4.0}])      # from 0 at the reset, not from 45
        self.assertEqual(weeks[0]['reset'], RESET)
        self.assertEqual(weeks[0]['pct'], 45.0)
        self.assertEqual(weeks[0]['days'][-1], {'day': '2026-10-15', 'used': 5.0})


class WeekArchiveTest(unittest.TestCase):
    def setUp(self):
        self.dir = Path(mkdtemp())
        self.h = fleet.WeekHistory(self.dir / 'week_history.json', tz=LA)

    def week(self, k, pct):
        reset = RESET + k * fleet.WEEK
        self.h.record(pct - 5, reset, reset - DAY)
        self.h.record(pct, reset, reset - HOUR)

    def test_keeps_the_newest_eight_weeks_newest_first(self):
        for k in range(11):
            self.week(k, 10 + k)
        self.week(11, 90)                                   # starts week 11: weeks 0..10 archived, 3..10 kept
        weeks = json.loads((self.dir / 'week_archive.json').read_text())['weeks']
        self.assertEqual(len(weeks), 8)
        self.assertEqual([w['pct'] for w in weeks], [20.0, 19.0, 18.0, 17.0, 16.0, 15.0, 14.0, 13.0])
        self.assertEqual(weeks[0]['reset'], RESET + 10 * fleet.WEEK)
        again = fleet.WeekHistory(self.dir / 'week_history.json', tz=LA)   # a restart keeps them
        self.assertEqual(again.weeks, weeks)

    def test_nothing_archived_without_samples_or_within_the_same_week(self):
        self.h.record(30, RESET, NOW)
        self.h.record(31, RESET + 60, NOW + 20 * MIN)       # the reset wobbled: same week
        self.assertFalse((self.dir / 'week_archive.json').exists())
        self.assertEqual(self.h.usage(31, RESET, NOW + 30 * MIN)[1], [])

    def test_a_broken_archive_starts_empty(self):
        (self.dir / 'week_archive.json').write_text('{"weeks": "x"}')
        self.assertEqual(fleet.WeekHistory(self.dir / 'week_history.json', tz=LA).weeks, [])


class UsageEndpointTest(unittest.TestCase):
    def test_shape(self):
        m, state = Machines(), Path(mkdtemp())
        clock = [NOW]
        f = fleet.Fleet(MACH, m, clock=lambda: clock[0])
        app = server.App(f, state_dir=state, web_dir=state)
        f.history.tz = LA
        f.poll()
        clock[0] = NOW + 20 * MIN
        m.week = 33
        f.poll()
        u = app.get('/api/usage')
        self.assertEqual(set(u), {'ok', 'now', 'tz', 'five', 'week', 'days', 'weeks'})
        self.assertTrue(u['ok'])
        self.assertEqual(u['five'], {'pct': 12, 'reset': None})
        self.assertEqual((u['week']['pct'], u['week']['reset']), (33, RESET))
        self.assertEqual(u['week']['pace'], fleet._projected(33, RESET, NOW + 20 * MIN))
        self.assertEqual(u['days'][0], {'day': '2026-10-08', 'used': 0.0})
        self.assertEqual(u['days'][-1], {'day': '2026-10-11', 'used': 2.0})
        self.assertEqual(len(u['days']), 4)
        self.assertEqual(u['weeks'], [])
        json.dumps(u)

    def test_before_any_numbers(self):
        f = fleet.Fleet(MACH, lambda *a: (_ for _ in ()).throw(OSError('down')), clock=lambda: NOW)
        state = Path(mkdtemp())
        u = server.App(f, state_dir=state, web_dir=state).get('/api/usage')
        self.assertEqual((u['five']['pct'], u['week']['pct'], u['week']['pace'], u['days'], u['weeks']),
                         (None, None, None, [], []))


class NeedsTest(RpcBase):
    SHORT = 'aaaaaaaa'

    def job(self, **st):
        d = self.cdir / 'jobs' / self.SHORT
        d.mkdir(parents=True, exist_ok=True)
        (d / 'state.json').write_text(json.dumps(st))

    def row(self, **a):
        self.transcript(SID_A, str(self.root / 'proj'), RPC_NOW - 60)
        ag = dict({'id': self.SHORT, 'sessionId': SID_A, 'kind': 'background', 'pid': 4242, 'cwd': str(self.root / 'proj')}, **a)
        r = self.sb.rpc({'verb': 'list'}, RPC_NOW, agents=[ag])
        return next(s for s in r['sessions'] if s['sid'] == SID_A)

    def test_a_blocked_chat_says_what_it_needs_trimmed(self):
        self.job(state='blocked', needs='  Confirm option 1 ' + 'x' * 400, suggestedReply='Go with option 1' + 'y' * 300)
        s = self.row(status='idle', state='blocked')
        self.assertTrue(s['needs'].startswith('Confirm option 1 x'))
        self.assertEqual(len(s['needs']), 300)
        self.assertEqual(len(s['suggested_reply']), 200)

    def test_waiting_also_reads_it(self):
        self.job(state='blocked', needs='Pick one', suggestedReply='One')
        s = self.row(status='waiting', state='running')
        self.assertEqual((s['needs'], s['suggested_reply']), ('Pick one', 'One'))

    def test_not_blocked_or_waiting_has_neither(self):
        self.job(state='blocked', needs='Pick one', suggestedReply='One')
        s = self.row(status='busy', state='running')
        self.assertNotIn('needs', s)
        self.assertNotIn('suggested_reply', s)

    def test_missing_parts_are_left_out(self):
        self.job(state='blocked', needs='Pick one')
        s = self.row(status='idle', state='blocked')
        self.assertEqual(s['needs'], 'Pick one')
        self.assertNotIn('suggested_reply', s)
        self.assertNotIn('needs', self.row(status='idle', state='blocked', id='../../etc'))

    def test_the_server_passes_them_through(self):
        row = {'sid': SID_A, 'title': 't', 'mtime': NOW - 60, 'active': NOW - 60, 'kind': 'background', 'status': 'idle',
               'state': 'blocked', 'needs': 'Pick one', 'suggested_reply': 'One'}
        f = fleet.Fleet(MACH[:1], lambda *a: {'ok': True, 'sessions': [row]}, clock=lambda: NOW)
        state = Path(mkdtemp())
        app = server.App(f, state_dir=state, web_dir=state)
        f.poll()
        s = app.get('/api/sessions')['machines'][0]['sessions'][0]
        self.assertEqual((s['needs'], s['suggested_reply']), ('Pick one', 'One'))


if __name__ == '__main__':
    unittest.main()
