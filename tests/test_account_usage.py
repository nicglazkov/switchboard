"""The account's own usage report: what Claude Code answers to get_usage, as the agent keeps it, the server serves it
(GET /api/usage `account`, POST /api/usage/refresh) and an older agent or server lives without it."""
import copy, datetime, json, subprocess, sys, tempfile, unittest
from pathlib import Path
from tests.helpers import fresh_sb
sys.path.insert(0, str(Path(__file__).resolve().parents[1] / 'server'))
import fleet  # noqa: E402

SAMPLE = json.loads((Path(__file__).parent / 'fixtures' / 'get_usage_sample.json').read_text())


def ts(iso):
    return datetime.datetime.fromisoformat(iso).timestamp()


NOW = ts('2033-05-18T04:00:00+00:00')
FIVE_RESET = ts('2033-05-18T07:00:00+00:00')
WEEK_RESET = ts('2033-05-21T21:00:00+00:00')
SID = 'aaaaaaaa-1111-2222-3333-444444444444'


class AgentBase(unittest.TestCase):
    def setUp(self):
        self.sb = fresh_sb(Path(tempfile.mkdtemp()).resolve())
        self.sb.paths()['state'].mkdir(parents=True, exist_ok=True)
        self.spawned = []
        self.sb.spawn_account_refresh = lambda now: self.spawned.append(now) or True


class ReportTest(AgentBase):
    def test_the_answer_becomes_limits_extra_usage_and_products(self):
        r = self.sb.account_report(SAMPLE, NOW)
        self.assertEqual((r['plan'], r['available'], r['at']), ('max', True, NOW))
        self.assertEqual([(x['kind'], x['pct'], x['reset'], x['label'], x['active']) for x in r['limits']],
                         [('session', 41, FIVE_RESET, None, False), ('weekly_all', 62, WEEK_RESET, None, True),
                          ('weekly_scoped', 18, WEEK_RESET, 'Fable', False)])
        self.assertEqual(r['limits'][1]['severity'], 'warning')
        e = r['extra']
        # Amounts stay in minor units with the exponent the source gives: 7500 with exponent 2 is 75.00.
        self.assertEqual(e['used'], {'minor': 1234, 'currency': 'USD', 'exp': 2})
        self.assertEqual(e['limit'], {'minor': 7500, 'currency': 'USD', 'exp': 2})
        self.assertEqual(e['balance'], {'minor': 2500, 'currency': 'USD', 'exp': 2})
        self.assertEqual((e['enabled'], e['pct'], e['reason'], e['limit_reached'], e['auto_reload']), (True, 16, None, False, None))
        p = r['products']
        self.assertEqual([(x['key'], x['name'], x['pct'], x['of_limit']) for x in p['rows']],
                         [('claude_code', 'Claude Code', 81, 50), ('chat', 'Chats', 12, 8), ('cowork', 'Cowork', 4, 2), ('other', 'Other', 3, 2)])
        self.assertEqual(p['start'], WEEK_RESET - 7 * 86400)
        json.dumps(r)  # goes over the wire as it is

    def test_a_claude_code_from_before_the_rows_is_read_from_its_named_windows(self):
        old = copy.deepcopy(SAMPLE)
        rl = old['rate_limits']
        for k in ('limits', 'spend', 'seven_day_breakdown'):
            del rl[k]
        rl['seven_day_opus'] = {'utilization': 30, 'resets_at': '2033-05-21T21:00:00Z'}
        r = self.sb.account_report(old, NOW)
        self.assertEqual([(x['kind'], x['label'], x['pct']) for x in r['limits']],
                         [('session', None, 41), ('weekly_all', None, 62), ('weekly_scoped', 'Opus', 30), ('weekly_scoped', 'Fable', 18)])
        self.assertEqual(r['limits'][2]['reset'], WEEK_RESET)
        self.assertEqual(r['extra']['used'], {'minor': 1234, 'currency': 'USD', 'exp': 2})
        self.assertEqual(r['extra']['limit']['minor'], 7500)
        self.assertEqual((r['extra']['enabled'], r['extra']['pct'], r['extra']['balance']), (True, 16, None))
        self.assertIsNone(r['products'])

    def test_a_plan_with_no_5_hour_window_has_no_session_row(self):
        d = copy.deepcopy(SAMPLE)
        d['rate_limits']['five_hour'] = None
        d['rate_limits']['limits'] = d['rate_limits']['limits'][1:]
        r = self.sb.account_report(d, NOW)
        self.assertEqual([x['kind'] for x in r['limits']], ['weekly_all', 'weekly_scoped'])
        lim = self.sb.account_limits(r)
        self.assertEqual((lim['five_hour'], lim['five_hour_reset'], lim['seven_day'], lim['seven_day_reset'], lim['updated']),
                         (None, None, 62, WEEK_RESET, NOW))

    def test_odd_answers_lose_a_section_and_never_raise(self):
        self.assertIsNone(self.sb.account_report(None, NOW))
        self.assertIsNone(self.sb.account_report([1], NOW))
        api_key = self.sb.account_report({'subscription_type': None, 'rate_limits_available': False, 'rate_limits': None}, NOW)
        self.assertEqual((api_key['available'], api_key['plan'], api_key['limits']), (False, None, []))
        self.assertIsNone(self.sb.account_limits(api_key))
        odd = {'subscription_type': 7, 'rate_limits': {
            'limits': [5, {'kind': 'weekly_all', 'percent': 'x'}, {'kind': 'weekly_all', 'percent': 250, 'resets_at': 'soon'},
                       {'kind': '', 'percent': 3}, {'kind': 'session', 'percent': -4, 'resets_at': FIVE_RESET, 'scope': {'model': 'x'}},
                       {'kind': 'weekly_scoped', 'percent': float('nan')}],
            'extra_usage': {'is_enabled': 'yes', 'monthly_limit': '75', 'used_credits': True, 'decimal_places': 9},
            'spend': {'used': {'amount_minor': None}, 'limit': 'x', 'percent': float('inf'), 'auto_reload': {'enabled': False}},
            'seven_day_breakdown': {'rows': [None, {'key': 'x'}, {'display_name': 'Chats', 'percent': 130}]}}}
        r = self.sb.account_report(odd, NOW)
        self.assertIsNone(r['plan'])
        self.assertEqual([(x['kind'], x['pct'], x['reset']) for x in r['limits']], [('weekly_all', 100, None), ('session', 0, FIVE_RESET)])
        self.assertEqual(r['extra'], {'enabled': None, 'used': None, 'limit': None, 'balance': None, 'pct': None, 'reason': None,
                                      'limit_reached': False, 'auto_reload': False})
        self.assertEqual(r['products']['rows'], [{'key': '', 'name': 'Chats', 'pct': 100, 'of_limit': None}])
        self.assertIsNone(self.sb.account_limits(r)['seven_day'], 'a week with no reset time is not a sighting')

    def test_reset_times_read_to_the_minute(self):
        t = self.sb._acct_time
        self.assertEqual(t('2033-05-21T20:59:59.617172+00:00'), WEEK_RESET)
        self.assertEqual(t('2033-05-21T21:00:00Z'), WEEK_RESET)
        self.assertEqual(t('2033-05-21T14:00:00-07:00'), WEEK_RESET)
        self.assertEqual(t(WEEK_RESET + 0.4), WEEK_RESET)
        for bad in (None, '', 'tomorrow', True, -5, float('nan'), {}):
            self.assertIsNone(t(bad), bad)


class CacheTest(AgentBase):
    def test_a_report_is_kept_and_goes_out_with_list(self):
        self.assertIsNone(self.sb.account_view())
        self.assertIsNone(self.sb.rpc({'verb': 'list'}, NOW, agents=[])['account'])
        self.sb.refresh_account(NOW, lambda: SAMPLE)
        r = self.sb.rpc({'verb': 'list'}, NOW + 5, agents=[])
        self.assertEqual((r['account']['plan'], r['account']['at'], r['account']['error']), ('max', NOW, None))
        self.assertIn('account_usage', r['features'])
        # Its week and 5 hours count as a sighting of the plan numbers, beside the status lines'.
        self.assertEqual((r['limits']['seven_day'], r['limits']['seven_day_reset'], r['limits']['five_hour'], r['limits']['updated']),
                         (62, WEEK_RESET, 41, NOW))
        self.assertEqual(self.spawned, [], 'nobody asked for a new one')

    def test_a_status_line_that_saw_more_still_wins_the_week(self):
        self.sb.refresh_account(NOW, lambda: SAMPLE)
        self.sb._jput(self.sb.paths()['state'] / f'{SID}.json',
                      {'seven_day': 64, 'seven_day_reset': WEEK_RESET, 'five_hour': 12, 'five_hour_reset': FIVE_RESET,
                       'limits_at': NOW + 60, 'updated': NOW + 60})
        lim = self.sb.latest_limits(NOW + 90)
        self.assertEqual((lim['seven_day'], lim['five_hour'], lim['updated']), (64, 41, NOW + 60))

    def test_a_failed_try_keeps_the_last_good_report(self):
        self.sb.refresh_account(NOW, lambda: SAMPLE)

        def boom():
            raise RuntimeError('get_usage is not supported in this context')
        self.sb.refresh_account(NOW + 100, boom)
        v = self.sb.account_view()
        self.assertEqual((v['plan'], v['at'], v['tried'], v['error']), ('max', NOW, NOW + 100, 'get_usage is not supported in this context'))

        def slow():
            raise subprocess.TimeoutExpired('claude', 25)
        self.sb.refresh_account(NOW + 200, slow)
        self.assertEqual(self.sb.account_view()['error'], 'Claude Code took too long to answer')
        self.sb.refresh_account(NOW + 300, lambda: 'nonsense')
        self.assertEqual(self.sb.account_view()['error'], 'the answer could not be read')

    def test_a_first_try_that_fails_says_so_with_no_report(self):
        self.sb.refresh_account(NOW, lambda: (_ for _ in ()).throw(OSError('no claude here')))
        v = self.sb.account_view()
        self.assertEqual((v['at'], v['error'], v['tried']), (None, 'no claude here', NOW))
        self.assertIsNone(self.sb.latest_limits(NOW))

    def test_list_starts_a_report_only_when_one_is_due(self):
        ask = lambda t, age: self.sb.rpc({'verb': 'list', 'usage': {'max_age': age}}, t, agents=[])
        ask(NOW, 900)
        self.assertEqual(self.spawned, [NOW], 'none yet: asked for')
        self.sb.refresh_account(NOW, lambda: SAMPLE)
        ask(NOW + 600, 900)
        self.assertEqual(len(self.spawned), 1, 'ten minutes old, fifteen allowed')
        ask(NOW + 901, 900)
        self.assertEqual(len(self.spawned), 2)
        ask(NOW + 5, 0)
        ask(NOW + 5, 'now')
        self.assertEqual(len(self.spawned), 2, 'never more often than every 20 seconds, whatever is asked')
        self.sb.rpc({'verb': 'list', 'usage': 'yes'}, NOW + 5000, agents=[])
        self.assertEqual(len(self.spawned), 2, 'not a request')

    def test_after_a_failure_the_poll_waits_half_an_hour_and_refresh_does_not(self):
        self.sb.refresh_account(NOW, lambda: (_ for _ in ()).throw(OSError('offline')))
        self.assertFalse(self.sb.account_due(NOW + 1000, 900))
        self.assertTrue(self.sb.account_due(NOW + 1801, 900))
        self.assertTrue(self.sb.account_due(NOW + 30, 20), 'Refresh may try again')
        self.assertFalse(self.sb.account_due(NOW + 10, 20))

    def test_the_verb_asks_now_and_answers_with_the_report(self):
        asked = []

        def asker():
            asked.append(1)
            return SAMPLE
        r = self.sb.account_now(NOW, 60, asker)
        self.assertEqual((r['ok'], r['account']['plan'], r['limits']['seven_day']), (True, 'max', 62))
        self.sb.account_now(NOW + 30, 60, asker)
        self.assertEqual(len(asked), 1, 'half a minute old is fresh enough')
        self.sb.account_now(NOW + 61, 60, asker)
        self.assertEqual(len(asked), 2)
        self.assertFalse(self.sb._account_lock().exists(), 'the slot is free again')

    def test_the_verb_waits_for_a_report_already_on_its_way(self):
        self.assertTrue(self.sb.lock_take(self.sb._account_lock(), NOW, 60, mine=False))
        r = self.sb.account_now(NOW, 60, lambda: self.fail('asked twice'), wait=0.3)
        self.assertEqual((r['ok'], r['account']), (True, None))

    def test_claude_codes_reply_is_read_from_its_control_response(self):
        seen = {}

        def run(argv, stdin, timeout, **kw):
            seen.update(argv=argv, stdin=json.loads(stdin), env=kw['env'])
            out = '\n'.join(['not json', json.dumps({'type': 'system', 'subtype': 'init'}), json.dumps(
                {'type': 'control_response', 'response': {'subtype': 'success', 'request_id': 'sb-usage', 'response': SAMPLE}})])
            return subprocess.CompletedProcess(argv, 0, out, '')
        self.sb.run_tree = run
        self.assertEqual(self.sb.ask_account_usage()['subscription_type'], 'max')
        self.assertEqual(seen['stdin']['request'], {'subtype': 'get_usage', 'skip_behaviors': True})
        self.assertIn('--no-session-persistence', seen['argv'])
        self.assertEqual(seen['env'].get('SB_INTERNAL'), '1', 'its hooks stay out of it')
        self.assertNotIn('ANTHROPIC_API_KEY', seen['env'])
        self.sb.run_tree = lambda argv, *a, **kw: subprocess.CompletedProcess(argv, 0, json.dumps(
            {'type': 'control_response', 'response': {'subtype': 'error', 'error': 'Unsupported control request subtype: get_usage'}}), '')
        with self.assertRaisesRegex(RuntimeError, 'Unsupported control request'):
            self.sb.ask_account_usage()
        self.sb.run_tree = lambda argv, *a, **kw: subprocess.CompletedProcess(argv, 1, '', 'error: unknown option')
        with self.assertRaisesRegex(RuntimeError, 'did not answer'):
            self.sb.ask_account_usage()


def report(sb_at, **kw):
    """A report as an agent sends it."""
    d = {'v': 1, 'at': sb_at, 'tried': sb_at, 'error': None, 'plan': 'max', 'available': True,
         'limits': [{'kind': 'weekly_all', 'pct': 62, 'reset': WEEK_RESET, 'label': None, 'severity': 'normal', 'active': True}],
         'extra': {'enabled': True, 'used': {'minor': 1234, 'currency': 'USD', 'exp': 2}, 'limit': {'minor': 7500, 'currency': 'USD', 'exp': 2},
                   'balance': None, 'pct': 16, 'reason': None, 'limit_reached': False, 'auto_reload': None},
         'products': {'as_of': NOW, 'start': WEEK_RESET - 7 * 86400, 'rows': [{'key': 'claude_code', 'name': 'Claude Code', 'pct': 81, 'of_limit': 50}]}}
    d.update(kw)
    return d


MACH = [{'name': 'desk', 'label': 'Desk', 'target': 'me@x', 'os': 'mac'},
        {'name': 'laptop', 'label': 'Laptop', 'target': 'me@m', 'os': 'mac'},
        {'name': 'tower', 'label': 'Tower', 'target': 'me@y', 'os': 'windows'}]


class Machines:
    """Three machines: desk and laptop run an agent that can report, tower an older one."""
    def __init__(self):
        self.reqs, self.down, self.account, self.clock_off = [], set(), {}, {}
        self.new = {'desk', 'laptop'}
        self.seen = NOW - 100

    def __call__(self, target, req, timeout):
        name = {'me@x': 'desk', 'me@m': 'laptop', 'me@y': 'tower'}[target]
        self.reqs.append((name, req))
        if name in self.down:
            raise OSError('ssh: connect timed out')
        lim = {'seven_day': 60, 'seven_day_reset': WEEK_RESET, 'five_hour': None, 'five_hour_reset': None, 'updated': self.seen, 'merged': True}
        if req['verb'] == 'account_usage':
            self.account[name] = report(NOW + 500)
            return {'ok': True, 'account': self.account[name], 'limits': dict(lim, seven_day=62)}
        r = {'ok': True, 'sessions': [], 'limits': lim, 'now': NOW + self.clock_off.get(name, 0)}
        if name in self.new:
            r.update(features=['account_usage'], account=self.account.get(name))
        return r


class ServerTest(unittest.TestCase):
    def setUp(self):
        self.clock = [NOW]
        self.m = Machines()
        self.f = fleet.Fleet(MACH, self.m, clock=lambda: self.clock[0])

    def asked(self):
        out = {n: r.get('usage') for n, r in self.m.reqs if r['verb'] == 'list'}
        self.m.reqs.clear()
        return out

    def test_one_machine_is_asked_once_the_server_knows_who_can(self):
        self.f.poll()
        self.assertEqual(self.asked(), {'desk': None, 'laptop': None, 'tower': None}, 'nobody known to be able yet')
        self.f.poll()
        self.assertEqual(self.asked(), {'desk': {'max_age': fleet.ACCOUNT_EVERY}, 'laptop': None, 'tower': None})
        # A report newer than the last number any status line saw: nothing moved, so an hour will do.
        self.m.account['desk'] = report(NOW)
        self.f.poll()
        self.f.poll()
        self.assertEqual(self.asked()['desk'], {'max_age': fleet.ACCOUNT_IDLE})
        self.m.seen = NOW + 50  # a chat replied since
        self.f.poll()
        self.f.poll()
        self.assertEqual(self.asked()['desk'], {'max_age': fleet.ACCOUNT_EVERY})

    def test_the_next_machine_takes_over_when_the_asker_is_away_or_failing(self):
        self.f.poll()
        self.m.down.add('desk')
        self.f.poll()
        self.f.poll()
        self.assertEqual(self.asked()['laptop'], {'max_age': fleet.ACCOUNT_EVERY})
        self.m.down.clear()
        self.m.account['desk'] = report(None, error='Claude Code did not answer (exit 1)', tried=NOW)
        self.f.poll()
        self.f.poll()
        a = self.asked()
        self.assertEqual((a['desk'], a['laptop']), (None, {'max_age': fleet.ACCOUNT_EVERY}))
        self.m.account['laptop'] = report(None, error='offline', tried=NOW)
        self.f.poll()
        self.f.poll()
        self.assertEqual(self.asked()['desk'], {'max_age': fleet.ACCOUNT_EVERY}, 'all failing: the first keeps trying')

    def test_usage_serves_the_newest_report_on_the_servers_clock(self):
        self.m.clock_off = {'laptop': -300}  # its clock runs five minutes behind
        self.m.account = {'desk': report(NOW - 400), 'laptop': report(NOW - 500, plan='pro')}
        self.f.poll()
        u = self.f.usage()
        self.assertEqual((u['account']['plan'], u['account']['machine'], u['account']['at']), ('pro', 'Laptop', NOW - 200))
        self.assertEqual(u['account_support'], {'able': 2, 'online': 2, 'machines': 3})
        self.assertEqual(u['updated'], NOW + 200, "the laptop's sighting, on the server's clock")
        self.assertEqual(u['account']['extra']['limit'], {'minor': 7500, 'currency': 'USD', 'exp': 2})
        self.assertEqual(u['week']['pct'], 60)
        json.dumps(u)

    def test_with_only_older_agents_there_is_no_report_and_the_rest_still_answers(self):
        self.m.new = set()
        self.f.poll()
        self.f.poll()
        self.assertEqual(set(self.asked().values()), {None})
        u = self.f.usage()
        self.assertIsNone(u['account'])
        self.assertEqual(u['account_support'], {'able': 0, 'online': 0, 'machines': 3})
        self.assertEqual(u['week']['pct'], 60)
        self.assertFalse(self.f.refresh_account())

    def test_a_first_failure_is_served_so_the_apps_can_say_why(self):
        self.m.account = {'desk': report(None, error='Claude Code did not answer (exit 1)', tried=NOW - 5, limits=[], extra=None, products=None)}
        self.f.poll()
        a = self.f.usage()['account']
        self.assertEqual((a['at'], a['error'], a['machine'], a['tried']), (None, 'Claude Code did not answer (exit 1)', 'Desk', NOW - 5))

    def test_refresh_asks_the_asker_now_and_keeps_its_answer(self):
        self.f.poll()
        self.m.reqs.clear()
        self.assertTrue(self.f.refresh_account())
        self.assertEqual(self.m.reqs, [('desk', {'verb': 'account_usage', 'max_age': fleet.ACCOUNT_FRESH, 'as': 'desk'})])
        u = self.f.usage()
        self.assertEqual((u['account']['at'], u['week']['pct']), (NOW + 500, 62))
        self.m.down.add('desk')
        self.assertFalse(self.f.refresh_account(), 'unreachable: the last report stands')
        self.assertEqual(self.f.usage()['account']['at'], NOW + 500)

    def test_a_machines_odd_report_never_reaches_the_apps(self):
        c = fleet.clean_account
        for bad in (None, 5, [], {}, {'at': 'x'}, {'plan': 'max'}):
            self.assertIsNone(c(bad), bad)
        odd = c({'at': NOW, 'plan': 'x' * 99, 'limits': [1, {'kind': 'weekly_all', 'pct': 300, 'reset': 'x', 'label': 5, 'active': 1},
                                                             {'kind': None, 'pct': 3}, {'kind': 'session', 'pct': None}],
                 'extra': {'enabled': 'on', 'used': {'minor': 'x'}, 'limit': {'minor': 500, 'exp': 12}, 'pct': -3},
                 'products': {'rows': [{'name': 'Chats', 'pct': 'x'}, 7]}, 'error': 3})
        self.assertEqual(len(odd['plan']), 32)
        self.assertEqual(odd['limits'], [{'kind': 'weekly_all', 'pct': 100, 'reset': None, 'label': None, 'severity': None, 'active': False}])
        self.assertEqual(odd['extra'], {'enabled': None, 'used': None, 'limit': {'minor': 500, 'currency': None, 'exp': 2}, 'balance': None,
                                        'pct': 0, 'reason': None, 'limit_reached': False, 'auto_reload': None})
        self.assertIsNone(odd['products'])
        self.assertIsNone(odd['error'])
        self.assertEqual(c(report(NOW))['products']['rows'][0]['name'], 'Claude Code')


if __name__ == '__main__':
    unittest.main()
