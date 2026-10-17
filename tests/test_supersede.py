import json, os, unittest
from tests.helpers import user, assistant, entry, write_transcript
from tests.test_rpc import RpcBase, NOW, SID_A, SID_B, _iso

SID_C = 'cccccccc-1111-2222-3333-444444444444'
SID_D = 'dddddddd-1111-2222-3333-444444444444'


class SupersedeTest(RpcBase):
    def chat(self, sid, title, started, last, prompt=None, entrypoint='cli', custom=True, cwd=None):
        cwd, prompt = cwd or str(self.root / 'proj'), prompt or f'first thing I asked in {sid[:8]}'
        rows = [dict(user(_iso(started), prompt), cwd=cwd, entrypoint=entrypoint, sessionId=sid),
                dict(assistant(_iso(last), 'ok', usage={'input_tokens': 9000}), cwd=cwd, sessionId=sid)]
        if title:
            rows.append({'type': 'custom-title' if custom else 'ai-title', 'customTitle' if custom else 'aiTitle': title, 'sessionId': sid})
        p = write_transcript(self.cdir / 'projects' / 'p1', rows, name=f'{sid}.jsonl')
        os.utime(p, (last + 1, last + 1))
        return p

    def listing(self, now=NOW, agents=()):
        r = self.sb.rpc({'verb': 'list'}, now, agents=list(agents))
        self.assertTrue(r['ok'], r)
        return [s['sid'] for s in r['sessions']], {x['sid']: x for x in r['replaced']}, r

    def test_start_fresh_by_name_hides_the_source(self):
        self.chat(SID_A, 'Garden planner', NOW - 9000, NOW - 5000, custom=False)
        self.chat(SID_B, 'Garden planner (fresh)', NOW - 4000, NOW - 100, prompt='[switchboard:abcdefabcdef] Continue')
        shown, gone, _ = self.listing()
        self.assertEqual(shown, [SID_B])
        self.assertEqual(gone[SID_A]['how'], 'fresh')
        self.assertEqual(gone[SID_A]['by'], {'sid': SID_B, 'machine': self.sb.machine_name()})
        self.assertEqual(gone[SID_A]['title'], 'Garden planner')

    def test_legacy_fresh_suffix_and_chains(self):
        self.chat(SID_A, 'Claude plan usage', NOW - 9000, NOW - 8000)
        self.chat(SID_B, 'Claude plan usage fresh', NOW - 7000, NOW - 6000)
        self.chat(SID_C, 'Claude plan usage (fresh 2)', NOW - 5000, NOW - 100)
        shown, gone, _ = self.listing()
        self.assertEqual(shown, [SID_C])
        self.assertEqual(set(gone), {SID_A, SID_B})
        self.assertEqual(gone[SID_A]['by']['sid'], SID_B)  # the closest successor
        self.assertEqual(gone[SID_B]['by']['sid'], SID_C)

    def test_continued_and_resumed_tags(self):
        self.chat(SID_A, 'Lights', NOW - 9000, NOW - 8000)
        self.chat(SID_B, 'Lights (from Laptop)', NOW - 7000, NOW - 100)
        self.chat(SID_C, 'Router', NOW - 9000, NOW - 8000)
        self.chat(SID_D, 'Router (resumed)', NOW - 7000, NOW - 200)
        shown, gone, _ = self.listing()
        self.assertEqual(shown, [SID_B, SID_D])
        self.assertEqual((gone[SID_A]['how'], gone[SID_C]['how']), ('continued', 'resumed'))

    def test_a_chat_marked_replaced_by_itself_stays(self):
        self.chat(SID_A, 'Garden Planner (fresh)', NOW - 4000, NOW - 3000)
        self.sb.mark_replaced(SID_A, 'resumed', NOW - 100, {'machine': self.sb.machine_name(), 'sid': SID_A})
        shown, gone, _ = self.listing()
        self.assertEqual(shown, [SID_A])
        self.assertEqual(gone, {})

    def test_copies_of_a_copy_keep_one_chat_on_the_list(self):
        # 2026-10-13, Garden Planner: a fresh chat, two resumed copies of it (same first message), and the fresh chat
        # marked as replaced by itself. Every one of them was hidden behind another hidden one.
        p = 'Continue from the handoff'
        self.chat(SID_A, 'Oct 13 3:47a Garden Planner (fresh)', NOW - 4000, NOW - 3800, prompt=p)
        self.chat(SID_B, 'Oct 13 3:50a Garden Planner (resumed)', NOW - 4000, NOW - 3800, prompt=p)
        self.chat(SID_C, 'Oct 13 3:50a Garden Planner (resumed)', NOW - 4000, NOW - 3800, prompt=p)
        self.sb.mark_replaced(SID_A, 'resumed', NOW - 3700, {'machine': self.sb.machine_name(), 'sid': SID_A})
        shown, gone, _ = self.listing()
        self.assertEqual(len(shown), 1, (shown, {k: (v['how'], v['by']) for k, v in gone.items()}))
        for sid, g in gone.items():  # each hidden one points, step by step, at the one that is shown
            seen = {sid}
            while g['by']['sid'] not in shown:
                g = gone[g['by']['sid']]
                self.assertNotIn(g['sid'], seen)
                seen.add(g['sid'])

    def test_the_old_one_comes_back_when_used_again(self):
        self.chat(SID_A, 'Lights', NOW - 9000, NOW - 50)  # replied after the fresh one started
        self.chat(SID_B, 'Lights (fresh)', NOW - 4000, NOW - 100)
        shown, gone, _ = self.listing()
        self.assertEqual(shown, [SID_A, SID_B])
        self.assertEqual(gone, {})

    def test_new_sessions_and_other_topics_stay(self):
        self.chat(SID_A, 'Lights', NOW - 9000, NOW - 8000)
        self.chat(SID_B, 'Lights (new, iPhone)', NOW - 4000, NOW - 100)
        self.chat(SID_C, 'Lights out', NOW - 9000, NOW - 8500)
        self.chat(SID_D, 'Lights two (fresh)', NOW - 4000, NOW - 200)
        shown, gone, _ = self.listing()
        self.assertEqual(set(shown), {SID_A, SID_B, SID_C, SID_D})

    def test_a_copy_made_by_resume_folds_into_the_one_in_use(self):
        self.chat(SID_A, 'Kitchen / remodel', NOW - 90000, NOW - 80000, prompt='Look at my desk')
        self.chat(SID_B, 'Kitchen / remodel', NOW - 90000, NOW - 100, prompt='Look at my desk')
        shown, gone, _ = self.listing()
        self.assertEqual(shown, [SID_B])
        self.assertEqual(gone[SID_A]['how'], 'resumed')

    def test_a_busy_session_is_never_hidden(self):
        self.chat(SID_A, 'Lights', NOW - 9000, NOW - 8000)
        self.chat(SID_B, 'Lights (fresh)', NOW - 4000, NOW - 100)
        agents = [{'sessionId': SID_A, 'id': 'aaaaaaaa', 'kind': 'background', 'status': 'busy', 'pid': 1}]
        shown, gone, _ = self.listing(agents=agents)
        self.assertEqual(set(shown), {SID_A, SID_B})

    def test_scripted_runs_of_one_job_fold_into_the_newest(self):
        p = 'New Camino (Kitchen remodel) activity just arrived for the user. Read README.md and act.'
        for i, sid in enumerate((SID_A, SID_B, SID_C)):
            self.chat(sid, None, NOW - 9000 + i * 1000, NOW - 8000 + i * 1000, prompt=p, entrypoint='sdk-cli')
        self.chat(SID_D, 'Same words, typed by a person', NOW - 500, NOW - 100, prompt=p, entrypoint='cli')
        shown, gone, r = self.listing()
        self.assertEqual(shown, [SID_D, SID_C])
        self.assertEqual(r['sessions'][1]['repeats'], 2)
        self.assertNotIn('repeats', r['sessions'][0])
        self.assertEqual(gone, {})

    def test_same_name_same_repo_keeps_the_newest_unless_the_old_one_runs(self):
        self.chat(SID_A, 'Project recap', NOW - 90000, NOW - 80000)
        self.chat(SID_B, 'Project recap', NOW - 9000, NOW - 100)
        self.chat(SID_C, 'Project recap', NOW - 9500, NOW - 9000, cwd=str(self.root))  # another repo
        shown, gone, _ = self.listing()
        self.assertEqual(shown, [SID_B, SID_C])
        self.assertEqual(gone[SID_A]['how'], 'same-name')
        agents = [{'sessionId': SID_A, 'kind': 'interactive', 'status': 'idle', 'pid': 1}]
        shown, gone, _ = self.listing(agents=agents)  # open in a terminal: stays
        self.assertEqual(shown, [SID_B, SID_C, SID_A])

    def test_a_script_is_known_by_its_tail_when_the_first_message_is_huge(self):
        cwd = str(self.root / 'proj')
        for i, sid in enumerate((SID_A, SID_B)):
            rows = [dict(user(_iso(NOW - 900 + i), 'x' * 300_000), cwd=cwd, entrypoint='sdk-py', sessionId=sid),
                    dict(assistant(_iso(NOW - 800 + i), 'ok'), cwd=cwd, entrypoint='sdk-py', sessionId=sid),
                    {'type': 'ai-title', 'aiTitle': f'Review run {i}', 'sessionId': sid}]
            p = write_transcript(self.cdir / 'projects' / 'p1', rows, name=f'{sid}.jsonl')
            os.utime(p, (NOW - 700 + i, NOW - 700 + i))
        shown, gone, r = self.listing()
        self.assertEqual(len(shown), 2)  # no first message to compare: different titles stay apart
        self.assertTrue(all(s.get('repeats') is None for s in r['sessions']))
        self.assertEqual(self.sb.session_meta(self.cdir / 'projects' / 'p1' / f'{SID_A}.jsonl')['entrypoint'], 'sdk-py')

    def test_supersede_verb_marks_until_claude_replies_again(self):
        p = self.chat(SID_A, 'Lights', NOW - 9000, NOW - 8000)
        r = self.sb.rpc({'verb': 'supersede', 'sid': SID_A, 'by': {'machine': 'laptop', 'sid': SID_B, 'x': 'y'}}, NOW, agents=[])
        self.assertTrue(r['ok'])
        shown, gone, _ = self.listing()
        self.assertEqual(shown, [])
        self.assertEqual(gone[SID_A]['how'], 'continued')
        self.assertEqual(gone[SID_A]['by'], {'machine': 'laptop', 'sid': SID_B})
        with open(p, 'a') as f:
            f.write(json.dumps(assistant(_iso(NOW + 10), 'back again', usage={'input_tokens': 5})) + '\n')
        os.utime(p, (NOW + 11, NOW + 11))
        shown, gone, _ = self.listing(NOW + 20)
        self.assertEqual(shown, [SID_A])

    def test_supersede_verb_rejects_bad_input(self):
        self.assertFalse(self.sb.rpc({'verb': 'supersede', 'sid': '../x'}, NOW, agents=[])['ok'])
        self.sb.rpc({'verb': 'supersede', 'sid': SID_A, 'by': {'machine': 'a b;c', 'sid': 7}}, NOW, agents=[])
        self.assertIsNone(json.loads((self.sb.paths()['state'] / 'replaced.json').read_text())[SID_A]['by'])

    def test_cap_counts_only_shown_sessions(self):
        cap = self.sb.LIST_MAX
        for i in range(cap):
            self.chat(f'{i:08x}-0000-0000-0000-000000000000', f'Topic {i} (fresh)', NOW - 500 + i, NOW - 400 + i)
        for i in range(5):
            self.chat(f'{i:08x}-1111-0000-0000-000000000000', f'Topic {i}', NOW - 90000 + i, NOW - 80000 + i)
        shown, gone, _ = self.listing()
        self.assertEqual(len(shown), cap)
        self.assertTrue(all(s.endswith('0000-000000000000') for s in shown))


class SupersedeHookTest(unittest.TestCase):
    def setUp(self):
        import tempfile
        from tests.helpers import fresh_sb
        self.tmp = tempfile.mkdtemp()
        self.sb = fresh_sb(self.tmp)

    def marks(self):
        return json.loads((self.sb.paths()['state'] / 'replaced.json').read_text())

    def test_clear_in_a_terminal_replaces_the_old_chat(self):
        self.sb.handle_hook({'hook_event_name': 'SessionEnd', 'session_id': SID_A, 'reason': 'clear'}, NOW, 777,
                            spawn=lambda *a: None, notify=lambda e: None)
        self.assertEqual(self.marks()[SID_A]['how'], 'cleared')

    def test_quitting_is_not_replacing(self):
        self.sb.handle_hook({'hook_event_name': 'SessionEnd', 'session_id': SID_A, 'reason': 'prompt_input_exit'}, NOW, 777,
                            spawn=lambda *a: None, notify=lambda e: None)
        self.assertFalse((self.sb.paths()['state'] / 'replaced.json').exists())

    def test_guard_fresh_then_clear_links_the_new_chat(self):
        self.sb.pending_put(777, {'session_id': SID_A, 'transcript_path': '/old', 'held': None, 'ts': NOW})
        self.sb.handoff_path(SID_A).write_text('# Handoff')
        self.sb.handle_hook({'hook_event_name': 'SessionStart', 'source': 'clear', 'session_id': SID_B, 'transcript_path': '/n'},
                            NOW + 5, 777, spawn=lambda *a: None, notify=lambda e: None)
        m = self.marks()[SID_A]
        self.assertEqual((m['how'], m['by']['sid']), ('fresh', SID_B))

    def test_plain_clear_names_the_new_chat(self):
        start = lambda sid, src: self.sb.handle_hook({'hook_event_name': 'SessionStart', 'source': src, 'session_id': sid,
                                                       'transcript_path': '/t'}, NOW, 777, spawn=lambda *a: None, notify=lambda e: None)
        start(SID_A, 'startup')
        end = lambda: self.sb.handle_hook({'hook_event_name': 'SessionEnd', 'session_id': SID_A, 'reason': 'clear'}, NOW + 9, 777,
                                          spawn=lambda *a: None, notify=lambda e: None)
        end()
        start(SID_B, 'clear')
        end()  # a late SessionEnd keeps the name
        m = self.marks()[SID_A]
        self.assertEqual((m['how'], m['by']['sid']), ('cleared', SID_B))

    def test_clear_after_guard_fresh_stays_fresh(self):
        self.sb.handle_hook({'hook_event_name': 'SessionStart', 'source': 'startup', 'session_id': SID_A, 'transcript_path': '/t'},
                            NOW, 777, spawn=lambda *a: None, notify=lambda e: None)
        self.sb.pending_put(777, {'session_id': SID_A, 'transcript_path': '/old', 'held': None, 'ts': NOW})
        self.sb.handoff_path(SID_A).write_text('# Handoff')
        self.sb.handle_hook({'hook_event_name': 'SessionStart', 'source': 'clear', 'session_id': SID_B, 'transcript_path': '/n'},
                            NOW + 5, 777, spawn=lambda *a: None, notify=lambda e: None)
        m = self.marks()[SID_A]
        self.assertEqual((m['how'], m['by']['sid']), ('fresh', SID_B))


if __name__ == '__main__':
    unittest.main()
