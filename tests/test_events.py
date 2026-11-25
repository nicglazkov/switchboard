"""What a summary cost, and the short log of fresh starts, holds and stops (state/events.json)."""
import datetime as dt, json, tempfile, unittest
from types import SimpleNamespace
from tests.helpers import fresh_sb, user, assistant, write_transcript
from tests.test_rpc import NOW, SID_A
from tests.test_jobs import JobBase

USAGE = {'input_tokens': 10, 'cache_creation_input_tokens': 1_000, 'cache_read_input_tokens': 2_000, 'output_tokens': 100,
         'cache_creation': {'ephemeral_5m_input_tokens': 600, 'ephemeral_1h_input_tokens': 400}}
BIG = {'input_tokens': 0, 'cache_creation_input_tokens': 0, 'cache_read_input_tokens': 640_000, 'output_tokens': 1}


class SummaryCostTest(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.mkdtemp(); self.sb = fresh_sb(self.tmp)
        self.t = write_transcript(self.tmp, [user('2026-10-10T10:00:00Z', 'Always run the tests.'),
                                             assistant('2026-10-10T10:00:05Z', 'Noted.')])

    def test_usage_is_weighed_like_every_other_estimate(self):
        # 10 plain + 600 written for 5 minutes (1.25) + 400 written for an hour (2) + 2,000 read (0.1) + 100 out (5)
        self.assertEqual(self.sb.usage_weight(USAGE), 10 + 750 + 800 + 200 + 500)
        # no split given: the whole write counts as the 5 minute kind, which is what a summary run uses
        self.assertEqual(self.sb.usage_weight({'cache_creation_input_tokens': 1_000, 'output_tokens': 2}), 1_260)
        for junk in (None, {}, [], 'x', {'input_tokens': 'many', 'output_tokens': None}):
            self.assertEqual(self.sb.usage_weight(junk), 0)

    def test_the_summary_run_says_what_it_used(self):
        said = json.dumps({'result': '## Goal\nX', 'usage': USAGE, 'total_cost_usd': 0.42})
        self.sb.run_tree = lambda *a, **kw: SimpleNamespace(stdout=said, stderr='', returncode=0)
        self.sb._claude_exe = lambda: 'claude'
        meter = {}
        self.assertEqual(self.sb.run_claude_opus('text', 'prompt', meter=meter), '## Goal\nX')
        self.assertEqual(meter['weighted'], 2_260)
        self.assertEqual(meter['tokens'], {'input': 10, 'cache_write_5m': 600, 'cache_write_1h': 400, 'cache_read': 2_000, 'output': 100})
        self.assertEqual(meter['usd'], 0.42)
        self.assertEqual(self.sb.run_claude_opus('text', 'prompt'), '## Goal\nX')  # nobody asked: as before

    def test_a_run_that_reports_no_usage_is_not_given_one(self):
        self.sb.run_tree = lambda *a, **kw: SimpleNamespace(stdout=json.dumps({'result': 'ok'}), stderr='', returncode=0)
        self.sb._claude_exe = lambda: 'claude'
        meter = {}
        self.sb.run_claude_opus('text', 'prompt', meter=meter)
        self.assertEqual(meter, {})

    def test_the_cost_is_kept_beside_the_note(self):
        meter = {}

        def runner(text, prompt):
            meter.update(tokens={'input': 10, 'output': 100}, weighted=31_000, usd=0.5)
            return '## Goal\nKeep going.'
        self.sb.generate_handoff(self.t, 'S1', runner=runner, meter=meter)
        c = self.sb.summary_cost('S1')
        self.assertEqual((c['weighted'], c['usd'], c['tokens']), (31_000, 0.5, {'input': 10, 'output': 100}))
        self.assertEqual(c['model'], 'opus')
        self.assertIsInstance(c['at'], float)
        self.assertEqual(sorted(p.name for p in self.sb.paths()['handoffs'].iterdir()), ['S1.cost.json', 'S1.md'])

    def test_a_note_written_without_a_count_has_no_stale_cost(self):
        self.sb._jput(self.sb.paths()['handoffs'] / 'S1.cost.json', {'v': 1, 'weighted': 9, 'at': 1.0})
        self.sb.generate_handoff(self.t, 'S1', runner=lambda text, prompt: '## Goal\nX')
        self.assertIsNone(self.sb.summary_cost('S1'))


class EventLogTest(unittest.TestCase):
    def setUp(self):
        self.sb = fresh_sb(tempfile.mkdtemp())
        self.path = self.sb.paths()['state'] / 'events.json'

    def test_rows_are_added_in_order_and_never_changed(self):
        self.sb.log_event('hold', 100.0, sid='S1', ctx=200_000)
        self.sb.log_event('stop', 101.0, sid='S2', by='user')
        rows = self.sb.events()
        self.assertEqual([(r['kind'], r['at'], r['sid']) for r in rows], [('hold', 100.0, 'S1'), ('stop', 101.0, 'S2')])
        self.assertEqual(rows[0]['ctx'], 200_000)
        self.assertEqual(json.loads(self.path.read_text())['v'], 1)

    def test_the_log_is_bounded(self):
        self.sb.EVENTS_KEEP, self.sb.EVENTS_TRIM = 20, 15
        for i in range(50):
            self.sb.log_event('stop', float(i), sid=f'S{i}')
        rows = self.sb.events()
        self.assertLessEqual(len(rows), 20)
        self.assertEqual(rows[-1]['sid'], 'S49')  # the newest are the ones kept
        self.assertEqual([r['at'] for r in rows], sorted(r['at'] for r in rows))

    def test_long_values_and_unknown_shapes_are_cut_down(self):
        self.sb.log_event('hold', 1.0, sid='S1', title='t' * 500, odd={'a': object()}, none=None)
        r = self.sb.events()[0]
        self.assertEqual(len(r['title']), 120)
        self.assertNotIn('odd', r)
        self.assertNotIn('none', r)

    def test_a_damaged_log_never_stops_the_caller(self):
        self.path.parent.mkdir(parents=True, exist_ok=True)
        self.path.write_text('{"v": 1, "rows": [{"kind"')
        self.sb.log_event('stop', 5.0, sid='S1')
        self.assertEqual([r['sid'] for r in self.sb.events()], ['S1'])
        self.path.write_text(json.dumps({'v': 1, 'rows': 'nope'}))
        self.assertEqual(self.sb.events(), [])
        self.sb.locked_update = lambda *a, **kw: (_ for _ in ()).throw(OSError('disk full'))
        self.sb.log_event('stop', 6.0, sid='S2')  # no error reaches a hook or a job


class HoldEventTest(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.mkdtemp(); self.sb = fresh_sb(self.tmp)
        self.t = write_transcript(self.tmp, [user('2026-10-10T01:00:00Z', 'Remember X.'), assistant('2026-10-10T01:00:05Z', 'ok', usage=BIG)])
        self.now = dt.datetime(2026, 10, 10, 10, tzinfo=dt.timezone.utc).timestamp()

    def hook(self, inp, pid=777):
        return self.sb.handle_hook(inp, self.now, pid, spawn=lambda *a: None, notify=lambda e: None)

    def ups(self, prompt):
        return {'hook_event_name': 'UserPromptSubmit', 'session_id': 'S1', 'transcript_path': self.t, 'prompt': prompt, 'cwd': '/w'}

    def test_a_hold_is_written_down_once_and_without_the_message(self):
        self.assertEqual(self.hook(self.ups('fix the lights'))['decision'], 'block')
        rows = self.sb.events()
        self.assertEqual(len(rows), 1)
        r = rows[0]
        self.assertEqual((r['kind'], r['sid'], r['ctx'], r['at']), ('hold', 'S1', 640_000, self.now))
        self.assertEqual(r['idle'], 9 * 3600 - 5)
        self.assertNotIn('fix the lights', json.dumps(rows))  # what was typed stays out of the log
        self.assertIsNone(self.hook(self.ups('fix the lights')))  # sent again: let through, and not a second hold
        self.assertEqual(len(self.sb.events()), 1)

    def test_a_fresh_start_by_clear_is_written_down_with_what_the_summary_cost(self):
        self.hook(self.ups('fix the lights'))
        self.sb.handoff_path('S1').write_text('HANDOFF BODY')
        self.sb._jput(self.sb.paths()['handoffs'] / 'S1.cost.json', {'v': 1, 'weighted': 30_000, 'at': self.now})
        self.sb.handoff_is_fresh = lambda *a: True
        out = self.hook({'hook_event_name': 'SessionStart', 'source': 'clear', 'session_id': 'S2', 'transcript_path': self.t})
        self.assertIn('HANDOFF BODY', out['hookSpecificOutput']['additionalContext'])
        r = [x for x in self.sb.events() if x['kind'] == 'fresh'][0]
        self.assertEqual((r['sid'], r['new'], r['via'], r['ctx'], r['summary']), ('S1', 'S2', 'clear', 640_000, 30_000))
        self.assertTrue(r['cold'])
        # going back: 611k written again at 2.0 and 29k read at 0.1. Start fresh: the 30k summary and a 68k new chat.
        self.assertEqual(r['resume'], 1_224_900)
        self.assertEqual(r['saved'], 1_224_900 - 30_000 - r['new_chat'])
        self.assertFalse(r.get('summary_est'))


class FreshJobEventTest(JobBase):
    def setUp(self):
        super().setUp()
        self.transcript(SID_A, str(self.root / 'proj'), NOW - 9 * 3600, ctx=200_000)
        self.sb.rpc({'verb': 'list'}, NOW, agents=[])

    def handoff(self, weighted=None):
        def make(tp, sid):
            self.sb.handoff_path(sid).write_text('HANDOFF ' + sid)
            if weighted is not None:
                self.sb._jput(self.sb.paths()['handoffs'] / f'{sid}.cost.json', {'v': 1, 'weighted': weighted, 'at': NOW,
                                                                                 'tokens': {'input': 5}, 'usd': 0.3})
        return make

    def test_a_fresh_start_is_logged_with_the_summary_as_a_cost(self):
        jid = self.job({'verb': 'fresh', 'sid': SID_A})
        self.drain(make_handoff=self.handoff(40_000))
        res = self.sb.rpc({'verb': 'job', 'id': jid}, NOW + 6, agents=[])
        self.assertEqual(res['state'], 'done')
        self.assertEqual(res['summary_cost'], {'weighted': 40_000, 'usd': 0.3, 'tokens': {'input': 5}})  # kept with the job
        r = [x for x in self.sb.events() if x['kind'] == 'fresh'][0]
        self.assertEqual((r['sid'], r['via'], r['ctx'], r['summary']), (SID_A, 'app', 200_000, 40_000))
        self.assertEqual(r['resume'], 344_900)  # cold: 171k written again at 2.0, 29k read from the cache
        self.assertEqual(r['saved'], r['resume'] - 40_000 - r['new_chat'])
        self.assertGreater(r['saved'], 0)

    def test_an_uncounted_summary_is_estimated_and_marked_so(self):
        self.job({'verb': 'fresh', 'sid': SID_A})
        self.drain(make_handoff=self.handoff(None))
        r = [x for x in self.sb.events() if x['kind'] == 'fresh'][0]
        self.assertTrue(r['summary_est'])
        self.assertGreater(r['summary'], 0)
        self.assertEqual(r['saved'], r['resume'] - r['summary'] - r['new_chat'])

    def test_a_small_chat_started_fresh_shows_as_a_loss(self):
        self.transcript(SID_A, str(self.root / 'proj'), NOW - 9 * 3600, ctx=40_000)
        self.sb.rpc({'verb': 'list'}, NOW, agents=[])
        self.job({'verb': 'fresh', 'sid': SID_A})
        self.drain(make_handoff=self.handoff(20_000))
        r = [x for x in self.sb.events() if x['kind'] == 'fresh'][0]
        self.assertLess(r['saved'], 0)

    def test_a_launch_that_failed_is_not_a_fresh_start(self):
        self.job({'verb': 'fresh', 'sid': SID_A})
        self.drain(make_handoff=self.handoff(40_000), runner=self.runner(rc=1, out='boom'))
        self.assertEqual([x for x in self.sb.events() if x['kind'] == 'fresh'], [])


class StopEventTest(JobBase):
    AGENT = {'sessionId': SID_A, 'kind': 'background', 'status': 'busy', 'pid': 500, 'id': 'aaaaaaaa'}

    def test_a_stop_from_the_apps_is_logged(self):
        self.assertEqual(self.sb.rpc({'verb': 'stop', 'sid': SID_A}, NOW, agents=[dict(self.AGENT)]), {'ok': True})
        r = self.sb.events()[-1]
        self.assertEqual((r['kind'], r['sid'], r['by'], r['at']), ('stop', SID_A, 'user', NOW))

    def test_a_stop_of_nothing_running_is_not_logged(self):
        self.sb.rpc({'verb': 'stop', 'sid': SID_A}, NOW, agents=[])
        self.sb.rpc({'verb': 'dismiss', 'sid': SID_A}, NOW, agents=[])
        self.assertEqual(self.sb.events(), [])

    def test_a_budget_stop_is_logged_for_each_chat(self):
        out = self.sb.budget_stop([dict(self.AGENT)], run=lambda args: 'stopped')
        self.assertEqual(len(out['stopped']), 1)
        r = self.sb.events()[-1]
        self.assertEqual((r['kind'], r['sid'], r['by']), ('stop', SID_A, 'budget'))


if __name__ == '__main__':
    unittest.main()
