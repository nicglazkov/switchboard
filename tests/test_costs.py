"""The one cost model for Resume vs Start fresh (agent/sb.py cost_model), the size bands, and the condensed size."""
import tempfile, unittest
from tests.helpers import fresh_sb, user, assistant, write_transcript


class CostModelTest(unittest.TestCase):
    def setUp(self):
        self.sb = fresh_sb(tempfile.mkdtemp())

    def test_145k_cold_matches_the_measured_example(self):
        c = self.sb.cost_model(145_000, condensed=55_000)
        self.assertEqual(c['resume']['cold'], 290_000)            # a full re-cache at the 1 hour write ratio
        self.assertEqual(c['fresh']['first'], 12_000)             # the note, about 6k, cached for the new chat
        self.assertEqual(c['fresh']['summary'], 79_375)            # 55.5k in (cached 5 min) + 2k out at 5x
        self.assertEqual(c['fresh']['total'], 91_375)
        self.assertEqual(c['pick'], {'warm': 'resume', 'cold': 'fresh'})
        self.assertEqual(c['text']['cold'], 'Start fresh: about 91k (summary 79k + new chat 12k). '
                                            'Resume: about 290k, because the cache is cold and all 145k is read again.')
        self.assertIn('because the cache is warm', c['text']['warm'])
        self.assertEqual(c['resume']['warm'], 14_500)
        self.assertEqual(c['band'], 'medium')

    def test_40k_is_small_and_resume_stays_the_pick(self):
        for warm in (True, False):
            c = self.sb.cost_model(40_000, condensed=12_000)
            self.assertEqual(c['pick']['warm' if warm else 'cold'], 'resume')
        self.assertEqual(c['band'], 'small')
        self.assertEqual(c['band_text']['cold'], '40k, small, so going back is cheap')
        self.assertEqual(c['band_text']['warm'], '40k, reply while warm')

    def test_300k_is_big(self):
        c = self.sb.cost_model(300_000, condensed=55_000)
        self.assertEqual(c['band'], 'big')
        self.assertEqual(c['resume']['cold'], 600_000)
        self.assertEqual(c['pick']['cold'], 'fresh')
        self.assertEqual(c['band_text']['cold'], '300k, big: resuming re-reads all 300k at full price')

    def test_a_written_summary_costs_nothing(self):
        c = self.sb.cost_model(145_000, condensed=55_000, summary_ready=True, note=5_000)
        self.assertEqual(c['fresh']['summary'], 0)
        self.assertEqual(c['fresh']['total'], 10_000)
        self.assertTrue(c['text']['cold'].startswith('Start fresh: about 10k (summary already written, new chat 10k).'))

    def test_no_count_yet_is_a_labelled_guess(self):
        c = self.sb.cost_model(145_000)
        self.assertTrue(c['condensed_guessed'])
        self.assertEqual(c['condensed'], 55_000)
        self.assertIn('guessed from the size', c['how'])

    def test_how_names_every_input_and_ratio(self):
        how = self.sb.cost_model(145_000, condensed=55_000)['how']
        for bit in ('145k of context', 'condensed transcript of 55k', 'cache write 2', 'cache read 0.1', 'output 5',
                    'API prices', 'Plan usage may not weigh'):
            self.assertIn(bit, how)
        self.assertNotIn('\u2014', how)

    def test_bands(self):
        self.assertEqual([self.sb.ctx_band(n) for n in (0, 59_999, 60_000, 149_999, 150_000)],
                         ['small', 'small', 'medium', 'medium', 'big'])
        self.assertEqual(self.sb.band_words(145_000, False),
                         '145k, resuming re-reads 145k; Start fresh is cheaper if you do not need the history')
        self.assertEqual(self.sb.band_words(200_000, True), '200k, reply while warm')
        self.assertEqual(self.sb.band_words(200_000, False), '200k, big: resuming re-reads all 200k at full price')


class CondensedSizeTest(unittest.TestCase):
    def test_counted_once_per_mtime(self):
        tmp = tempfile.mkdtemp(); sb = fresh_sb(tmp)
        t = write_transcript(tmp, [user('2026-10-10T10:00:00Z', 'Remember: never X. ' * 50), assistant('2026-10-10T10:00:05Z', 'ok ' * 300)])
        n = sb.condensed_size(t, mtime=1.0, now=100.0)
        self.assertGreater(n, 100)
        calls = []
        orig = sb.condense
        sb.condense = lambda *a, **k: calls.append(1) or orig(*a, **k)
        self.assertEqual(sb.condensed_size(t, mtime=1.0, now=10_000.0), n)   # same mtime: from the cache
        self.assertEqual(calls, [])
        sb.condensed_size(t, mtime=2.0, now=10_000.0)                        # written to since: counted again
        self.assertEqual(calls, [1])
        sb.condensed_size(t, mtime=3.0, now=10_000.0 + 10)                   # still being written, counted just now
        self.assertEqual(calls, [1])
        sb.condensed_size(t, mtime=4.0, now=10_000.0 + sb.CONDENSED_KEEP + 1)
        self.assertEqual(calls, [1, 1])


class JobProgressTest(unittest.TestCase):
    def test_start_fresh_says_it_is_writing_the_summary(self):
        tmp = tempfile.mkdtemp(); sb = fresh_sb(tmp)
        t = write_transcript(tmp, [user('2026-10-10T10:00:00Z', 'hi'), assistant('2026-10-10T10:00:05Z', 'ok')])
        seen = []
        made = lambda tp, sid: sb.handoff_path(sid).write_text('# Handoff')
        job = {'verb': 'handoff', 'sid': 'S1', 'transcript': t, 'cwd': tmp}
        sb._run_job(job, None, None, made, 100.0, progress=seen.append)
        self.assertEqual(seen, [{'step': 'summary', 'note': 'Writing the summary, about a minute'}])
        seen.clear()
        sb._run_job(job, None, None, made, 100.0, progress=seen.append)   # current: never written twice
        self.assertEqual(seen, [])


class SummaryModelPrefTest(unittest.TestCase):
    def test_the_setting_reaches_every_agent_on_the_poll(self):
        import sys
        from pathlib import Path
        from tempfile import mkdtemp
        sys.path.insert(0, str(Path(__file__).resolve().parent.parent / 'server'))
        import fleet, server
        calls = []
        f = fleet.Fleet([{'name': 'studio', 'label': 'Studio', 'target': 'me@x', 'os': 'mac'}],
                        lambda t, req, timeout: calls.append(req) or {'ok': True, 'sessions': []}, clock=lambda: 100.0)
        app = server.App(f, state_dir=Path(mkdtemp()), web_dir=Path(mkdtemp()), cfg_dir=Path(mkdtemp()), clock=lambda: 100.0)
        self.assertEqual(app.get('/api/prefs')['summary_model'], 'opus')
        app.post('/api/prefs', {'summary_model': 'sonnet'})
        f.poll()
        self.assertEqual(calls[-1]['summary_model'], 'sonnet')
        self.assertEqual(app.sessions()['prefs']['summary_model'], 'sonnet')
        with self.assertRaises(server.HttpError):
            app.post('/api/prefs', {'summary_model': 'haiku'})


if __name__ == '__main__':
    unittest.main()


from tests.test_rpc import RpcBase, NOW, SID_A  # noqa: E402
from tests.helpers import entry  # noqa: E402


class ListCarriesTheCostTest(RpcBase):
    def test_each_chat_says_both_costs_and_how(self):
        import os
        big = {'input_tokens': 0, 'cache_creation_input_tokens': 0, 'cache_read_input_tokens': 145_000, 'output_tokens': 1}
        rows = [dict(user('2026-10-01T00:00:01Z', 'Remember: always run the tests.'), cwd=str(self.root)),
                assistant('2026-10-01T00:00:05Z', 'ok', usage=big), {'type': 'ai-title', 'aiTitle': 'Lights', 'sessionId': SID_A}]
        p = write_transcript(self.cdir / 'projects' / 'p1', rows, name=f'{SID_A}.jsonl')
        os.utime(p, (NOW - 60, NOW - 60))
        s = next(s for s in self.sb.rpc({'verb': 'list'}, NOW, agents=[])['sessions'] if s['sid'] == SID_A)
        self.assertGreater(s['condensed'], 0)
        c = s['cost']
        self.assertEqual((c['ctx'], c['band'], c['condensed_guessed']), (145_000, 'medium', False))
        self.assertEqual(c['resume']['cold'], 290_000)
        self.assertEqual(set(c['text']), {'warm', 'cold'})
        self.assertIn('How this is estimated', c['how'])
