"""The helper's `own` verb (keys phase 4): this computer's chats with their live state, for a device that reaches
this computer by itself while the home computer is away. It gives what `list` plus the server's derive() gives, and
it writes nothing."""
import hashlib, json, os, sys, unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / 'server'))
import fleet  # noqa: E402
from tests.helpers import user, assistant, entry, write_transcript  # noqa: E402
from tests.test_rpc import RpcBase, NOW, SID_A, SID_B, _iso  # noqa: E402

SID_C = 'cccccccc-1111-2222-3333-444444444444'
LIVE = ('activity', 'cache', 'cold_at', 'heavy')


def snapshot(*roots):
    """Every file under the folders: its bytes and when it was last written. The log is left out: `own` may log."""
    out = {}
    for root in roots:
        for d, _, files in os.walk(root):
            for n in files:
                p = Path(d) / n
                if n in ('sb.log', 'sb.log.1'):
                    continue
                st = p.stat()
                out[str(p)] = (hashlib.sha256(p.read_bytes()).hexdigest(), st.st_mtime_ns, st.st_size)
    return out


class OwnBase(RpcBase):
    def transcript(self, sid, cwd, mtime, title='A title', ctx=200_000, url_id='cse_XYZ123'):
        """RpcBase's, with Claude's last reply at `mtime` instead of months before it."""
        rows = [entry('bridge-session', _iso(mtime - 30), bridgeSessionId=url_id, sessionId=sid),
                dict(user(_iso(mtime - 20), 'first thing I asked'), cwd=cwd),
                dict(assistant(_iso(mtime), 'ok', usage={'input_tokens': ctx}), cwd=cwd),
                {'type': 'ai-title', 'aiTitle': title, 'sessionId': sid}]
        p = write_transcript(self.cdir / 'projects' / 'p1', rows, name=f'{sid}.jsonl')
        os.utime(p, (mtime, mtime))
        return p

    def setUp(self):
        super().setUp()
        self.transcript(SID_A, str(self.root / 'proj'), NOW - 600, title='Older')
        self.transcript(SID_B, str(self.root), NOW - 60, title='Newer', ctx=40_000)
        self.transcript(SID_C, str(self.root), NOW - 30, title='Third', ctx=160_000)
        self.agents = [
            {'sessionId': SID_A, 'id': 'aaaaaaaa', 'kind': 'background', 'status': 'idle', 'state': 'done', 'pid': 1},
            {'sessionId': SID_B, 'id': 'bbbbbbbb', 'kind': 'background', 'status': 'busy', 'state': 'running', 'pid': 2},
            {'sessionId': SID_C, 'id': 'cccccccc', 'kind': 'interactive', 'status': 'waiting', 'pid': 3},
        ]

    def own(self, now=NOW, **req):
        r = self.sb.rpc(dict({'verb': 'own'}, **req), now, agents=self.agents)
        self.assertTrue(r.get('ok'), r)
        return r

    def rows(self, now=NOW, **req):
        return {s['sid']: s for s in self.own(now, **req)['machine']['sessions']}


class OwnShapeTest(OwnBase):
    def test_the_answer_is_one_machine_row(self):
        r = self.own()
        self.assertEqual(set(r), {'ok', 'now', 'machine'})
        m = r['machine']
        self.assertEqual(r['now'], NOW)
        self.assertEqual((m['online'], m['as_of'], m['agents_ok']), (True, NOW, True))
        self.assertEqual(m['name'], self.sb.machine_name())
        self.assertEqual(m['os'], self.sb.os_name())
        self.assertEqual(m['replaced'], [])
        self.assertEqual([s['title'] for s in m['sessions']], ['Third', 'Newer', 'Older'])
        json.dumps(r)  # it goes out as JSON

    def test_each_row_is_what_list_and_the_servers_rule_give(self):
        listed = {s['sid']: s for s in self.sb.rpc({'verb': 'list'}, NOW, agents=self.agents)['sessions']}
        rows = self.rows()
        self.assertEqual(set(rows), set(listed))
        for sid, s in listed.items():
            row = rows[sid]
            for k, v in s.items():
                self.assertEqual(row[k], v, f'{sid} {k}')
            want = fleet.derive(dict(s), NOW, 0.0, False, None, NOW)
            self.assertEqual({k: row[k] for k in LIVE}, want, sid)
            aged = fleet.mark_age(dict(s, **want), NOW)
            self.assertEqual((row['stale'], row['stalled'], row['held']), (aged['stale'], aged['stalled'], False))
        self.assertEqual(rows[SID_A]['activity'], 'quiet')
        self.assertEqual(rows[SID_B]['activity'], 'working')
        self.assertEqual(rows[SID_C]['activity'], 'needs')
        self.assertTrue(rows[SID_C]['heavy'])
        self.assertFalse(rows[SID_B]['heavy'])

    def test_claude_code_not_answering_is_said(self):
        self.sb.claude_agents = lambda: self.sb._NoAgents()
        r = self.sb.rpc({'verb': 'own'}, NOW)
        self.assertTrue(r['ok'])
        self.assertIs(r['machine']['agents_ok'], False)
        self.assertEqual(len(r['machine']['sessions']), 3, [s['title'] for s in r['machine']['sessions']])


class OwnPingTest(OwnBase):
    def test_a_ping_changes_the_state_as_on_home(self):
        ping = lambda **kw: {SID_A: dict({'at': NOW - 1}, **kw)}
        self.assertEqual(self.rows(pings=ping(why='prompt', status='busy'))[SID_A]['activity'], 'working')
        self.assertEqual(self.rows(pings=ping(why='wait', status='waiting'))[SID_A]['activity'], 'needs')
        self.assertEqual(self.rows(pings={SID_B: {'why': 'stop', 'status': 'idle', 'at': NOW - 1}})[SID_B]['activity'], 'quiet')
        self.assertEqual(self.rows(pings={SID_C: {'why': 'prompt', 'status': 'busy', 'at': NOW - 1}})[SID_C]['activity'], 'working')
        # the same answers the server's rule gives for the same ping
        s = next(x for x in self.sb.rpc({'verb': 'list'}, NOW, agents=self.agents)['sessions'] if x['sid'] == SID_A)
        hook = {'why': 'prompt', 'at': NOW - 1, 'cold_at': None}
        want = fleet.derive(dict(s, status='busy'), NOW, 0.0, False, hook, NOW)
        got = self.rows(pings=ping(why='prompt', status='busy'))[SID_A]
        self.assertEqual({k: got[k] for k in LIVE}, want)
        self.assertEqual(got['status'], 'busy')

    def test_the_status_lines_numbers_are_taken(self):
        row = self.rows(pings={SID_B: {'ctx': 170_000, 'cold_at': NOW + 240.5, 'at': NOW - 2}})[SID_B]
        self.assertEqual((row['ctx'], row['heavy'], row['cache'], row['cold_at']), (170_000, True, 'warm', NOW + 240.5))
        self.assertEqual(row['activity'], 'working', 'numbers alone do not change what it is doing')

    def test_an_old_ping_and_a_ping_that_is_not_one_are_ignored(self):
        plain = self.rows()
        for pings in ({SID_A: {'why': 'prompt', 'status': 'busy', 'at': NOW - 60}}, {SID_A: {'why': 'prompt', 'status': 'busy'}},
                      {SID_A: 'prompt'}, {SID_A: {'why': 'rm -rf', 'status': 'gone', 'ctx': -5, 'cold_at': 'x', 'at': NOW - 1}},
                      {SID_A: {'why': 'prompt', 'at': True}}, {'no-such-chat': {'why': 'prompt', 'at': NOW - 1}}, [], 'x', None):
            self.assertEqual(self.rows(pings=pings), plain, pings)

    def test_a_late_ping_does_not_bring_a_stopped_chat_back(self):
        self.agents[0]['status'] = 'stopped'
        row = self.rows(pings={SID_A: {'why': 'prompt', 'status': 'busy', 'at': NOW - 1}})[SID_A]
        self.assertEqual((row['status'], row['activity']), ('stopped', 'quiet'))

    def test_a_held_message_shows_as_held(self):
        self.sb.held_set(SID_B, 'send this when it is warm', NOW - 5)
        rows = self.rows()
        self.assertEqual((rows[SID_B]['held'], rows[SID_B]['activity']), (True, 'held'))
        self.assertEqual((rows[SID_A]['held'], rows[SID_A]['activity']), (False, 'quiet'))
        self.sb.held_clear(SID_B)
        self.assertEqual(self.rows()[SID_B]['activity'], 'working')


class OwnReadOnlyTest(OwnBase):
    def watched(self):
        return snapshot(self.tmp)

    def test_nothing_is_written_on_a_computer_that_never_listed(self):
        before = self.watched()
        self.own(pings={SID_A: {'why': 'prompt', 'status': 'busy', 'at': NOW - 1}})
        self.assertEqual(self.watched(), before)
        state = Path(os.environ['SB_HOME']) / 'state'
        self.assertEqual([n for n in os.listdir(state)], [], 'no index, no cache, no marks')

    def test_every_file_is_the_same_afterwards(self):
        # a computer in use: identity, budget, the index and the caches from a poll, a held message, a dismiss
        self.sb.rpc({'verb': 'list', 'machine': 'laptop', 'budget': {'day': 5}, 'options': {}}, NOW - 100, agents=self.agents)
        self.sb.held_set(SID_B, 'later', NOW - 50)
        self.sb.rpc({'verb': 'dismiss', 'sid': SID_A}, NOW - 40, agents=self.agents)
        # and since then a transcript grew, so the index is out of date, and one chat was never indexed
        p = self.transcript(SID_B, str(self.root), NOW - 20, title='Newer, grown', ctx=90_000)
        os.utime(p, (NOW - 20, NOW - 20))
        self.transcript('dddddddd-1111-2222-3333-444444444444', str(self.root / 'proj'), NOW - 10, title='Fourth')
        before = self.watched()
        self.assertTrue(any(k.endswith('index.json') for k in before))
        for now in (NOW, NOW + 1, NOW + 400):
            rows = self.rows(now)
        self.assertEqual(self.watched(), before)
        self.assertEqual(rows[SID_B]['title'], 'Newer, grown', 'it read the new transcript all the same')
        self.assertIn('dddddddd-1111-2222-3333-444444444444', rows)
        self.assertNotIn(SID_A, rows, 'a dismissed chat stays hidden')
        self.assertFalse(self.sb._READ_ONLY, 'and the helper writes again afterwards')
        # the next poll from home still does its writing
        self.sb.rpc({'verb': 'list'}, NOW + 500, agents=self.agents)
        self.assertNotEqual(self.watched(), before)

    def test_nothing_in_the_request_is_taken_for_identity_budget_usage_or_spend(self):
        boom = lambda *a, **k: (_ for _ in ()).throw(AssertionError('own must not do this'))
        for name in ('remember_identity', 'budget_put', 'spawn_account_refresh', 'spend_report', 'prune_state', 'setup_issues',
                     'mark_replaced'):
            setattr(self.sb, name, boom)
        before = self.watched()
        r = self.own(machine='evil', label='Evil', budget={'day': 1}, usage={'max_age': 0}, spend={'since': 0},
                     options={'x': True}, root='/')
        self.assertEqual(len(r['machine']['sessions']), 3)
        self.assertEqual(self.watched(), before)

    def test_a_failure_inside_still_ends_read_only(self):
        self.sb.list_sessions = lambda *a, **k: (_ for _ in ()).throw(RuntimeError('boom'))
        r = self.sb.rpc({'verb': 'own'}, NOW, agents=self.agents)
        self.assertFalse(r.get('ok'))
        self.assertFalse(self.sb._READ_ONLY)


if __name__ == '__main__':
    unittest.main()
