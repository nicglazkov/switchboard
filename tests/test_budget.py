"""The usage budget (spec 2026-10-13-usage-budget): settings, status, the snapshot field, what chats see, the hard limit."""
import json, os, sys, tempfile, time, unittest
from pathlib import Path
from tempfile import mkdtemp
from tests.helpers import fresh_sb, user, assistant, write_transcript
from tests.test_week_history import LA, MIN, HOUR, DAY, at
sys.path.insert(0, str(Path(__file__).resolve().parents[1] / 'server'))
import budget, fleet, server  # noqa: E402

RESET = at(2026, 10, 15, 9)            # Thursday 9 AM
NOW = at(2026, 10, 13, 15)             # Tuesday 3 PM
MACH = [{'name': 'desk', 'label': 'Desk', 'target': 'me@x', 'os': 'mac'},
        {'name': 'tower', 'label': 'Tower', 'target': 'me@y', 'os': 'windows'}]
SID1 = 'aaaaaaaa-1111-2222-3333-444444444444'


class Machines:
    def __init__(self):
        self.week, self.five, self.calls, self.down = 40.0, 4.0, [], set()
        self.features = ['budget_stop']

    def __call__(self, target, req, timeout):
        name = 'desk' if target == 'me@x' else 'tower'
        self.calls.append((name, req))
        if name in self.down:
            raise OSError('down')
        if req['verb'] == 'list':
            return {'ok': True, 'sessions': [{'sid': SID1, 'title': 'Build it', 'mtime': NOW, 'status': 'busy'}] if name == 'desk' else [],
                    'features': self.features,
                    'limits': {'five_hour': self.five, 'seven_day': self.week, 'seven_day_reset': RESET, 'updated': NOW - 10}}
        if req['verb'] == 'budget_stop':
            return {'ok': True, 'stopped': [{'sid': SID1, 'bg_id': 'abcd1234'}] if name == 'desk' else [], 'failed': []}
        return {'ok': True}


class Base(unittest.TestCase):
    def setUp(self):
        self.m, self.clock, self.pushes = Machines(), [NOW], []
        self.cfg, self.state = Path(mkdtemp()), Path(mkdtemp())
        self.f = fleet.Fleet(MACH, self.m, clock=lambda: self.clock[0])
        self.app = server.App(self.f, state_dir=self.state, web_dir=self.state, clock=lambda: self.clock[0], cfg_dir=self.cfg,
                              pusher=lambda *a: self.pushes.append(a))
        self.app._refresh_soon = lambda *a, **k: None
        self.f.history.tz = LA
        self.b = self.app.feat.budget

    def sample(self, t, pct):
        self.clock[0] = t
        self.m.week = pct
        self.f.poll()

    def post(self, **body):
        return self.app.post('/api/budget', body)


class SettingsTest(Base):
    def test_defaults(self):
        g = self.app.get('/api/budget')
        self.assertEqual(g['settings'], {'daily': 14.3, 'five_hour': None, 'hard': {'on': False, 'daily': 20.0, 'five_hour': None},
                                         'lifted_until': None})
        self.assertEqual(set(g), {'ok', 'settings', 'status', 'events'})

    def test_set_is_partial_and_saved(self):
        r = self.post(op='set', daily=10, hard={'on': True})
        self.assertEqual(r['settings']['daily'], 10.0)
        self.assertEqual(r['settings']['hard'], {'on': True, 'daily': 20.0, 'five_hour': None})
        self.post(op='set', five_hour=30.04, hard={'daily': None, 'five_hour': 50})
        s = json.loads((self.cfg / 'budget.json').read_text())
        self.assertEqual((s['daily'], s['five_hour'], s['hard']), (10.0, 30.0, {'on': True, 'daily': None, 'five_hour': 50.0}))
        self.post(op='set', daily=None)
        self.assertIsNone(self.app.get('/api/budget')['settings']['daily'])

    def test_bad_values_are_refused_whole(self):
        for body in ({'daily': 0}, {'daily': 101}, {'daily': True}, {'daily': '14'}, {'daily': float('nan')},
                     {'five_hour': -1}, {'hard': {'on': 1}}, {'hard': {'daily': 0}}, {'hard': {'x': 1}}, {'hard': True},
                     {'hard': {}}, {'lifted_until': 5}, {'weekly': 3}, {}):
            with self.assertRaises(server.HttpError, msg=body) as cm:
                self.post(op='set', **body)
            self.assertEqual(cm.exception.code, 400, body)
        self.assertEqual(self.app.get('/api/budget')['settings']['daily'], 14.3)
        for op in ('nope', None):
            with self.assertRaises(server.HttpError):
                self.post(op=op)
        with self.assertRaises(server.HttpError):
            self.post(op='lift', until=1)

    def test_a_broken_file_falls_back_field_by_field(self):
        (self.cfg / 'budget.json').write_text(json.dumps({'daily': 'x', 'five_hour': 25, 'hard': {'on': 'yes', 'daily': 30}}))
        s = self.b.settings()
        self.assertEqual((s['daily'], s['five_hour'], s['hard']), (14.3, 25.0, {'on': False, 'daily': 30.0, 'five_hour': None}))
        (self.cfg / 'budget.json').write_text('not json')
        self.assertEqual(self.b.settings()['daily'], 14.3)

    def test_lift_until_the_end_of_today_and_unlift(self):
        r = self.post(op='lift')
        self.assertEqual(r['settings']['lifted_until'], at(2026, 10, 14))
        self.assertTrue(r['status']['lifted'])
        self.assertEqual(r['events'][0]['kind'], 'lift')
        self.clock[0] = at(2026, 10, 14, 0, 1)
        self.assertIsNone(self.app.get('/api/budget')['settings']['lifted_until'], 'over at midnight')
        self.assertFalse(self.app.get('/api/budget')['status']['lifted'])
        self.clock[0] = NOW
        self.post(op='lift')
        self.assertIsNone(self.post(op='unlift')['settings']['lifted_until'])


class StatusMathTest(Base):
    def test_today_is_the_week_now_minus_the_week_at_midnight(self):
        self.sample(at(2026, 10, 12, 23), 21)
        self.sample(at(2026, 10, 13, 1), 23)         # 2 points over 2 hours: 22 at midnight
        self.sample(NOW, 40)
        st = self.app.get('/api/budget')['status']
        self.assertEqual(st['today'], 18.0)
        self.assertEqual(st['day'], '2026-10-13')
        self.assertTrue(st['over_daily'] and st['over_budget'])
        self.assertFalse(st['over_hard'])
        self.assertEqual(st['week'], 40.0)
        self.assertEqual(st['projected'], fleet._projected(40, RESET, NOW))
        self.assertEqual(st['five_hour'], 4.0)
        self.assertEqual(st['summary'], f'Today: 18% of the week (budget 14%, over). Week: 40%, on pace for {st["projected"]}%. '
                                        '5 hours: 4%. Hard limit: off.')
        self.assertIn('over their daily usage budget (today 18% of the week, budget 14%; week on pace for', st['notice'])
        self.assertTrue(st['notice'].endswith('Mention this to them once, briefly, then carry on.'))
        self.assertIsNone(st['block'])

    def test_days_follow_switchboard_tz_not_utc(self):
        # 4 PM in LA is 11 PM UTC; the LA day started at 07:00 UTC, when the week was at 30
        self.sample(at(2026, 10, 12, 23, 45), 29)
        self.sample(at(2026, 10, 13, 0, 15), 30)
        self.sample(at(2026, 10, 13, 16), 33)
        self.assertAlmostEqual(self.b.status()['today'], 3.5, delta=0.1)
        self.sample(at(2026, 10, 14, 0, 30), 34)     # just past the next local midnight
        st = self.b.status()
        self.assertEqual(st['day'], '2026-10-14')
        self.assertLess(st['today'], 1.0)
        self.assertFalse(st['over_budget'])

    def test_the_week_starting_today_counts_from_the_reset(self):
        reset = at(2026, 10, 20, 9)                  # the week began at 9 AM today
        days = fleet.today_used([(at(2026, 10, 13, 10), 1.0)], 3.0, reset, at(2026, 10, 13, 12), LA)
        self.assertEqual(days, 3.0)

    def test_a_gap_across_midnight_uses_the_first_sample_of_the_day(self):
        s = [(at(2026, 10, 12, 18), 10.0), (at(2026, 10, 13, 6), 20.0)]   # 12 hours and 10 points: cannot tell midnight
        self.assertEqual(fleet.today_used(s, 25.0, RESET, NOW, LA), 5.0)
        self.assertIsNone(fleet.today_used([], None, RESET, NOW, LA))
        self.assertIsNone(fleet.today_used([], 5.0, NOW - 1, NOW, LA), 'numbers from a week that is over')

    def test_nothing_known(self):
        self.m.down = {'desk', 'tower'}
        self.f.poll()
        st = self.b.status()
        self.assertFalse(st['known'] or st['over_budget'] or st['blocking'])
        self.assertIn('cannot tell', st['summary'])

    def test_five_hour_cap(self):
        self.post(op='set', daily=None, five_hour=3)
        self.sample(NOW, 40)
        st = self.b.status()
        self.assertTrue(st['over_five_hour'] and st['over_budget'])
        self.assertFalse(st['over_daily'])
        self.assertIn('over their 5-hour usage cap (5-hour window at 4%, cap 3%', st['notice'])
        self.assertTrue(st['summary'].startswith('Today: not known yet.'), 'one sample: the day start cannot be told')

    def test_close_numbers_keep_a_decimal(self):
        self.assertEqual(budget.fmt_pair(14.4, 14.3), ('14.4%', '14.3%'))
        self.assertEqual(budget.fmt_pair(18.2, 14.3), ('18%', '14%'))
        self.assertEqual(budget.fmt(0.4), '0.4%')


class SnapshotTest(Base):
    def test_sessions_carry_budget_and_polls_carry_it_to_agents(self):
        self.sample(NOW, 40)
        snap = self.app.sessions()
        self.assertEqual(set(snap['budget']), {'settings', 'status'})
        self.assertEqual(snap['budget']['settings']['daily'], 14.3)
        self.assertIn('over_budget', snap['budget']['status'])
        json.dumps(snap)
        lists = [r for n, r in self.m.calls if r['verb'] == 'list']
        self.assertEqual(set(lists[-1]['budget']), {'known', 'day', 'over_budget', 'blocking', 'summary', 'notice', 'block'})


class HardLimitTest(Base):
    def over(self):
        self.post(op='set', hard={'on': True, 'daily': 20})
        self.sample(at(2026, 10, 13, 0, 5), 18)
        self.sample(NOW, 40)                         # 22 today

    def test_block_text_and_stop_fan_out_once(self):
        self.over()
        st = self.b.status()
        self.assertTrue(st['over_hard'] and st['blocking'])
        self.assertEqual(st['block'], 'Switchboard hard limit: today is at 22% of the week (limit 20%). Lift it for today '
                                      'in Switchboard Settings, or wait until tomorrow.')
        r = self.b.after_poll()
        self.assertEqual(r['stopped'], [{'machine': 'desk', 'sid': SID1, 'title': 'Build it'}])
        self.assertEqual(sorted(n for n, q in self.m.calls if q['verb'] == 'budget_stop'), ['desk', 'tower'])
        self.assertEqual(self.pushes, [('Hard limit hit', 'Stopped 1 chat. Today at 22% of the week.', '/?usage=1')])
        ev = self.app.get('/api/budget')['events'][0]
        self.assertEqual((ev['kind'], ev['text']), ('hard', 'Hard limit hit: Stopped 1 chat. Today at 22% of the week.'))
        self.assertIsNone(self.b.after_poll(), 'once a day')
        self.assertEqual(len([1 for n, q in self.m.calls if q['verb'] == 'budget_stop']), 2)
        self.sample(at(2026, 10, 14, 0, 30), 41)     # the next day, over again: once more
        self.sample(at(2026, 10, 14, 12), 70)
        self.assertTrue(self.b.after_poll())
        self.assertEqual(len(self.pushes), 2)

    def test_a_lift_stops_nothing_that_day_even_after_unlift(self):
        self.over()
        self.post(op='lift')
        self.assertFalse(self.b.status()['blocking'])
        self.assertIsNone(self.b.after_poll())
        self.post(op='unlift')
        self.assertTrue(self.b.status()['blocking'])
        self.assertIsNone(self.b.after_poll(), 'never again after a lift that day')
        self.assertEqual([q for n, q in self.m.calls if q['verb'] == 'budget_stop'], [])
        self.assertEqual(self.pushes, [])

    def test_off_or_under_does_nothing(self):
        self.sample(NOW, 40)
        self.assertFalse(self.b.status()['blocking'], 'hard limit off')
        self.assertIsNone(self.b.after_poll())
        self.post(op='set', hard={'on': True, 'daily': 90})
        self.assertIsNone(self.b.after_poll())

    def test_older_agents_and_offline_machines_are_skipped(self):
        self.m.features = []
        self.over()
        r = self.b.after_poll()
        self.assertEqual(r['stopped'], [])
        self.assertEqual(self.pushes[0][1], 'No background chats were working. Today at 22% of the week.')


# ---------------------------------------------------------------- the agent

STATUS = {'known': True, 'day': '2026-10-13', 'over_budget': True, 'blocking': False, 'summary': 'Today: 18% of the week.',
          'notice': 'Switchboard: the user is over their daily usage budget.', 'block': None}


class AgentBudgetTest(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.mkdtemp()
        self.sb = fresh_sb(self.tmp)
        self.t = write_transcript(self.tmp, [user('2026-10-13T20:00:00Z', 'hi'), assistant('2026-10-13T20:00:05Z', 'ok')])
        self.now = time.time()

    def ups(self, prompt='fix it', sid='S1'):
        return {'hook_event_name': 'UserPromptSubmit', 'session_id': sid, 'transcript_path': self.t, 'prompt': prompt, 'cwd': '/w'}

    def hook(self, inp, now=None):
        return self.sb.handle_hook(inp, now or self.now, 777, spawn=lambda *a: None, notify=lambda e: None)

    def keep(self, st, now=None):
        self.sb.budget_put(st, now or self.now)

    def test_the_poll_keeps_the_status(self):
        r = self.sb.rpc({'verb': 'list', 'budget': dict(STATUS, extra='x', summary='y' * 2000)}, self.now, agents=[])
        self.assertTrue(r['ok'])
        kept = json.loads((self.sb.paths()['state'] / 'budget.json').read_text())
        self.assertEqual(kept['at'], self.now)
        self.assertNotIn('extra', kept['status'])
        self.assertEqual(len(kept['status']['summary']), self.sb.BUDGET_TEXT_MAX)
        self.assertIn('budget_stop', self.sb.AGENT_FEATURES)

    def test_soft_notice_once_per_chat_per_day(self):
        self.keep(STATUS)
        out = self.hook(self.ups())
        self.assertEqual(out['hookSpecificOutput']['additionalContext'], STATUS['notice'])
        self.assertIsNone(self.hook(self.ups('again')), 'once a day')
        self.assertIn('over', self.hook(self.ups(sid='S2'))['hookSpecificOutput']['additionalContext'], 'each chat once')
        self.keep(dict(STATUS, day='2026-10-14'))
        self.assertIsNotNone(self.hook(self.ups()), 'a new day: once more')

    def test_no_notice_when_under_or_unknown(self):
        self.keep(dict(STATUS, over_budget=False))
        self.assertIsNone(self.hook(self.ups()))
        self.keep(None)
        self.assertIsNone(self.hook(self.ups()))

    def test_injected_prompts_and_seeds_never_get_it(self):
        self.keep(dict(STATUS, blocking=True, block='Switchboard hard limit: ...'))
        self.assertIsNone(self.hook(self.ups('<task-notification>done</task-notification>')))

    def test_notice_joins_other_context(self):
        out = self.sb.budget_merge(self.sb._ctx_out('UserPromptSubmit', 'GO'), 'S1', self.sb.budget_clean(STATUS))
        self.assertEqual(out['hookSpecificOutput']['additionalContext'], 'GO\n\n' + STATUS['notice'])
        blk = {'decision': 'block', 'reason': 'held'}
        self.assertEqual(self.sb.budget_merge(dict(blk), 'S2', self.sb.budget_clean(STATUS)), blk)
        self.assertIsNotNone(self.sb.budget_notice_due('S2', self.sb.budget_clean(STATUS)), 'not spent on a block')

    def test_hard_limit_blocks_typed_prompts(self):
        text = ('Switchboard hard limit: today is at 22% of the week (limit 20%). Lift it for today in Switchboard Settings, '
                'or wait until tomorrow.')
        self.keep(dict(STATUS, blocking=True, block=text))
        out = self.hook(self.ups())
        self.assertEqual(out, {'decision': 'block', 'reason': text})
        self.assertFalse(self.sb.hook_ping(self.ups(), out, self.now, ping=lambda e: None))
        os.environ['SB_INTERNAL'] = '1'
        try:
            self.assertIsNone(self.hook(self.ups()), "Switchboard's own runs go through")
        finally:
            os.environ.pop('SB_INTERNAL')

    def test_stale_cache_asks_the_server_quickly_then_remembers(self):
        self.keep(STATUS, now=self.now - 120)
        asked = []
        st = self.sb.budget_status(self.now, fetch=lambda: asked.append(1) or dict(STATUS, over_budget=False))
        self.assertFalse(st['over_budget'])
        self.assertEqual(asked, [1])
        self.sb.budget_status(self.now + 5, fetch=lambda: asked.append(2))
        self.assertEqual(asked, [1], 'kept for a minute')

    def test_missing_cache_and_no_server_adds_nothing_fast(self):
        t = time.time()
        self.assertIsNone(self.sb.budget_status(self.now))
        self.assertLess(time.time() - t, 0.2)
        self.assertIsNone(self.hook(self.ups()))
        slow = lambda base: time.sleep(2)
        t = time.time()
        self.assertIsNone(self.sb.budget_fetch(servers=['http://a', 'http://b'], get=slow))
        self.assertLess(time.time() - t, 0.6, 'never holds the hook past ~300 ms')
        self.assertEqual(self.sb.budget_fetch(servers=['http://a'], get=lambda b: {'status': STATUS})['day'], '2026-10-13')

    def test_budget_stop_stops_busy_background_chats_only(self):
        ran = []
        ags = [{'kind': 'background', 'status': 'busy', 'id': 'abcd1234', 'sessionId': SID1},
               {'kind': 'background', 'status': 'idle', 'id': 'bbbb1234', 'sessionId': SID1},
               {'kind': 'interactive', 'status': 'busy', 'id': 'cccc1234'},
               {'kind': 'background', 'status': 'busy', 'id': '../x'}]
        r = self.sb.budget_stop(ags, run=ran.append)
        self.assertEqual(ran, [['stop', 'abcd1234']])
        self.assertEqual(r, {'ok': True, 'stopped': [{'sid': SID1, 'bg_id': 'abcd1234'}], 'failed': []})
        calls = []
        self.sb.run_claude_cmd = lambda args, **kw: calls.append(args) or ''
        self.assertTrue(self.sb.rpc({'verb': 'budget_stop'}, self.now, agents=ags[:1])['ok'])
        self.assertEqual(calls, [['stop', 'abcd1234']])

    def test_usage_tool(self):
        out, bad = self.sb.mcp_tool('usage', {}, call=lambda m, p, b=None: {'status': {'summary': 'Today: 18% of the week.'}})
        self.assertEqual((out, bad), ('Today: 18% of the week.', False))

        def down(*a, **k):
            raise self.sb.RpcError('Switchboard is not reachable from here (URLError).')
        out, bad = self.sb.mcp_tool('usage', {}, call=down)
        self.assertFalse(bad)
        self.assertIn('cannot tell', out)
        r = self.sb.mcp_handle({'jsonrpc': '2.0', 'id': 1, 'method': 'tools/call', 'params': {'name': 'usage', 'arguments': {}}},
                               tool=lambda n, a: ('Today: 18% of the week.', False))
        self.assertEqual(r['result']['content'], [{'type': 'text', 'text': 'Today: 18% of the week.'}])
