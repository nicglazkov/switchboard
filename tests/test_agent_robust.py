"""The helper keeps working when something around it is broken: a transcript it cannot read, a Claude Code that does
not answer, a holder of a claim file that died, two writers of one file."""
import json, os, subprocess, sys, tempfile, threading, time, unittest
from pathlib import Path
from tests.helpers import fresh_sb, user, assistant, write_transcript
from tests.test_rpc import RpcBase, NOW, SID_A, SID_B


class OddTranscriptTest(RpcBase):
    """One transcript in a shape the helper does not know never costs the machine its list."""

    ODD = ['[1, 2]', '"words"', '7', 'null', '{"type": "user", "message": "not an object"}',
           '{"type": "user", "origin": "human", "message": {"content": "typed by a person"}}',
           '{"type": "assistant", "timestamp": "2026-10-01T00:00:09Z", "message": {"model": {"a": 1}, "content": [{"type": "text", "text": 5}]}}',
           '{"type": "assistant", "message": {"content": {"type": "text"}}}', '{"type": "user", "message": {"content": [null, 3]}}']

    def odd(self, sid, mtime):
        p = self.chat(sid, mtime, 'something odd', title='Odd one')
        p.write_text('\n'.join(self.ODD) + '\n' + p.read_text() + '\n'.join(self.ODD) + '\n')
        os.utime(p, (mtime, mtime))
        return p

    def chat(self, sid, mtime, said, title='A title'):
        """A chat of its own: another first message, so it is nobody's copy."""
        p = Path(self.transcript(sid, str(self.root / 'proj'), mtime, title=title))
        p.write_text(p.read_text().replace('first thing I asked', said))
        os.utime(p, (mtime, mtime))
        return p

    def sids(self, **kw):
        r = self.sb.rpc({'verb': 'list'}, NOW, agents=kw.get('agents', []))
        self.assertTrue(r['ok'], r)
        return [s['sid'] for s in r['sessions']]

    def test_odd_lines_are_read_past(self):
        self.transcript(SID_A, str(self.root / 'proj'), NOW - 60)
        p = self.odd(SID_B, NOW - 30)
        self.assertEqual(self.sids(), [SID_B, SID_A])
        # every reader of a transcript, not only the list
        sb = self.sb
        sb.condense(str(p))
        sb.last_reply(str(p))
        sb.waiting_tail(p, NOW - 30)
        sb._turn_open(sb._tail_lines(p, sb.TAIL_READ)[1])
        sb.busy_report(SID_B, NOW, [], {})
        sb.thread_piece(SID_B)
        sb.generate_handoff(str(p), SID_B, runner=lambda a, b: 'ok')
        waiting = [{'sessionId': SID_B, 'kind': 'background', 'status': 'waiting', 'state': 'blocked', 'id': 'abcd1234'}]
        self.assertEqual(self.sids(agents=waiting), [SID_B, SID_A])
        for req in ({'verb': 'browse'}, {'verb': 'report', 'sid': SID_B}, {'verb': 'summary', 'sid': SID_B},
                    {'verb': 'thread_cost', 'sids': [SID_B]}, {'verb': 'ask', 'sid': SID_B}):
            r = sb.rpc(req, NOW, agents=[])
            self.assertNotEqual(r.get('error'), 'Something went wrong. Check sb.log on that machine.', req)
        sb.handle_hook({'hook_event_name': 'UserPromptSubmit', 'session_id': SID_B, 'transcript_path': str(p), 'prompt': 'hi'},
                             NOW, 1, spawn=lambda *a: None, notify=lambda e: None)
        self.assertFalse(sb.paths()['log'].exists() and 'Traceback' in sb.paths()['log'].read_text(), sb.paths()['log'].read_text()[-1500:] if sb.paths()['log'].exists() else '')

    def test_a_transcript_that_cannot_be_read_is_left_out_and_not_read_again(self):
        self.transcript(SID_A, str(self.root / 'proj'), NOW - 60)
        self.chat(SID_B, NOW - 30, 'another thing', title='Broken')
        real, asked = self.sb.session_meta, []

        def meta(f):
            asked.append(Path(f).stem)
            if Path(f).stem == SID_B:
                raise KeyError('a shape nobody thought of')
            return real(f)
        self.sb.session_meta = meta
        self.assertEqual(self.sids(), [SID_A])
        self.assertIn(SID_B, self.sb.paths()['log'].read_text())
        del asked[:]
        self.assertEqual(self.sids(), [SID_A])
        self.assertEqual(asked, [], 'left out quietly until the file changes')
        self.sb.session_meta = real
        self.chat(SID_B, NOW - 10, 'another thing', title='Mended')
        self.assertEqual(self.sids(), [SID_B, SID_A])

    def test_a_row_that_cannot_be_built_is_left_out_for_that_pass_only(self):
        self.transcript(SID_A, str(self.root / 'proj'), NOW - 60)
        self.chat(SID_B, NOW - 30, 'another thing', title='Another')
        real = self.sb.read_state
        self.sb.read_state = lambda sid: 1 / 0 if sid == SID_B else real(sid)
        self.assertEqual(self.sids(), [SID_A])
        self.sb.read_state = real
        self.assertEqual(self.sids(), [SID_B, SID_A])

    def test_a_state_file_that_is_not_an_object_reads_as_empty(self):
        (self.sb.paths()['state'] / f'{SID_A}.json').write_text('[1, 2]')
        self.assertEqual(self.sb.read_state(SID_A), {})

    def test_a_synthetic_record_does_not_change_the_model(self):
        p = write_transcript(self.tmp, [user('2026-10-01T00:00:00Z', 'hi'), assistant('2026-10-01T00:00:05Z', 'ok', model='claude-opus-5-5'),
                                        assistant('2026-10-01T00:00:09Z', 'API Error', model='<synthetic>')])
        self.assertEqual(self.sb.transcript_stats(p)['model'], 'claude-opus-5-5')

    def test_each_chat_of_a_thread_says_when_it_started(self):
        self.transcript(SID_A, str(self.root / 'proj'), NOW - 60)
        self.sids()
        self.assertIsNotNone(self.sb.thread_piece(SID_A)['started'])


class AgentsUnknownTest(RpcBase):
    """`claude agents` failing is not the same as nothing running: nothing is stopped, hidden or started on it."""

    def setUp(self):
        super().setUp()
        self.transcript(SID_A, str(self.root / 'proj'), NOW - 60)
        self.down = True

        def run(args, **kw):
            self.calls.append(args)
            if args[0] == 'agents':
                if self.down:
                    raise RuntimeError('claude: command timed out')
                return json.dumps([{'sessionId': SID_A, 'kind': 'background', 'status': 'busy', 'id': 'abcd1234', 'pid': 99}])
            return ''
        self.sb.run_claude_cmd = run

    def rpc(self, req):
        return self.sb.rpc(req, NOW)

    def test_the_list_says_so_and_changes_nothing(self):
        st = self.sb.paths()['state']
        (st / 'dismissed.json').write_text(json.dumps({SID_A: {'at': NOW - 5}}))  # dismissed while it worked
        (st / 'names_pending.json').write_text(json.dumps({SID_A: 'A name waiting for the chat to stop'}))
        before = next((self.cdir / 'projects' / 'p1').glob('*.jsonl')).read_text()
        r = self.rpc({'verb': 'list'})
        self.assertTrue(r['ok'])
        self.assertIs(r['agents_ok'], False)
        self.assertIsNone(r['mem'])
        self.assertEqual(r['sessions'], [], 'still hidden: its turn is not known to be over')
        self.assertEqual(json.loads((st / 'dismissed.json').read_text()), {SID_A: {'at': NOW - 5}})
        self.assertEqual(json.loads((st / 'names_pending.json').read_text()), {SID_A: 'A name waiting for the chat to stop'})
        self.assertEqual(next((self.cdir / 'projects' / 'p1').glob('*.jsonl')).read_text(), before, 'no title written under a live chat')
        self.down = False
        r = self.rpc({'verb': 'list'})
        self.assertIs(r['agents_ok'], True)

    def test_stop_dismiss_resume_relaunch_and_the_hard_limit_refuse(self):
        self.sb.rpc({'verb': 'list'}, NOW, agents=[])
        for req in ({'verb': 'stop', 'sid': SID_A}, {'verb': 'dismiss', 'sid': SID_A}, {'verb': 'resume', 'sid': SID_A},
                    {'verb': 'resume', 'sid': SID_A, 'relaunch': True}, {'verb': 'budget_stop'}, {'verb': 'reap_orphans', 'dry_run': True}):
            r = self.rpc(req)
            self.assertFalse(r['ok'], req)
            self.assertIn("Couldn't ask Claude Code", r['error'], req)
        self.assertFalse((self.sb.paths()['state'] / 'dismissed.json').exists())
        self.assertEqual([c for c in self.calls if c[0] in ('stop', 'rm')], [], 'nothing was stopped')
        self.assertEqual(list(self.sb.paths_q()['queue'].glob('*.json')), [], 'nothing was queued')

    def test_a_queued_resume_does_not_start_a_copy(self):
        self.sb.rpc({'verb': 'list'}, NOW, agents=[])
        self.down = False
        self.sb.trigger_launcher = lambda: None
        jid = self.rpc({'verb': 'resume', 'sid': SID_A})['job']
        self.down = True  # Claude Code stops answering between the click and the launch
        launched = []
        self.sb.launch_queue(runner=lambda *a: launched.append(a) or (0, ''), find_url=lambda bg: None, now=NOW + 5, live=lambda bg: 'ready')
        self.assertEqual(launched, [])
        r = self.sb.rpc({'verb': 'job', 'id': jid}, NOW + 6, agents=[])
        self.assertEqual(r['state'], 'error')
        self.assertIn("Couldn't ask Claude Code", r['error'])

    def test_a_rename_waits_when_it_is_not_known_whether_the_chat_runs(self):
        self.sb.rpc({'verb': 'list'}, NOW, agents=[])
        r = self.rpc({'verb': 'rename', 'sid': SID_A, 'name': 'Kitchen plan'})
        self.assertEqual((r['ok'], r['in_transcript']), (True, False))

    def test_busy_still_answers(self):
        self.assertTrue(self.rpc({'verb': 'busy', 'sid': SID_A})['ok'])


class ClaimTest(unittest.TestCase):
    def setUp(self):
        self.tmp = Path(tempfile.mkdtemp())
        self.sb = fresh_sb(self.tmp)
        self.p = self.tmp / 'x.lock'

    def dead_pid(self):
        c = subprocess.Popen([sys.executable, '-c', 'pass'])
        c.wait()
        return c.pid

    def test_one_holder_at_a_time(self):
        self.assertTrue(self.sb.lock_take(self.p, 1000, 60))
        self.assertEqual(json.loads(self.p.read_text()), {'pid': os.getpid(), 'ts': 1000})
        self.assertFalse(self.sb.lock_take(self.p, 1010, 60))
        self.sb.lock_drop(self.p)
        self.assertFalse(self.p.exists())

    def test_a_dead_holder_is_replaced_at_once(self):
        self.p.write_text(json.dumps({'pid': self.dead_pid(), 'ts': 1000}))
        self.assertTrue(self.sb.lock_take(self.p, 1001, 900))
        self.assertEqual(json.loads(self.p.read_text())['pid'], os.getpid())

    def test_a_live_holder_is_replaced_only_after_the_ttl(self):
        self.p.write_text(json.dumps({'pid': os.getppid(), 'ts': 1000}))
        self.assertFalse(self.sb.lock_take(self.p, 1900, 900))
        self.assertTrue(self.sb.lock_take(self.p, 1901, 900))

    def test_claims_from_an_older_helper_and_empty_ones(self):
        self.p.write_text('1000.0')  # the time alone
        self.assertFalse(self.sb.lock_take(self.p, 1010, 60))
        self.assertTrue(self.sb.lock_take(self.p, 1061, 60))
        self.p.write_text('')  # created this instant, nothing written yet
        self.assertFalse(self.sb.lock_take(self.p, time.time(), 60))
        self.p.write_text('{nope')
        self.assertTrue(self.sb.lock_take(self.p, 1000, 60))

    def test_a_claim_taken_over_is_not_dropped_by_its_old_holder(self):
        self.p.write_text(json.dumps({'pid': os.getppid(), 'ts': 1000}))
        self.sb.lock_drop(self.p)
        self.assertTrue(self.p.exists())

    def test_only_one_of_many_takes_a_stale_claim(self):
        self.p.write_text(json.dumps({'pid': self.dead_pid(), 'ts': 1000}))
        won, go = [], threading.Event()

        def take():
            go.wait()
            if self.sb.lock_take(self.p, 1001, 900):
                won.append(1)
        ts = [threading.Thread(target=take) for _ in range(16)]
        [t.start() for t in ts]
        go.set()
        [t.join() for t in ts]
        self.assertEqual(len(won), 1)

    def test_writers_of_one_file_all_keep_their_change(self):
        f = self.sb.paths()['state'] / 'replaced.json'

        def work(n):
            for i in range(15):
                self.sb.mark_replaced(f'{n}-{i}', 'fresh', 1000 + i)
        ts = [threading.Thread(target=work, args=(n,)) for n in range(8)]
        [t.start() for t in ts]
        [t.join() for t in ts]
        self.assertEqual(len(json.loads(f.read_text())), 120)
        self.assertFalse(f.with_name('replaced.json.lock').exists())

    def test_a_claim_nobody_frees_never_hangs_a_writer(self):
        f = self.sb.paths()['state'] / 'replaced.json'
        f.with_name('replaced.json.lock').write_text(json.dumps({'pid': os.getppid(), 'ts': time.time()}))
        t0 = time.time()
        self.sb.mark_replaced('s', 'fresh', 1000)
        self.assertLess(time.time() - t0, 3)
        self.assertIn('s', json.loads(f.read_text()))


class HandoffClaimTest(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.mkdtemp()
        self.sb = fresh_sb(self.tmp)
        self.t = write_transcript(self.tmp, [user('2026-10-10T10:00:00Z', 'Remember the plug.'), assistant('2026-10-10T10:00:05Z', 'Noted.')])
        self.lock = self.sb.paths()['handoffs'] / 'S1.generating'

    def test_a_writer_that_died_does_not_block_the_next_start_fresh(self):
        c = subprocess.Popen([sys.executable, '-c', 'pass'])
        c.wait()
        self.lock.write_text(json.dumps({'pid': c.pid, 'ts': time.time()}))
        spawned = []
        self.assertTrue(self.sb.start_handoff(self.t, 'S1', time.time(), lambda tp, sid: spawned.append(sid)))
        self.assertEqual(spawned, ['S1'])

    def test_a_claim_for_a_writer_that_never_started_is_freed(self):
        def spawn(tp, sid):
            raise OSError('no more processes')
        with self.assertRaises(OSError):
            self.sb.start_handoff(self.t, 'S1', time.time(), spawn)
        self.assertFalse(self.lock.exists())

    def test_the_writer_puts_its_name_on_the_claim_and_frees_it(self):
        self.assertTrue(self.sb.handoff_lock('S1', time.time()))
        self.assertFalse(self.sb.handoff_lock('S1', time.time()))
        seen = []
        self.sb.generate_handoff(self.t, 'S1', runner=lambda a, b: seen.append(json.loads(self.lock.read_text())['pid']) or 'ok')
        self.assertEqual(seen, [os.getpid()])
        self.assertFalse(self.lock.exists())

    def test_a_fallback_note_is_written_again_next_time(self):
        def boom(a, b):
            raise RuntimeError('rate limited')
        self.sb.generate_handoff(self.t, 'S1', runner=boom)
        self.assertTrue(self.sb.handoff_is_current('S1', self.t), 'nobody waits for a better one')
        self.assertFalse(self.sb.handoff_is_fresh('S1', self.t))
        spawned = []
        self.assertTrue(self.sb.start_handoff(self.t, 'S1', time.time(), lambda tp, sid: spawned.append(sid)))
        self.sb.generate_handoff(self.t, 'S1', runner=lambda a, b: '## Goal\nA real summary.')
        self.assertTrue(self.sb.handoff_is_fresh('S1', self.t))
        self.assertFalse(self.sb.start_handoff(self.t, 'S1', time.time(), lambda tp, sid: self.fail('written twice')))


class PromptByFileTest(unittest.TestCase):
    PROMPT = 'Write a handoff.\nKeep "quoted" words & 100% of the rules.'

    def setUp(self):
        self.sb = fresh_sb(tempfile.mkdtemp())

    def test_a_cmd_shim_gets_the_prompt_from_a_file(self):
        args, f = self.sb.prompt_args('C:\\Users\\you\\AppData\\Roaming\\npm\\claude.cmd', self.PROMPT, by_file=True)
        self.assertEqual(args, ['--system-prompt-file', str(f)])
        self.assertEqual(f.read_text(encoding='utf-8'), self.PROMPT)
        self.assertEqual(self.sb.cmd_safe(['claude.cmd', '-p', *args]), ['claude.cmd', '-p', *args])
        if os.name == 'posix':
            self.assertEqual(f.stat().st_mode & 0o777, 0o600)

    def test_only_where_it_is_needed_and_possible(self):
        as_arg = (['--system-prompt', self.PROMPT], None)
        self.assertEqual(self.sb.prompt_args('/usr/local/bin/claude', self.PROMPT, by_file=True), as_arg)
        self.assertEqual(self.sb.prompt_args('C:\\x\\claude.exe', self.PROMPT, by_file=True), as_arg)
        self.assertEqual(self.sb.prompt_args('C:\\x\\claude.cmd', self.PROMPT, by_file=False), as_arg, 'an older Claude Code')
        self.assertEqual(self.sb.prompt_args('C:\\x\\claude.cmd', 'plain words', by_file=True), (['--system-prompt', 'plain words'], None))

    def test_the_help_page_says_whether_a_file_is_read(self):
        sb = self.sb
        exe = Path(tempfile.mkdtemp()) / 'claude'
        exe.write_text('')
        sb._claude_exe = lambda: str(exe)
        new = 'Usage: claude\n  --model <model>\n  --bare  context via: --system-prompt[-file],\n  --system-prompt <prompt>\n'
        self.assertFalse(sb.prompt_file_ok(runner=lambda: 'not a help page'))
        self.assertTrue(sb.prompt_file_ok(runner=lambda: new))
        self.assertTrue(sb.prompt_file_ok(runner=lambda: self.fail('asked twice for one build')))
        os.utime(exe, (5, 5))  # another build
        self.assertFalse(sb.prompt_file_ok(runner=lambda: 'Usage: claude\n  --model <model>\n  --system-prompt <prompt>\n'))

    def test_the_file_is_gone_after_the_run(self):
        sb, seen = self.sb, {}
        sb._claude_exe = lambda: 'C:\\x\\claude.cmd'
        sb.prompt_file_ok = lambda: True

        def run(argv, stdin_text, timeout, env=None, cwd=None):
            f = Path(argv[argv.index('--system-prompt-file') + 1])
            seen.update(f=f, text=f.read_text(encoding='utf-8'))
            return type('R', (), {'stdout': json.dumps({'result': 'done'}), 'stderr': '', 'returncode': 0})()
        sb.run_tree = run
        sb.run_claude_opus('the chat', self.PROMPT)
        self.assertEqual(seen['text'], self.PROMPT)
        self.assertFalse(seen['f'].exists())



class TidyTest(RpcBase):
    """The helper's own per-chat files go once the chat is gone or past the history window; never before."""
    OLD, NEW, GONE = ('aaaaaaaa-0000-4000-8000-00000000000%d' % i for i in (1, 2, 3))

    def setUp(self):
        super().setUp()
        import os
        self.os = os
        self.proj = self.cdir / 'projects' / 'p'
        self.proj.mkdir(parents=True, exist_ok=True)
        self.long_ago = NOW - 60 * 86400
        for sid, at in ((self.OLD, self.long_ago), (self.NEW, NOW - 3600)):
            t = self.proj / f'{sid}.jsonl'
            t.write_text('{}\n')
            os.utime(t, (at, at))
        self.state, self.handoffs = self.sb.paths()['state'], self.sb.paths()['handoffs']
        self.queue = self.sb.paths_q()['queue']

    def put(self, d, name, at):
        f = d / name
        f.write_text('{}')
        self.os.utime(f, (at, at))
        return f

    def test_old_files_of_old_and_gone_chats_go(self):
        gone = [self.put(self.state, f'{sid}{ext}', self.long_ago)
                for sid in (self.OLD, self.GONE) for ext in ('.json', '.ping', '.tools', '.held.json')
                if (sid, ext) != (self.OLD, '.tools')]  # its transcript is still there: what it was started with stays
        kept = self.put(self.state, f'{self.OLD}.tools', self.long_ago)
        gone += [self.put(self.handoffs, f'{self.GONE}.md', self.long_ago),
                 self.put(self.queue, 'j1.result.json', NOW - 2 * 86400)]
        self.assertEqual(self.sb.prune_state(NOW), len(gone))
        self.assertEqual([f.name for f in gone if f.exists()], [])
        self.assertTrue(kept.exists())

    def test_anything_recent_or_of_a_chat_in_the_window_stays(self):
        keep = [self.put(self.state, f'{self.NEW}.json', self.long_ago),      # the chat was written to an hour ago
                self.put(self.handoffs, f'{self.NEW}.md', self.long_ago),
                self.put(self.state, f'{self.GONE}.json', NOW - 86400),  # the file itself is recent
                self.put(self.state, f'{self.OLD}.ping', NOW - 44 * 86400),
                self.put(self.state, 'names.json', self.long_ago),            # not a chat's file
                self.put(self.state, 'replaced.json', self.long_ago),
                self.put(self.queue, 'j2.result.json', NOW - 3600),
                self.put(self.queue, 'j3.json', self.long_ago)]               # a job itself is the launcher's to drop
        self.assertEqual(self.sb.prune_state(NOW), 0)
        self.assertEqual([f.name for f in keep if not f.exists()], [])

    def test_nothing_goes_when_no_transcript_can_be_seen(self):
        for t in self.proj.glob('*.jsonl'):
            t.unlink()
        f = self.put(self.state, f'{self.GONE}.json', self.long_ago)
        self.assertEqual(self.sb.prune_state(NOW), 0)
        self.assertTrue(f.exists())

    def test_it_runs_once_an_hour_from_the_list(self):
        f = self.put(self.state, f'{self.GONE}.ping', self.long_ago)
        self.sb.rpc({'verb': 'list'}, NOW, agents=[])
        self.assertFalse(f.exists())
        f = self.put(self.state, f'{self.GONE}.ping', self.long_ago)
        self.assertIsNone(self.sb.prune_state(NOW + 60))
        self.assertTrue(f.exists())
        self.assertEqual(self.sb.prune_state(NOW + 3700), 1)



class BigListTest(RpcBase):
    """A list poll never does whole-transcript work for every row at once."""

    def chat(self, sid, mtime, said):
        p = Path(self.transcript(sid, str(self.root / 'proj'), mtime, title=said))
        p.write_text(p.read_text().replace('first thing I asked', said))
        os.utime(p, (mtime, mtime))
        return p

    def test_a_poll_counts_transcripts_for_a_few_seconds_and_the_next_goes_on(self):
        sids = ['bbbbbbbb-0000-4000-8000-00000000000%d' % i for i in range(4)]
        for i, sid in enumerate(sids):
            self.chat(sid, NOW - 60 - i, f'job number {i}')
        real, read = self.sb.condense, []
        self.sb.condense = lambda p: read.append(str(p)) or real(p)
        self.sb.CONDENSE_POLL_S = 0  # the time is used up by the first row
        rows = self.sb.rpc({'verb': 'list'}, NOW, agents=[])['sessions']
        self.assertEqual(len(rows), 4)
        self.assertEqual(len(read), 1, 'one row is always counted, so every poll gets somewhere')
        self.assertEqual([r['condensed'] is None for r in rows].count(True), 3)
        self.assertTrue(all(r['cost'] for r in rows), 'an uncounted row still shows a cost, estimated')
        self.sb.CONDENSE_POLL_S = 3.0
        rows = self.sb.rpc({'verb': 'list'}, NOW, agents=[])['sessions']
        self.assertEqual(len(read), 4, 'the next poll goes on where this one stopped')
        self.assertTrue(all(isinstance(r['condensed'], int) for r in rows))

    def test_a_name_is_found_in_a_file_read_in_pieces(self):
        f = self.tmp / 'big.bin'
        f.write_bytes(b'x' * 1000 + b'needle-123' + b'y' * 1000)
        for chunk in (1, 7, 1005, 1006, 4096):  # the name falls across the edge of a piece, or not
            self.assertTrue(self.sb._file_has(f, b'needle-123', chunk), chunk)
            self.assertFalse(self.sb._file_has(f, b'needle-124', chunk), chunk)
        self.assertTrue(self.sb._file_has(f, b'y', 3))
        self.assertFalse(self.sb._file_has(f, b'z', 3))



class StatusStateTest(unittest.TestCase):
    def setUp(self):
        self.tmp = Path(tempfile.mkdtemp()).resolve()
        self.sb = fresh_sb(self.tmp)

    def test_a_model_given_as_a_plain_name_is_still_recorded(self):
        for model, want in (('Opus', 'Opus'), ({'display_name': 'Sonnet'}, 'Sonnet'), (None, None), (7, None)):
            self.sb.write_state({'session_id': 'S1', 'model': model}, 100.0)
            self.assertEqual(self.sb.read_state('S1').get('model'), want)
            self.assertEqual(self.sb.read_state('S1')['updated'], 100.0)

    def test_two_renders_of_one_chat_never_share_a_temp_file(self):
        errs = []

        def render(i):
            try:
                for n in range(40):
                    self.sb.write_state({'session_id': 'S1', 'model': {'display_name': f'm{i}'}}, float(n))
            except Exception as e:
                errs.append(e)
        ts = [threading.Thread(target=render, args=(i,)) for i in range(6)]
        [t.start() for t in ts]
        [t.join() for t in ts]
        self.assertEqual(errs, [])
        self.assertEqual(self.sb.read_state('S1')['updated'], 39.0)
        self.assertEqual([f.name for f in self.sb.paths()['state'].glob('*.tmp')], [])


if __name__ == '__main__':
    unittest.main()
