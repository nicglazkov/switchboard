"""Several chats started at once: their summaries are written side by side, and no chat waits behind another's.

Six Start fresh clicks in a row once took five minutes for the last, each summary waiting for the one before it."""
import threading, time, unittest
from unittest import mock
from tests.test_jobs import JobBase
from tests.test_rpc import NOW, SID_A, SID_B

SID_C = 'cccccccc-1111-2222-3333-444444444444'
WAIT = 10  # generous: a pass never waits it out, only a failure does


class ParallelLaunchTest(JobBase):
    def setUp(self):
        super().setUp()
        for sid in (SID_A, SID_B, SID_C):
            self.transcript(sid, str(self.root / 'proj'), NOW - 60)

    def state(self, jid):
        return self.sb.rpc({'verb': 'job', 'id': jid}, NOW + 6, agents=[])

    def write(self, sid):
        self.sb.handoff_path(sid).write_text('HANDOFF ' + sid)

    def test_summaries_are_written_side_by_side(self):
        together = threading.Barrier(3, timeout=WAIT)  # passes only when all three are being written at once

        def make(tp, sid):
            together.wait()
            self.write(sid)
        jids = [self.job({'verb': 'fresh', 'sid': s}) for s in (SID_A, SID_B, SID_C)]
        self.drain(make_handoff=make)
        self.assertEqual([self.state(j)['state'] for j in jids], ['done'] * 3)
        self.assertEqual(len(self.launched), 3)

    def test_chats_are_still_launched_one_at_a_time(self):
        at_once, seen, guard = [0], [], threading.Lock()

        def run(argv, cwd, env):
            with guard:
                at_once[0] += 1
                seen.append(at_once[0])
            time.sleep(0.05)
            with guard:
                at_once[0] -= 1
            return 0, '  claude stop 3022c163    stop this session\n'
        jids = [self.job({'verb': 'fresh', 'sid': s}) for s in (SID_A, SID_B, SID_C)]
        self.drain(runner=run, make_handoff=lambda tp, sid: self.write(sid))
        self.assertEqual([self.state(j)['state'] for j in jids], ['done'] * 3)
        self.assertEqual(seen, [1, 1, 1])

    def test_a_new_chat_does_not_wait_for_another_chats_summary(self):
        started = threading.Event()
        base = self.runner()

        def run(argv, cwd, env):
            if '--resume' not in argv and not any('switchboard:' in a for a in argv):
                started.set()  # the plain new chat
            return base(argv, cwd, env)

        def make(tp, sid):
            if not started.wait(WAIT):
                raise AssertionError('the new chat waited for the summary')
            self.write(sid)
        n = iter(range(1, 1000))  # job ids in the order asked, so the Start fresh is always the first one taken
        with mock.patch.object(self.sb.os, 'urandom', lambda size: next(n).to_bytes(size, 'big')):
            fresh = self.job({'verb': 'fresh', 'sid': SID_A})
            new = self.job({'verb': 'start', 'folder': 'proj', 'prompt': 'hello'})
            self.drain(runner=run, make_handoff=make)
        self.assertEqual((self.state(fresh)['state'], self.state(new)['state']), ('done', 'done'))

    def test_no_more_summaries_at_once_than_the_limit(self):
        at_once, most, guard = [0], [0], threading.Lock()

        def make(tp, sid):
            with guard:
                at_once[0] += 1
                most[0] = max(most[0], at_once[0])
            time.sleep(0.1)
            with guard:
                at_once[0] -= 1
            self.write(sid)
        with mock.patch.object(self.sb, 'SUMMARY_PARALLEL', 2):
            jids = [self.job({'verb': 'fresh', 'sid': s}) for s in (SID_A, SID_B, SID_C)]
            self.drain(make_handoff=make)
        self.assertEqual([self.state(j)['state'] for j in jids], ['done'] * 3)
        self.assertEqual(most[0], 2)

    def test_one_failing_job_does_not_take_the_others_down(self):
        def make(tp, sid):
            if sid == SID_B:
                raise RuntimeError('boom')
            self.write(sid)
        jids = [self.job({'verb': 'fresh', 'sid': s}) for s in (SID_A, SID_B, SID_C)]
        self.drain(make_handoff=make)
        self.assertEqual([self.state(j)['state'] for j in jids], ['done', 'error', 'done'])

    def test_a_job_queued_while_others_run_is_picked_up_in_the_same_drain(self):
        late = []

        def make(tp, sid):
            if sid == SID_A and not late:
                late.append(self.job({'verb': 'fresh', 'sid': SID_B}))
            self.write(sid)
        first = self.job({'verb': 'fresh', 'sid': SID_A})
        self.drain(make_handoff=make)
        self.assertEqual((self.state(first)['state'], self.state(late[0])['state']), ('done', 'done'))


class UptimeTest(JobBase):
    def test_the_machine_says_how_long_it_has_been_up(self):
        up = self.sb.uptime_s()
        self.assertTrue(up is None or 0 < up < 20 * 365 * 86400, up)
        self.transcript(SID_A, str(self.root), NOW - 60)
        self.assertIn('up', self.sb.rpc({'verb': 'list'}, NOW, agents=[]))


if __name__ == '__main__':
    unittest.main()
