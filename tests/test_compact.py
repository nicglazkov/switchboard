"""Compacting a History thread into one fresh chat: each chat's piece (its note, or its condensed transcript), the
combined text, its cost, and the job that writes one note and starts the new chat from it (agent/sb.py)."""
import os, unittest
from tests.test_jobs import JobBase
from tests.test_rpc import NOW
from tests.helpers import user, assistant, write_transcript

SID_1 = '11111111-1111-4111-8111-111111111111'
SID_2 = '22222222-2222-4222-8222-222222222222'
SID_3 = '33333333-3333-4333-8333-333333333333'


class CompactBase(JobBase):
    def chat(self, sid, said, mtime, title):
        rows = [dict(user('2026-10-01T00:00:01Z', said), cwd=str(self.root / 'proj'), sessionId=sid),
                dict(assistant('2026-10-01T00:00:05Z', 'ok, done'), cwd=str(self.root / 'proj'), sessionId=sid),
                {'type': 'ai-title', 'aiTitle': title, 'sessionId': sid}]
        p = write_transcript(self.cdir / 'projects' / 'p1', rows, name=f'{sid}.jsonl')
        os.utime(p, (mtime, mtime))
        return p


class PiecesTest(CompactBase):
    def test_a_current_note_is_used_and_a_chat_without_one_is_condensed(self):
        self.chat(SID_1, 'Always use tabs. Build the parser.', NOW - 9000, 'Parser')
        self.chat(SID_2, 'Never push to main. Add the tests.', NOW - 5000, 'Parser tests')
        self.sb.handoff_path(SID_1).write_text('# Handoff: Parser\n\n## Goal\nA parser.')
        a, b = self.sb.thread_piece(SID_1), self.sb.thread_piece(SID_2)
        self.assertEqual((a['kind'], b['kind']), ('handoff', 'condensed'))
        self.assertIn('## Goal', a['text'])
        self.assertIn('USER: Never push to main.', b['text'])
        self.assertEqual(b['directives'], ['Never push to main.'])
        self.assertGreater(b['tokens'], 0)

    def test_a_note_older_than_the_chat_is_not_used(self):
        p = self.chat(SID_1, 'Build it.', NOW, 'Parser')
        self.sb.handoff_path(SID_1).write_text('# Handoff: stale')
        os.utime(self.sb.handoff_path(SID_1), (1_700_000_000, 1_700_000_000))   # before its last reply
        os.utime(p, (NOW, NOW))
        self.assertEqual(self.sb.thread_piece(SID_1)['kind'], 'condensed')

    def test_rpc_measures_only_its_own_chats(self):
        self.chat(SID_1, 'Build it.', NOW - 100, 'Parser')
        r = self.sb.rpc({'verb': 'thread_pieces', 'sids': [SID_1, SID_3], 'measure': True}, NOW, agents=[])
        self.assertEqual([p['sid'] for p in r['pieces']], [SID_1])      # SID_3 lives on another machine
        self.assertEqual(set(r['pieces'][0]), {'sid', 'kind', 'tokens'})
        bad = self.sb.rpc({'verb': 'thread_pieces', 'sids': ['../x']}, NOW, agents=[])
        self.assertFalse(bad['ok'])


class CombineTest(CompactBase):
    def piece(self, title, kind, text, directives=(), started=None):
        return {'title': title, 'kind': kind, 'text': text, 'directives': list(directives), 'started': started}

    def test_oldest_first_with_every_rule_kept(self):
        out = self.sb.combine_thread([self.piece('One', 'handoff', '## Goal\nfirst', ['Always use tabs.'], 1_760_000_000),
                                      self.piece('Two', 'condensed', 'USER: next', ['Never push to main.', 'Always use tabs.'])])
        self.assertTrue(out.startswith('# Thread of 2 chats, oldest first'))
        self.assertLess(out.index('## Chat 1 of 2: One'), out.index('## Chat 2 of 2: Two'))
        self.assertIn('its handoff note', out)
        self.assertIn('date unknown, condensed transcript', out)
        rules = out.split('## Directives from every chat (verbatim)')[1]
        self.assertEqual(rules.strip().splitlines(), ['- Always use tabs.', '- Never push to main.'])
        self.assertNotIn('\u2014', out)

    def test_over_the_budget_the_oldest_transcript_gives_way_and_notes_stay_whole(self):
        note = '## Goal\n' + 'n' * 5000
        out = self.sb.combine_thread([self.piece('Old', 'condensed', 'a' * 50_000), self.piece('Mid', 'handoff', note),
                                      self.piece('New', 'condensed', 'b' * 20_000)], max_chars=40_000)
        self.assertIn('[... cut to fit the thread ...]', out)
        self.assertIn(note, out)
        self.assertIn('b' * 20_000, out)
        self.assertLess(len(out), 47_000)


class CostTest(CompactBase):
    def test_the_same_weights_as_start_fresh(self):
        c = self.sb.compact_cost(30_000)
        self.assertEqual(c['summary'], round((30_000 + 500) * 1.25 + 2_000 * 5))
        self.assertEqual(c['first'], 12_000)
        self.assertEqual(c['total'], c['summary'] + c['first'])
        self.assertTrue(c['text'].startswith('About 60k up front: writing one summary of the whole thread 48k'))
        self.assertIn('API prices', c['text'])
        r = self.sb.rpc({'verb': 'thread_cost', 'sids': [SID_1], 'tokens': 30_000}, NOW, agents=[])
        self.assertEqual(r['total'], c['total'])


class CompactJobTest(CompactBase):
    def test_one_note_then_a_new_chat_in_the_newest_chats_folder(self):
        self.chat(SID_1, 'Always use tabs.', NOW - 9000, 'Parser')
        self.chat(SID_2, 'Add the tests.', NOW - 100, 'Parser tests')
        pieces = [dict(self.sb.thread_piece(x)) for x in (SID_1, SID_2)]
        seen = []
        self.sb.run_claude_opus = lambda text, prompt, **kw: seen.append((text, prompt)) or '## Goal\nThe whole thread.'
        jid = self.job({'verb': 'compact', 'sid': SID_2, 'pieces': pieces})
        self.drain()
        r = self.sb.rpc({'verb': 'job', 'id': jid}, NOW + 6, agents=[])
        self.assertEqual(r['state'], 'done', r)
        text, prompt = seen[0]
        self.assertIn('## Chat 1 of 2: Parser', text)
        self.assertIn('one thread made of several chats', prompt)
        note = self.sb.handoff_path(f'thread-{SID_2}').read_text()
        self.assertIn('The whole thread.', note)
        argv = self.launched[0]['argv']
        self.assertIn('(compacted)', argv[argv.index('--name') + 1])
        self.assertEqual(self.launched[0]['cwd'], str((self.root / 'proj').resolve()))
        seed = argv[-1]
        self.assertTrue(seed.startswith('[switchboard:'))
        marks = self.sb._jget(self.sb.paths()['state'] / 'replaced.json')
        self.assertEqual(marks[SID_2]['how'], 'fresh')     # the newest chat steps aside; the older ones stay as they are

    def test_a_failed_summary_still_keeps_every_rule(self):
        self.chat(SID_1, 'Always use tabs.', NOW - 9000, 'Parser')
        self.chat(SID_2, 'Never push to main.', NOW - 100, 'Parser tests')
        pieces = [self.sb.thread_piece(x) for x in (SID_1, SID_2)]
        self.job({'verb': 'compact', 'sid': SID_2, 'pieces': pieces})
        self.drain()
        note = self.sb.handoff_path(f'thread-{SID_2}').read_text()
        self.assertIn('Automatic summary failed', note)
        self.assertIn('- Always use tabs.', note)
        self.assertIn('- Never push to main.', note)

    def test_needs_two_chats_and_a_known_newest(self):
        self.chat(SID_2, 'Add the tests.', NOW - 100, 'Parser tests')
        one = [self.sb.thread_piece(SID_2)]
        self.assertFalse(self.sb.rpc({'verb': 'compact', 'sid': SID_2, 'pieces': one}, NOW, agents=[])['ok'])
        self.assertFalse(self.sb.rpc({'verb': 'compact', 'sid': SID_3, 'pieces': one * 2}, NOW, agents=[])['ok'])


if __name__ == '__main__':
    unittest.main()
