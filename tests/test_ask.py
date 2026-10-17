"""The `ask` card on a waiting chat's row (spec 2026-10-13 needs-shaped replies): AskUserQuestion mapping, the Haiku
card's validation and limits, the cache by transcript mtime, and `list` never waiting on Haiku."""
import json, os, sys, time, unittest
from pathlib import Path
from tempfile import mkdtemp
from tests.helpers import entry, user, assistant, write_transcript, sb_options
from tests.test_rpc import RpcBase, NOW as RPC_NOW, SID_A
from tests.test_week_history import NOW, MACH
sys.path.insert(0, str(Path(__file__).resolve().parents[1] / 'server'))
import fleet, server  # noqa: E402

ASK_INPUT = {'questions': [
    {'question': 'How do you picture building a playlist?', 'header': 'Workflow', 'multiSelect': False,
     'options': [{'label': 'Filter, then hand-pick (Recommended)', 'description': 'd1'},
                 {'label': 'Pure rules, auto-filled', 'description': 'd2'}]},
    {'question': 'Which sources?', 'header': 'Sources', 'multiSelect': True,
     'options': [{'label': 'Likes'}, {'label': 'Playlists'}, {'label': 'Search'}]}]}


def ask_use(tid='toolu_1', inp=ASK_INPUT):
    return entry('assistant', '2026-10-01T00:01:00Z', message={'role': 'assistant', 'stop_reason': 'tool_use', 'content': [
        {'type': 'text', 'text': 'Two questions first.'},
        {'type': 'tool_use', 'id': tid, 'name': 'AskUserQuestion', 'input': inp}]})


def result(tid='toolu_1', text='User has answered your questions'):
    return entry('user', '2026-10-01T00:02:00Z', message={'role': 'user', 'content': [
        {'type': 'tool_result', 'tool_use_id': tid, 'content': text}]})


def lines(rows):
    return [json.dumps(r).encode() for r in rows]


class QuestionToolTest(RpcBase):
    def test_maps_questions_options_and_multiselect_exactly(self):
        a = self.sb.question_tool_ask(lines([user('2026-10-01T00:00:00Z', 'build it'), ask_use()]), 12.5, 'needs line')
        self.assertEqual(a['kind'], 'questions')
        self.assertEqual((a['source'], a['for']), ('question_tool', 12.5))
        q1, q2 = a['questions']
        self.assertEqual(q1, {'q': 'How do you picture building a playlist?', 'header': 'Workflow',
                              'options': ['Filter, then hand-pick', 'Pure rules, auto-filled'], 'multi': False,
                              'about_you': False, 'recommended': 'Filter, then hand-pick'})
        self.assertEqual((q2['options'], q2['multi'], q2['recommended']), (['Likes', 'Playlists', 'Search'], True, None))
        self.assertTrue(a['summary'].startswith('Answer 2 questions: How do you picture'))

    def test_answered_or_superseded_is_not_pending(self):
        sb = self.sb
        self.assertIsNone(sb.question_tool_ask(lines([ask_use(), result()])))
        self.assertIsNone(sb.question_tool_ask(lines([ask_use(), result(), assistant('2026-10-01T00:03:00Z', 'Thanks')])))
        self.assertIsNone(sb.question_tool_ask(lines([ask_use(), user('2026-10-01T00:03:00Z', 'never mind')])))
        self.assertIsNone(sb.question_tool_ask(lines([assistant('2026-10-01T00:03:00Z', 'Which one?')])))

    def test_an_older_answered_call_does_not_hide_a_new_one(self):
        rows = [ask_use('toolu_1'), result('toolu_1'), ask_use('toolu_2')]
        self.assertEqual(self.sb.question_tool_ask(lines(rows))['kind'], 'questions')

    def test_slash_command_noise_after_it_is_skipped(self):
        noise = entry('user', '2026-10-01T00:03:00Z', message={'role': 'user', 'content': '<command-name>/model</command-name>'})
        self.assertEqual(self.sb.question_tool_ask(lines([ask_use(), noise]))['kind'], 'questions')

    def test_bad_input_is_not_a_card(self):
        self.assertIsNone(self.sb.question_tool_ask(lines([ask_use(inp={'questions': []})])))
        self.assertIsNone(self.sb.question_tool_ask(lines([ask_use(inp={'questions': [{'header': 'x'}]})])))


class ValidateTest(RpcBase):
    def v(self, d, needs='Pick one', said='I recommend option 1'):
        return self.sb.validate_ask(d, 'haiku', 7.0, needs, said)

    def test_choose_is_clipped_and_keeps_one_recommendation(self):
        opts = [{'label': f'Option {i} ' + 'x' * 200, 'reply': 'r' * 900, 'recommended': True} for i in range(6)]
        a = self.v({'kind': 'choose', 'summary': 's' * 500, 'options': opts, 'assumptions': ['a'] * 9, 'extra': 1})
        self.assertEqual(len(a['summary']), 140)
        self.assertEqual(len(a['options']), 4)
        self.assertTrue(all(len(o['label']) <= 80 and len(o['reply']) <= 300 for o in a['options']))
        self.assertEqual([o['recommended'] for o in a['options']], [True, False, False, False])
        self.assertEqual(len(a['assumptions']), 6)
        self.assertNotIn('extra', a)
        self.assertEqual((a['source'], a['for']), ('haiku', 7.0))

    def test_a_recommendation_claude_never_made_is_dropped(self):
        d = {'kind': 'choose', 'summary': 'Pick', 'options': [{'label': 'A', 'recommended': True}, {'label': 'B'}]}
        a = self.v(d, said='Option A or option B?')
        self.assertEqual([o['recommended'] for o in a['options']], [False, False])
        self.assertEqual(a['options'][0]['reply'], 'A')  # reply defaults to the label

    def test_questions_about_you_never_preselect(self):
        a = self.v({'kind': 'questions', 'summary': 'Answer', 'questions': [
            {'q': 'Are you a member?', 'options': ['Yes', 'No', 'Yes'], 'about_you': True, 'recommended': 'Yes'},
            {'q': 'Layout?', 'options': ['A', 'B'], 'recommended': 'C'},
            {'q': ''}] + [{'q': f'Q{i}', 'options': [str(j) for j in range(9)]} for i in range(6)]})
        q = a['questions']
        self.assertEqual(len(q), 5)
        self.assertEqual((q[0]['options'], q[0]['recommended'], q[0]['about_you']), (['Yes', 'No'], None, True))
        self.assertIsNone(q[1]['recommended'])  # not one of its options
        self.assertEqual(len(q[2]['options']), 5)

    def test_task_and_fix(self):
        a = self.v({'kind': 'task', 'summary': 'Do it', 'steps': ['one', '', 's' * 400] + ['n'] * 10})
        self.assertEqual((len(a['steps']), len(a['steps'][1]), a['done_reply']), (8, 160, 'Done.'))
        f = self.v({'kind': 'fix', 'summary': 'Chrome', 'fix': {'action': 'relaunch_chrome', 'alt_reply': 'Open now.'}})
        self.assertEqual(f['fix'], {'action': 'relaunch_chrome', 'label': 'Restart this chat with Chrome',
                                    'alt_reply': 'Open now.'})

    def test_invalid_becomes_open(self):
        sb = self.sb
        for bad in (None, 'x', [], {'kind': 'other', 'summary': 'x'}):
            a = self.v(bad)
            self.assertEqual((a['kind'], a['summary'], a['source']), ('open', 'Pick one', 'claude_code'))
        for d in ({'kind': 'fix', 'summary': 'Reboot', 'fix': {'action': 'reboot_mac'}},
                  {'kind': 'choose', 'summary': 'Reboot', 'options': [{'label': 'only one'}]},
                  {'kind': 'questions', 'summary': 'Reboot', 'questions': 'no'},
                  {'kind': 'task', 'summary': 'Reboot', 'steps': []}):
            a = self.v(d)
            self.assertEqual((a['kind'], a['summary'], a['source']), ('open', 'Reboot', 'haiku'))
            self.assertEqual(set(a), {'kind', 'summary', 'source', 'for'})
        self.assertEqual(sb.validate_ask({'kind': 'open'}, 'haiku', 1, None)['summary'], sb.ASK_DEFAULT)

    def test_strings_are_one_line_without_em_dashes(self):
        a = self.v({'kind': 'open', 'summary': '  Pick \u2014 now\n\nplease  '})
        self.assertEqual(a['summary'], 'Pick, now please')

    def test_json_is_found_inside_fences(self):
        self.assertEqual(self.sb._parse_json_object('```json\n{"kind": "open"}\n```'), {'kind': 'open'})
        self.assertIsNone(self.sb._parse_json_object('no json here'))


class BuildAskTest(RpcBase):
    def setUp(self):
        super().setUp()
        sb_options(self.sb, ai_ask_cards=True)  # these test the Haiku pass, which is opt-in
        rows = [user('2026-10-01T00:00:00Z', 'plan it'),
                assistant('2026-10-01T00:00:05Z', 'Option 1 (I recommend it) or option 2?'),
                entry('user', '2026-10-01T00:00:06Z', isMeta=True, message={'role': 'user', 'content': 'caveat'}),
                entry('user', '2026-10-01T00:00:07Z', message={'role': 'user', 'content': '<command-name>/exit</command-name>'})]
        self.path = write_transcript(self.cdir / 'projects' / 'p1', rows, name=f'{SID_A}.jsonl')
        os.utime(self.path, (RPC_NOW - 60, RPC_NOW - 60))
        self.seen = []

    def runner(self, out):
        def run(text, prompt):
            self.seen.append((text, prompt))
            if isinstance(out, Exception):
                raise out
            return out
        return run

    def test_haiku_card_is_cached_for_the_mtime(self):
        out = json.dumps({'kind': 'choose', 'summary': 'Pick an option',
                          'options': [{'label': 'Option 1', 'reply': 'Option 1.', 'recommended': True},
                                      {'label': 'Option 2', 'reply': 'Option 2.'}]})
        a = self.sb.build_ask(self.path, SID_A, None, runner=self.runner(out))
        self.assertEqual((a['kind'], a['source'], a['for']), ('choose', 'haiku', RPC_NOW - 60))
        self.assertIn('Option 1 (I recommend it) or option 2?', self.seen[0][0])  # past the /exit noise
        self.assertIn('Never answer for the user', self.seen[0][1])
        cached = json.loads((self.sb.paths()['state'] / f'{SID_A}.ask.json').read_text())
        self.assertEqual(cached, {'for': RPC_NOW - 60, 'ask': a})
        got = self.sb.session_ask(SID_A, Path(self.path), RPC_NOW - 60, None, None, RPC_NOW)
        self.assertEqual(got, a)
        spawned = []
        self.sb.spawn_ask = lambda *x: spawned.append(x)
        later = self.sb.session_ask(SID_A, Path(self.path), RPC_NOW - 30, None, 'Pick one', RPC_NOW)  # changed since
        self.assertEqual((later['kind'], later['source'], later['summary']), ('open', 'claude_code', 'Pick one'))
        self.assertEqual(len(spawned), 1)

    def test_a_failed_runner_caches_the_needs_card(self):
        self.cdir.joinpath('jobs', 'abcd1234').mkdir(parents=True)
        self.cdir.joinpath('jobs', 'abcd1234', 'state.json').write_text(json.dumps({'needs': 'Pick option 1 or 2'}))
        a = self.sb.build_ask(self.path, SID_A, 'abcd1234', runner=self.runner(RuntimeError('logged out')))
        self.assertEqual((a['kind'], a['source'], a['summary'], a['for']), ('open', 'claude_code', 'Pick option 1 or 2',
                                                                            RPC_NOW - 60))
        spawned = []
        self.sb.spawn_ask = lambda *x: spawned.append(x)
        cache = self.sb.paths()['state'] / f'{SID_A}.ask.json'
        rec = json.loads(cache.read_text())
        self.assertEqual(rec['tries'], 1)
        at = rec['retry_at'] = RPC_NOW + 1800  # on the test's clock
        cache.write_text(json.dumps(rec))
        self.assertEqual(self.sb.session_ask(SID_A, Path(self.path), RPC_NOW - 60, 'abcd1234', 'x', at - 1), a)
        self.assertEqual(spawned, [])  # waits its turn, never loops
        self.sb.session_ask(SID_A, Path(self.path), RPC_NOW - 60, 'abcd1234', 'x', at + 1)
        self.assertEqual(len(spawned), 1)  # then tries again
        self.sb.ask_release()
        for _ in range(2):
            self.sb.build_ask(self.path, SID_A, 'abcd1234', runner=self.runner(RuntimeError('logged out')))
        self.assertNotIn('retry_at', json.loads(cache.read_text()))  # 3 tries in all, then it rests on the needs card
        self.sb.session_ask(SID_A, Path(self.path), RPC_NOW - 60, 'abcd1234', 'x', at + 99999)
        self.assertEqual(len(spawned), 1)

    def test_old_answer_cards_are_pruned(self):
        state = self.sb.paths()['state']
        state.mkdir(parents=True, exist_ok=True)
        old, fresh, other = state / 'old.ask.json', state / 'fresh.ask.json', state / 'old.json'
        for f in (old, fresh, other):
            f.write_text('{}')
        now = time.time()
        for f in (old, other):
            os.utime(f, (now - 15 * 86400, now - 15 * 86400))
        self.assertEqual(self.sb.prune_asks(now), 1)
        self.assertEqual((old.exists(), fresh.exists(), other.exists()), (False, True, True))

    def test_garbage_from_haiku_is_open(self):
        a = self.sb.build_ask(self.path, SID_A, None, runner=self.runner('Sure! Here you go.'))
        self.assertEqual((a['kind'], a['source']), ('open', 'claude_code'))

    def test_a_pending_question_tool_skips_haiku(self):
        p = write_transcript(self.cdir / 'projects' / 'p1', [user('2026-10-01T00:00:00Z', 'go'), ask_use()],
                             name=f'{SID_A}.jsonl')
        a = self.sb.build_ask(p, SID_A, None, runner=self.runner(AssertionError('not called')))
        self.assertEqual(a['source'], 'question_tool')
        self.assertEqual(self.seen, [])

    def test_the_detached_pass_frees_the_slot(self):
        self.assertTrue(self.sb.ask_slot(RPC_NOW))
        self.sb.run_ask_haiku = self.runner(json.dumps({'kind': 'open', 'summary': 'Reply'}))
        self.sb.run_need_ask(self.path, SID_A, '')
        self.assertFalse(self.sb._ask_lock().exists())
        self.assertEqual(json.loads((self.sb.paths()['state'] / f'{SID_A}.ask.json').read_text())['ask']['summary'], 'Reply')


class ListAskTest(RpcBase):
    SHORT = 'bbbbbbbb'

    def setUp(self):
        super().setUp()
        sb_options(self.sb, ai_ask_cards=True)

    def row(self, rows=None, **a):
        if rows is None:
            self.transcript(SID_A, str(self.root / 'proj'), RPC_NOW - 60)
        else:
            p = write_transcript(self.cdir / 'projects' / 'p1', rows, name=f'{SID_A}.jsonl')
            os.utime(p, (RPC_NOW - 60, RPC_NOW - 60))
        ag = dict({'id': self.SHORT, 'sessionId': SID_A, 'kind': 'background', 'pid': 4242}, **a)
        r = self.sb.rpc({'verb': 'list'}, RPC_NOW, agents=[ag])
        return next(s for s in r['sessions'] if s['sid'] == SID_A)

    def test_list_never_waits_on_haiku_and_starts_one_pass_at_a_time(self):
        spawned = []
        self.sb.spawn_ask = lambda *x: spawned.append(x)
        self.sb.run_ask_haiku = lambda *x: time.sleep(30)  # would hang if list ran it
        d = self.cdir / 'jobs' / self.SHORT
        d.mkdir(parents=True)
        (d / 'state.json').write_text(json.dumps({'needs': 'Answer 3 questions', 'suggestedReply': 'Not a member'}))
        t = time.time()
        s = self.row(status='idle', state='blocked')
        self.assertLess(time.time() - t, 5)
        self.assertEqual(s['ask'], {'kind': 'open', 'summary': 'Answer 3 questions', 'source': 'claude_code',
                                    'for': RPC_NOW - 60})
        self.assertEqual(s['suggested_reply'], 'Not a member')  # kept for old clients
        self.assertEqual(len(spawned), 1)
        self.assertEqual(spawned[0][1:], (SID_A, self.SHORT))
        self.row(status='idle', state='blocked')
        self.assertEqual(len(spawned), 1)  # the slot is still held

    def test_a_question_tool_card_shows_at_once(self):
        spawned = []
        self.sb.spawn_ask = lambda *x: spawned.append(x)
        s = self.row([dict(user('2026-10-01T00:00:00Z', 'go'), cwd=str(self.root / 'proj')), ask_use()], status='waiting')
        self.assertEqual((s['ask']['kind'], s['ask']['source']), ('questions', 'question_tool'))
        self.assertEqual(spawned, [])

    def test_a_working_chat_has_no_ask(self):
        self.assertNotIn('ask', self.row(status='busy', state='running'))

    def test_a_spawn_failure_falls_back_and_frees_the_slot(self):
        self.sb.spawn_ask = lambda *x: (_ for _ in ()).throw(OSError('no python'))
        s = self.row(status='idle', state='blocked')
        self.assertEqual((s['ask']['kind'], s['ask']['source']), ('open', 'claude_code'))
        self.assertFalse(self.sb._ask_lock().exists())

    def test_a_busy_chat_starts_no_pass(self):
        spawned = []
        self.sb.spawn_ask = lambda *x: spawned.append(x)
        s = self.row(status='busy', state='blocked')
        self.assertEqual((s['ask']['kind'], s['ask']['source']), ('open', 'claude_code'))
        self.assertEqual(spawned, [])
        self.assertFalse(self.sb._ask_lock().exists())

    def test_a_stale_slot_is_taken_over(self):
        self.sb._ask_lock().write_text(str(RPC_NOW - self.sb.ASK_LOCK_TTL - 5))
        spawned = []
        self.sb.spawn_ask = lambda *x: spawned.append(x)
        self.row(status='idle', state='blocked')
        self.assertEqual(len(spawned), 1)


class ServerPassThroughTest(unittest.TestCase):
    def test_ask_reaches_api_sessions_untouched(self):
        ask = {'kind': 'questions', 'summary': 'Answer 1 question', 'source': 'haiku', 'for': NOW - 60,
               'questions': [{'q': 'Are you a member?', 'options': ['Yes', 'No'], 'multi': False, 'about_you': True,
                              'recommended': None}]}
        row = {'sid': SID_A, 'title': 't', 'mtime': NOW - 60, 'active': NOW - 60, 'kind': 'background', 'status': 'idle',
               'state': 'blocked', 'needs': 'Pick one', 'ask': ask}
        f = fleet.Fleet(MACH[:1], lambda *a: {'ok': True, 'sessions': [row]}, clock=lambda: NOW)
        state = Path(mkdtemp())
        app = server.App(f, state_dir=state, web_dir=state)
        f.poll()
        s = app.get('/api/sessions')['machines'][0]['sessions'][0]
        self.assertEqual(s['ask'], ask)


if __name__ == '__main__':
    unittest.main()
