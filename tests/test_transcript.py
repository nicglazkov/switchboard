"""A chat as its messages (agent: rpc_transcript): what the phones' chat page draws, a slice of the transcript at a
time, read by position."""
import json, os, unittest
from pathlib import Path
from tests.helpers import entry, user, assistant
from tests.test_rpc import RpcBase, NOW

SID = '0a0a0a0a-1111-2222-3333-444444444444'


def tool(ts, name, inp, tid, **kw):
    return entry('assistant', ts, uuid='a-' + tid, message={'role': 'assistant', 'content': [{'type': 'tool_use', 'id': tid, 'name': name, 'input': inp}]}, **kw)


def result(ts, tid, content, extra=None, error=False):
    b = {'type': 'tool_result', 'tool_use_id': tid, 'content': content}
    if error:
        b['is_error'] = True
    d = entry('user', ts, uuid='r-' + tid, message={'role': 'user', 'content': [b]})
    if extra is not None:
        d['toolUseResult'] = extra
    return d


class TranscriptTest(RpcBase):
    def write(self, rows, sid=SID):
        p = self.cdir / 'projects' / 'p1' / f'{sid}.jsonl'
        p.parent.mkdir(parents=True, exist_ok=True)
        p.write_bytes(b''.join(json.dumps(r).encode() + b'\n' for r in rows))
        return p

    def read(self, **kw):
        r = self.sb.rpc(dict({'verb': 'transcript', 'sid': SID}, **kw), NOW, agents=[])
        self.assertTrue(r['ok'], r)
        return r

    def kinds(self, r):
        return [i['k'] for i in r['items']]

    def test_messages_tools_and_results_come_out_as_items(self):
        patch = [{'oldStart': 10, 'oldLines': 2, 'newStart': 10, 'newLines': 3, 'lines': [' keep', '-old', '+new', '+more']}]
        self.write([
            user('2026-10-01T00:00:01Z', 'Move the promo field', uuid='u1'),
            entry('assistant', '2026-10-01T00:00:02Z', uuid='t1', thinkingDurationMs=8100,
                  message={'content': [{'type': 'thinking', 'thinking': 'Find it first.'}, {'type': 'text', 'text': 'I will look.'}]}),
            tool('2026-10-01T00:00:03Z', 'Read', {'file_path': '/w/src/a.tsx'}, 'tu1'),
            result('2026-10-01T00:00:04Z', 'tu1', 'line\n' * 40, {'type': 'text', 'file': {'filePath': '/w/src/a.tsx', 'numLines': 212}}),
            tool('2026-10-01T00:00:05Z', 'Edit', {'file_path': '/w/src/a.tsx', 'old_string': 'old', 'new_string': 'new'}, 'tu2'),
            result('2026-10-01T00:00:06Z', 'tu2', 'The file was updated', {'filePath': '/w/src/a.tsx', 'structuredPatch': patch}),
            tool('2026-10-01T00:00:07Z', 'Bash', {'command': 'npm   test\n--run', 'description': 'Run the tests'}, 'tu3'),
            result('2026-10-01T00:00:08Z', 'tu3', 'Exit code 1', {'stdout': '2 failed', 'stderr': 'relation missing', 'interrupted': False}, error=True),
            entry('assistant', '2026-10-01T00:00:09Z', uuid='end', message={'stop_reason': 'end_turn', 'content': [{'type': 'text', 'text': 'Done in `a.tsx`.'}]}),
        ])
        r = self.read()
        self.assertEqual(self.kinds(r), ['user', 'think', 'text', 'tool', 'result', 'tool', 'result', 'tool', 'result', 'text'])
        it = r['items']
        self.assertEqual(it[0]['text'], 'Move the promo field')
        self.assertEqual((it[1]['ms'], it[1]['text']), (8100, 'Find it first.'))
        self.assertEqual((it[3]['verb'], it[3]['path'], it[3]['id']), ('Read', '/w/src/a.tsx', 'tu1'))
        self.assertEqual((it[4]['id'], it[4]['lines']), ('tu1', 212))
        self.assertNotIn('out', it[4])  # a read file is opened from its path, not sent again
        self.assertEqual(it[5]['verb'], 'Edited')
        self.assertEqual(it[6]['patch'], [['c', 10, 10, 'keep'], ['d', 11, 0, 'old'], ['a', 0, 11, 'new'], ['a', 0, 12, 'more']])
        self.assertEqual((it[6]['add'], it[6]['cut']), (2, 1))
        self.assertEqual((it[7]['verb'], it[7]['arg'], it[7]['note']), ('Ran', 'npm test --run', 'Run the tests'))
        self.assertEqual(it[7]['cmd'], 'npm   test\n--run')
        self.assertTrue(it[8]['err'])
        self.assertEqual(it[8]['out'], '2 failed\nrelation missing')
        self.assertEqual(len({i['id'] for i in it if i['k'] != 'result'}), 7)  # every item can be told apart
        self.assertFalse(r['working'])
        self.assertFalse(r['more'])
        self.assertEqual((r['start'], r['end']), (0, r['size']))

    def test_what_claude_code_writes_under_the_users_name_is_not_a_message(self):
        self.write([
            user('2026-10-01T00:00:01Z', '<task-notification>done</task-notification>'),
            user('2026-10-01T00:00:02Z', '<command-name>/model</command-name><command-args>opus</command-args>'),
            user('2026-10-01T00:00:03Z', '[Request interrupted by user]'),
            user('2026-10-01T00:00:04Z', 'hidden', isMeta=True),
            user('2026-10-01T00:00:05Z', 'Here it is.\n<pasted_content id="1a">\none\ntwo\nthree\n</pasted_content id="1a">'),
            user('2026-10-01T00:00:06Z', 'typed<system-reminder>not for people</system-reminder>'),
            entry('user', '2026-10-01T00:00:07Z', isCompactSummary=True, message={'content': 'This session is being continued from...'}),
            entry('system', '2026-10-01T00:00:08Z', subtype='compact_boundary', content='Conversation compacted'),
            assistant('2026-10-01T00:00:09Z', 'from a helper', side=True),
            entry('assistant', '2026-10-01T00:00:10Z', isApiErrorMessage=True, message={'content': [{'type': 'text', 'text': 'API Error: overloaded'}]}),
        ])
        it = self.read()['items']
        self.assertEqual([(i['k'], i.get('what')) for i in it],
                         [('note', 'command'), ('note', 'stopped'), ('user', None), ('user', None), ('note', 'compacted'), ('note', 'compacted'), ('note', 'error')])
        self.assertEqual(it[0]['text'], '/model opus')
        self.assertEqual((it[2]['text'], it[2]['pasted'], it[2]['paste']), ('Here it is.', 3, 'one\ntwo\nthree'))
        self.assertEqual(it[3]['text'], 'typed')
        self.assertEqual(it[6]['text'], 'API Error: overloaded')

    def test_to_dos_questions_plans_agents_and_other_tools(self):
        self.write([
            tool('2026-10-01T00:00:01Z', 'TodoWrite', {'todos': [{'content': 'Find it', 'status': 'completed'}, {'content': 'Move it', 'status': 'in_progress'}, {'content': 'x', 'status': 'odd'}]}, 'a'),
            tool('2026-10-01T00:00:02Z', 'TaskCreate', {'subject': 'Run the tests'}, 'b'),
            result('2026-10-01T00:00:02Z', 'b', 'Task #4 created', {'task': {'id': '4', 'subject': 'Run the tests'}}),
            tool('2026-10-01T00:00:03Z', 'TaskUpdate', {'taskId': '4', 'status': 'completed'}, 'c'),
            tool('2026-10-01T00:00:04Z', 'AskUserQuestion', {'questions': [{'question': 'Which one?', 'options': [{'label': 'A'}, {'label': 'B'}]}]}, 'd'),
            tool('2026-10-01T00:00:05Z', 'ExitPlanMode', {'plan': '1. Do it'}, 'e'),
            tool('2026-10-01T00:00:06Z', 'Agent', {'description': 'Look for coupon code', 'subagent_type': 'Explore', 'prompt': 'Find every place'}, 'f'),
            result('2026-10-01T00:00:07Z', 'f', [{'type': 'text', 'text': 'Found 3'}], {'totalDurationMs': 38000, 'totalTokens': 21000, 'totalToolUseCount': 14}),
            tool('2026-10-01T00:00:08Z', 'mcp__claude-in-chrome__read_page', {'tabId': 1}, 'g'),
            tool('2026-10-01T00:00:09Z', 'SomethingNew', {'query': 'what is this'}, 'h'),
            tool('2026-10-01T00:00:10Z', 'Grep', {'pattern': 'applyCoupon', 'path': 'src'}, 'i'),
        ])
        it = {i['id']: i for i in self.read()['items'] if i['k'] == 'tool'}
        res = {i['id']: i for i in self.read()['items'] if i['k'] == 'result'}
        self.assertEqual(it['a']['todos'], [{'t': 'Find it', 's': 'completed'}, {'t': 'Move it', 's': 'in_progress'}, {'t': 'x', 's': 'pending'}])
        self.assertEqual(it['b']['todo'], {'t': 'Run the tests'})
        self.assertEqual(res['b']['todo_n'], '4')
        self.assertEqual(it['c']['todo'], {'n': '4', 's': 'completed', 't': None})
        self.assertEqual(it['d']['questions'], [{'q': 'Which one?', 'options': ['A', 'B']}])
        self.assertEqual(it['e']['plan'], '1. Do it')
        self.assertEqual((it['f']['verb'], it['f']['arg']), ('Explore agent', 'Look for coupon code'))
        self.assertEqual((res['f']['ms'], res['f']['tokens'], res['f']['steps'], res['f']['out']), (38000, 21000, 14, 'Found 3'))
        self.assertEqual((it['g']['verb'], it['g']['arg']), ('Used', 'claude-in-chrome: read page'))
        self.assertEqual((it['h']['verb'], it['h']['arg']), ('Used', 'what is this'))
        self.assertEqual((it['i']['verb'], it['i']['arg'], it['i']['path']), ('Searched for', 'applyCoupon', 'src'))

    def test_long_things_are_cut_and_say_so(self):
        sb = self.sb
        big = [{'oldStart': 1, 'newStart': 1, 'lines': ['+' + 'x' * 900] * (sb.CHAT_PATCH_LINES + 25)}]
        self.write([
            assistant('2026-10-01T00:00:01Z', 'w' * (sb.CHAT_TEXT * 2)),
            tool('2026-10-01T00:00:02Z', 'Bash', {'command': 'cat big'}, 'a'),
            result('2026-10-01T00:00:03Z', 'a', '', {'stdout': 'A' + 'o' * (sb.CHAT_OUT * 3) + 'Z', 'stderr': ''}),
            tool('2026-10-01T00:00:04Z', 'Write', {'file_path': '/w/n.txt', 'content': 'x'}, 'b'),
            result('2026-10-01T00:00:05Z', 'b', 'ok', {'type': 'update', 'structuredPatch': big}),
        ])
        it = self.read()['items']
        self.assertLessEqual(len(it[0]['text']), sb.CHAT_TEXT + 8)
        self.assertIn('[...]', it[0]['text'])
        out = it[2]['out']
        self.assertTrue(it[2]['clipped'])
        self.assertTrue(out.startswith('A') and out.endswith('Z') and len(out) <= sb.CHAT_OUT + 8)
        self.assertEqual((len(it[4]['patch']), it[4]['patch_left'], it[4]['add']), (sb.CHAT_PATCH_LINES, 25, sb.CHAT_PATCH_LINES + 25))
        self.assertEqual(len(it[4]['patch'][0][3]), sb.CHAT_LINE)

    def test_paging_back_and_asking_for_what_is_new(self):
        sb = self.sb
        pad = 'p' * 3000
        rows = [assistant(f'2026-10-01T00:{i // 60:02d}:{i % 60:02d}Z', f'message {i} {pad}') for i in range(400)]
        p = self.write(rows)
        self.assertGreater(p.stat().st_size, sb.CHAT_WINDOW * 2)
        r = self.read()
        seen, pages = [i['text'].split()[1] for i in r['items']], 1
        self.assertTrue(r['more'])
        self.assertEqual(seen[-1], '399')
        end = r['end']
        self.assertEqual(end, p.stat().st_size)
        while r['more']:
            r = self.read(before=r['start'])
            seen = [i['text'].split()[1] for i in r['items']] + seen
            pages += 1
            self.assertLess(pages, 50)
        self.assertEqual(seen, [str(i) for i in range(400)])  # every message once, in order
        self.assertGreater(pages, 2)
        # nothing new, then two more lines, the second still being written
        self.assertEqual(self.read(after=end)['items'], [])
        with open(p, 'ab') as fh:
            fh.write(json.dumps(assistant('2026-10-01T01:00:00Z', 'newest')).encode() + b'\n' + b'{"type": "assistant", "mess')
        r = self.read(after=end)
        self.assertEqual([i['text'] for i in r['items']], ['newest'])
        self.assertEqual(r['start'], end)
        with open(p, 'ab') as fh:
            fh.write(b'age": {"content": [{"type": "text", "text": "the rest"}]}}\n')
        self.assertEqual([i['text'] for i in self.read(after=r['end'])['items']], ['the rest'])
        # a transcript that is no longer the one the position came from: start over from its end
        r = self.read(after=p.stat().st_size + 10)
        self.assertTrue(r['reset'])
        self.assertEqual(r['items'][-1]['text'], 'the rest')
        self.assertTrue(self.read(after=0)['reset'])  # too far behind to send in one answer

    def test_an_answer_stays_small_and_never_holds_half_a_record(self):
        sb = self.sb
        blocks = [{'type': 'text', 'text': f'part {n} ' + 'z' * 20_000} for n in range(4)]
        rows = [entry('assistant', '2026-10-01T00:00:01Z', uuid=f'm{i}', message={'content': blocks}) for i in range(6)]
        self.write(rows)
        r = self.read()
        self.assertLess(len(json.dumps(r)), sb.CHAT_BUDGET + 90_000)
        self.assertTrue(r['more'])
        self.assertEqual(len(r['items']) % 4, 0)
        self.assertEqual(r['items'][0]['text'].split()[1], '0')
        got = len(r['items'])
        while r['more']:
            r = self.read(before=r['start'])
            got += len(r['items'])
        self.assertEqual(got, 24)
        # one record bigger than a whole answer still comes, alone
        self.write([entry('assistant', '2026-10-01T00:00:01Z', uuid='huge', message={'content': [{'type': 'text', 'text': 'q' * 23_000}] * 20})])
        r = self.read()
        self.assertEqual((len(r['items']), r['start'], r['more']), (20, 0, False))

    def test_a_line_longer_than_a_window_is_stepped_over(self):
        sb = self.sb
        p = self.write([assistant('2026-10-01T00:00:01Z', 'first')])
        with open(p, 'ab') as fh:
            fh.write(b'{"type": "api-request-blob", "data": "' + b'd' * (sb.CHAT_WINDOW * 2 + 500) + b'"}\n')
            fh.write(json.dumps(assistant('2026-10-01T00:00:03Z', 'last')).encode() + b'\n')
        r = self.read()
        texts, turns = [i['text'] for i in r['items']], 0
        while r['more']:
            r = self.read(before=r['start'])
            texts = [i['text'] for i in r['items']] + texts
            turns += 1
            self.assertLess(turns, 10)
        self.assertEqual(texts, ['first', 'last'])

    def test_working_is_said_while_a_turn_is_open(self):
        self.write([user('2026-10-01T00:00:01Z', 'go'), tool('2026-10-01T00:00:02Z', 'Bash', {'command': 'sleep 9'}, 'a')])
        self.assertTrue(self.read()['working'])

    def test_bad_requests_are_refused(self):
        self.write([user('2026-10-01T00:00:01Z', 'hi')])
        for bad in ({'sid': 'nope'}, {'before': -1}, {'after': '5'}, {'before': True}, {'sid': '9' * 8 + '-1111-2222-3333-' + '4' * 12}):
            r = self.sb.rpc(dict({'verb': 'transcript', 'sid': SID}, **bad), NOW, agents=[])
            self.assertFalse(r['ok'], bad)
        self.assertIn('transcript', self.sb.AGENT_FEATURES)

    def test_an_empty_or_odd_transcript_reads_as_nothing(self):
        p = self.write([])
        r = self.read()
        self.assertEqual((r['items'], r['more'], r['end']), ([], False, 0))
        p.write_bytes(b'[1, 2]\n"words"\nnull\n{"type": "user", "message": "not an object"}\n{"type": "assistant", "message": {"content": [null, 3, {"type": "text", "text": 5}, {"type": "tool_use"}]}}\n')
        r = self.read()
        self.assertEqual(self.kinds(r), ['tool'])


if __name__ == '__main__':
    unittest.main()
