"""GET /api/usage (this week by day, past weeks) and what a blocked chat needs from you (spec 2026-10-13, 1 and 4)."""
import json, sys, unittest
from pathlib import Path
from tempfile import mkdtemp
from tests.test_rpc import RpcBase, NOW as RPC_NOW, SID_A
from tests.test_week_history import LA, MIN, HOUR, DAY, at, RESET, START, NOW, MACH, Machines, steady
sys.path.insert(0, str(Path(__file__).resolve().parents[1] / 'server'))
import fleet, server  # noqa: E402


class DaysUsedTest(unittest.TestCase):
    def test_each_local_day_is_the_percent_at_its_end_minus_the_percent_at_its_start(self):
        samples = steady([8, 11, 7, 5 / (NOW - at(2026, 10, 11)) * DAY], NOW)
        week = samples[-1][1]
        days = fleet.days_used(samples[:-1], week, RESET, NOW, LA)
        self.assertEqual([d['day'] for d in days], ['2026-10-08', '2026-10-09', '2026-10-10', '2026-10-11'])
        # Thursday runs from the 9 AM reset to midnight, the others midnight to midnight: together, the whole week.
        self.assertAlmostEqual(sum(d['used'] for d in days), week, delta=0.15)
        self.assertEqual(days[-1]['used'], fleet.today_used(samples[:-1], week, RESET, NOW, LA), 'today is the budget\'s number')
        self.assertEqual(days[-1]['used'], fleet.week_daily(samples[:-1], week, RESET, NOW, LA)[-1])

    def test_use_between_one_days_last_sample_and_the_nexts_first_is_not_lost(self):
        samples = [(START + HOUR, 2), (at(2026, 10, 8, 23), 6), (at(2026, 10, 9, 1), 8),      # 1 point each side of midnight
                   (at(2026, 10, 9, 23, 30), 16), (at(2026, 10, 10, 0, 30), 15)]             # went down: never negative
        days = fleet.days_used(samples, 15, RESET, at(2026, 10, 10, 0, 45), LA)
        self.assertEqual([d['used'] for d in days], [7.0, 8.5, 0.0])

    def test_just_after_midnight_today_already_has_a_number(self):
        samples = [(START + HOUR, 2), (at(2026, 10, 8, 23, 50), 6)]
        days = fleet.days_used(samples, 6, RESET, at(2026, 10, 9, 0, 5), LA)
        self.assertEqual([d['used'] for d in days], [6.0, 0.0])

    def test_a_day_the_samples_cannot_account_for_is_unknown(self):
        # The server was off overnight while usage grew: 20% at 6 PM, 32% at 9 AM, 33% now. Nobody can say how much
        # of the 12 points fell before midnight; today counts from the first sample of the day, like the budget.
        samples = [(START + HOUR, 1), (at(2026, 10, 8, 16), 19), (at(2026, 10, 8, 18), 20), (at(2026, 10, 9, 9), 32)]
        days = fleet.days_used(samples, 33, RESET, at(2026, 10, 9, 10), LA)
        self.assertEqual(days, [{'day': '2026-10-08', 'used': None}, {'day': '2026-10-09', 'used': 1.0}])
        self.assertEqual(fleet.days_used([], None, RESET, NOW, LA), [])
        fresh = fleet.days_used([(at(2026, 10, 10, 12), 9), (at(2026, 10, 10, 13), 12)], 12, RESET, at(2026, 10, 10, 14), LA)
        self.assertEqual([d['used'] for d in fresh], [None, None, 3.0], 'a new install does not make up the days before it')

    def test_a_reset_starts_the_days_again(self):
        h = fleet.WeekHistory(Path(mkdtemp()) / 'week_history.json', tz=LA)
        h.record(40, RESET, RESET - 2 * HOUR)
        h.record(45, RESET, RESET - HOUR)                  # Thu Oct 15, 8 AM: the last hour of the week
        nxt = RESET + fleet.WEEK
        h.record(1, nxt, RESET + HOUR)                     # 10 AM, a new week
        h.record(3, nxt, RESET + 3 * HOUR)
        days, weeks = h.usage(4, nxt, RESET + 4 * HOUR)
        self.assertEqual(days, [{'day': '2026-10-15', 'used': 4.0}])      # from 0 at the reset, not from 45
        self.assertEqual(weeks[0]['reset'], RESET)
        self.assertEqual(weeks[0]['pct'], 45.0)
        self.assertEqual(weeks[0]['days'][-1], {'day': '2026-10-15', 'used': None})   # 40 by 7 AM: nobody saw when


class WeekArchiveTest(unittest.TestCase):
    def setUp(self):
        self.dir = Path(mkdtemp())
        self.h = fleet.WeekHistory(self.dir / 'week_history.json', tz=LA)

    def week(self, k, pct):
        reset = RESET + k * fleet.WEEK
        self.h.record(pct - 5, reset, reset - DAY)
        self.h.record(pct, reset, reset - HOUR)

    def test_keeps_the_newest_eight_weeks_newest_first(self):
        for k in range(11):
            self.week(k, 10 + k)
        self.week(11, 90)                                   # starts week 11: weeks 0..10 archived, 3..10 kept
        weeks = json.loads((self.dir / 'week_archive.json').read_text())['weeks']
        self.assertEqual(len(weeks), 8)
        self.assertEqual([w['pct'] for w in weeks], [20.0, 19.0, 18.0, 17.0, 16.0, 15.0, 14.0, 13.0])
        self.assertEqual(weeks[0]['reset'], RESET + 10 * fleet.WEEK)
        again = fleet.WeekHistory(self.dir / 'week_history.json', tz=LA)   # a restart keeps them
        self.assertEqual(again.weeks, weeks)

    def test_nothing_archived_without_samples_or_within_the_same_week(self):
        self.h.record(30, RESET, NOW)
        self.h.record(31, RESET + 60, NOW + 20 * MIN)       # the reset wobbled: same week
        self.assertFalse((self.dir / 'week_archive.json').exists())
        self.assertEqual(self.h.usage(31, RESET, NOW + 30 * MIN)[1], [])

    def test_a_week_cut_short_is_not_a_past_week(self):
        self.week(0, 30)
        early = RESET - HOUR + 30 * MIN + fleet.WEEK        # the limit was reset early, before the week's own reset
        self.assertTrue(self.h.record(1, early, RESET - 30 * MIN))
        self.assertEqual(self.h.samples, [[RESET - 30 * MIN, 1.0]])
        self.assertFalse((self.dir / 'week_archive.json').exists())
        self.assertEqual(self.h.weeks, [])

    def test_a_second_account_does_not_wipe_the_week_or_invent_past_weeks(self):
        # Two machines signed in to different accounts, whose weeks end two days apart. Whichever answers, one week
        # is kept (the one that ends last) and its samples stay.
        ours, theirs = RESET, RESET - 2 * DAY
        for k in range(6):
            self.h.record(30 + k, ours, NOW + k * 20 * MIN)
            self.assertFalse(self.h.record(10 + k, theirs, NOW + k * 20 * MIN + 10 * MIN))
        self.assertEqual([p for _, p in self.h.samples], [30.0, 31.0, 32.0, 33.0, 34.0, 35.0])
        self.assertEqual(self.h.reset, ours)
        self.assertFalse((self.dir / 'week_archive.json').exists())
        # The other way round (the week that ends later shows up second): one switch, then it stays.
        h = fleet.WeekHistory(self.dir / 'w2.json', tz=LA)
        h.record(10, theirs, NOW)
        for k in range(1, 5):
            h.record(30 + k, ours, NOW + k * 20 * MIN)
            self.assertFalse(h.record(10 + k, theirs, NOW + k * 20 * MIN + 10 * MIN))
        self.assertEqual([p for _, p in h.samples], [31.0, 32.0, 33.0, 34.0])
        self.assertEqual(h.weeks, [])

    def test_a_broken_archive_starts_empty(self):
        (self.dir / 'week_archive.json').write_text('{"weeks": "x"}')
        self.assertEqual(fleet.WeekHistory(self.dir / 'week_history.json', tz=LA).weeks, [])


class UsageEndpointTest(unittest.TestCase):
    def test_shape(self):
        m, state = Machines(), Path(mkdtemp())
        clock = [NOW]
        f = fleet.Fleet(MACH, m, clock=lambda: clock[0])
        app = server.App(f, state_dir=state, web_dir=state)
        f.history.tz = LA
        f.poll()
        clock[0] = NOW + 20 * MIN
        m.week = 33
        f.poll()
        u = app.get('/api/usage')
        self.assertEqual(set(u), {'ok', 'now', 'tz', 'five', 'week', 'days', 'weeks', 'updated', 'account', 'account_support', 'top'})
        self.assertTrue(u['ok'])
        self.assertEqual(u['five'], {'pct': 12, 'reset': None})
        self.assertEqual((u['week']['pct'], u['week']['reset']), (33, RESET))
        self.assertEqual(u['week']['pace'], fleet._projected(33, RESET, NOW + 20 * MIN))
        self.assertEqual(u['days'][0], {'day': '2026-10-08', 'used': None})   # before the server kept samples
        self.assertEqual(u['days'][-1], {'day': '2026-10-11', 'used': 2.0})
        self.assertEqual(len(u['days']), 4)
        self.assertEqual(u['weeks'], [])
        json.dumps(u)

    def test_before_any_numbers(self):
        f = fleet.Fleet(MACH, lambda *a: (_ for _ in ()).throw(OSError('down')), clock=lambda: NOW)
        state = Path(mkdtemp())
        u = server.App(f, state_dir=state, web_dir=state).get('/api/usage')
        self.assertEqual((u['five']['pct'], u['week']['pct'], u['week']['pace'], u['days'], u['weeks']),
                         (None, None, None, [], []))


class LimitsMergeTest(unittest.TestCase):
    """The plan numbers across machines (fleet.merge_limits, Fleet.limits)."""
    def fleet(self, limits, clock=None, skew=None):
        names = {'me@x': 'desk', 'me@m': 'laptop', 'me@y': 'tower'}
        self.limits, self.clock = limits, clock or [NOW]

        def transport(target, req, timeout):
            name = names[target]
            if self.limits.get(name) is None:
                raise OSError('down')
            return {'ok': True, 'sessions': [], 'limits': self.limits[name],
                    **({'clock': self.clock[0] - skew[name]} if skew and name in skew else {})}
        f = fleet.Fleet(MACH, transport, clock=lambda: self.clock[0])
        f.poll()
        return f

    def test_the_highest_number_in_the_open_week_wins_whatever_its_age(self):
        f = self.fleet({'desk': {'seven_day': 60, 'seven_day_reset': RESET, 'updated': NOW - 2 * HOUR, 'merged': True},
                        'laptop': {'seven_day': 33, 'seven_day_reset': RESET, 'updated': NOW - 5, 'merged': True}})
        s = f.summary()
        self.assertEqual((s['seven_day'], s['limits_updated']), (60, NOW - 2 * HOUR))
        self.assertEqual(f.usage()['week']['pct'], 60)

    def test_a_new_week_beats_a_higher_number_from_the_old_one(self):
        f = self.fleet({'desk': {'seven_day': 91, 'seven_day_reset': NOW - MIN, 'updated': NOW - HOUR, 'merged': True},
                        'laptop': {'seven_day': 0, 'seven_day_reset': NOW + fleet.WEEK - MIN, 'updated': NOW - 5, 'merged': True}})
        s = f.summary()
        self.assertEqual((s['seven_day'], s['seven_day_reset']), (0, NOW + fleet.WEEK - MIN))
        # Nobody has replied since the reset: the old number, and the reset time that says it is over. No pace.
        f = self.fleet({'desk': {'seven_day': 91, 'seven_day_reset': NOW - MIN, 'updated': NOW - HOUR, 'merged': True}})
        s = f.summary()
        self.assertEqual((s['seven_day'], s['seven_day_reset'], s['week_projected']), (91, NOW - MIN, None))
        self.assertIsNone(fleet._projected(59, NOW - 1, NOW))
        self.assertIsNone(f.usage()['week']['pace'])

    def test_the_five_hour_window_is_read_by_itself(self):
        five = NOW + 2 * HOUR
        f = self.fleet({'desk': {'seven_day': 60, 'seven_day_reset': RESET, 'five_hour': 20, 'five_hour_reset': five,
                                 'updated': NOW - 50, 'merged': True},
                        'laptop': {'seven_day': 58, 'seven_day_reset': RESET, 'five_hour': 24, 'five_hour_reset': five,
                                   'updated': NOW - 500, 'merged': True}})
        s = f.summary()
        self.assertEqual((s['seven_day'], s['five_hour'], s['five_hour_reset']), (60, 24, five))
        # A plan that reports no 5-hour window: the newest reply has none, whatever an old one saw weeks ago.
        f = self.fleet({'desk': {'seven_day': 60, 'seven_day_reset': RESET, 'updated': NOW - 50, 'merged': True},
                        'laptop': {'seven_day': 12, 'seven_day_reset': NOW - 20 * DAY, 'five_hour': 80,
                                   'five_hour_reset': NOW - 21 * DAY, 'updated': NOW - 21 * DAY, 'merged': True}})
        s = f.summary()
        self.assertEqual((s['seven_day'], s['five_hour'], s['five_hour_reset']), (60, None, None))

    def test_when_they_were_seen_is_on_the_servers_clock(self):
        # The laptop's clock runs five minutes fast, so its own times are five minutes ahead of the server's.
        f = self.fleet({'laptop': {'seven_day': 60, 'seven_day_reset': RESET, 'updated': NOW + 300 - 40, 'merged': True}},
                       skew={'laptop': -300})
        self.assertAlmostEqual(f.summary()['limits_updated'], NOW - 40, delta=1)

    def test_the_same_limits_twice_are_not_a_change(self):
        lim = {'desk': {'seven_day': 60, 'seven_day_reset': RESET, 'updated': NOW - 50, 'merged': True}}
        f = self.fleet(lim)
        changed = []
        f.on_change = changed.append
        self.clock[0] = NOW + 15
        f.poll()                                                     # the agent's answer is the same until a reply
        self.assertEqual(changed, [])

    def test_an_agent_from_before_the_merge_cannot_pull_the_week_down(self):
        # It reports whichever chat drew its status line last: 57, 33, 59, 33... with `updated` always seconds old.
        lim = {'desk': {'seven_day': 57, 'seven_day_reset': RESET, 'updated': NOW - 3}}
        f = self.fleet(lim)
        f.history = fleet.WeekHistory(Path(mkdtemp()) / 'w.json', tz=LA)   # the server samples the week on every poll
        f.poll()
        seen = []
        for k, pct in enumerate([33, 59, 33, 52, 60, 33]):
            self.clock[0] = NOW + (k + 1) * 15 * MIN
            lim['desk'] = {'seven_day': pct, 'seven_day_reset': RESET, 'updated': self.clock[0] - 3}
            f.poll()
            seen.append(f.summary()['seven_day'])
        self.assertEqual(seen, [57, 59, 59, 59, 60, 60])
        self.assertEqual([p for _, p in f.history.samples], [57.0, 57.0, 59.0, 59.0, 59.0, 60.0, 60.0])
        self.assertAlmostEqual(f.summary()['limits_updated'], NOW + 6 * 15 * MIN - 3, delta=1,
                               msg='as of the last time the machine showed a number, not when 60 first showed')
        # A new week starts it again.
        nxt = RESET + fleet.WEEK
        self.clock[0] = RESET + HOUR
        lim['desk'] = {'seven_day': 2, 'seven_day_reset': nxt, 'updated': self.clock[0] - 3}
        f.poll()
        self.assertEqual(f.summary()['seven_day'], 2)

    def old(self, week, five=None, five_reset=None, **kw):
        """What an agent from before merged limits says: one chat's numbers, drawn seconds ago."""
        return dict({'seven_day': week, 'seven_day_reset': RESET, 'five_hour': five,
                     'five_hour_reset': five_reset, 'updated': self.clock[0] - 3}, **kw)

    def test_an_old_agents_redraws_are_not_a_change_every_poll(self):
        self.clock = [NOW]
        lim = {'desk': self.old(57)}
        f = self.fleet(lim, self.clock)
        changed = []
        f.on_change = changed.append
        for pct in (33, 57, 41, 33, 57, 12):        # 15 seconds apart: stale chats drawing their old numbers
            self.clock[0] += 15
            lim['desk'] = self.old(pct)
            f.poll()
        self.assertLessEqual(len(changed), 1, 'at most the one step in its age, never one a poll')
        del changed[:]
        self.clock[0] = (NOW // fleet.LIMITS_STEP + 1) * fleet.LIMITS_STEP + 100   # well inside one step of the age
        lim['desk'] = self.old(57)
        f.poll()
        del changed[:]
        self.clock[0] += 15
        lim['desk'] = self.old(58)                  # the number that stands moved: that is one
        f.poll()
        self.assertEqual(changed, ['desk'])
        del changed[:]
        for _ in range(40):                         # ten minutes of redraws: told a few times, for the "as of" age
            self.clock[0] += 15
            lim['desk'] = self.old(33)
            f.poll()
        self.assertIn(len(changed), (2, 3))

    def test_a_held_number_is_as_fresh_as_the_last_sighting(self):
        # 45% stands for 40 minutes while chats keep replying: the apps must not call it 40 minutes old.
        self.clock = [NOW]
        lim = {'desk': self.old(45)}
        f = self.fleet(lim, self.clock)
        for pct in (45, 41, 45, 44):
            self.clock[0] += 10 * MIN
            lim['desk'] = self.old(pct)
            f.poll()
            s = f.summary()
            self.assertEqual(s['seven_day'], 45)
            self.assertAlmostEqual(s['limits_updated'], self.clock[0] - 3, delta=1)
        import budget
        st = budget.compute(budget.DEFAULTS, 45, RESET, None, None, 5.0, self.clock[0], LA, f.limits()['updated'])
        self.assertNotIn('These numbers are from', st['summary'])

    def test_the_five_hours_are_held_by_their_own_reset(self):
        self.clock = [NOW]
        five = NOW + 2 * HOUR
        lim = {'desk': self.old(45, 30, five)}
        f = self.fleet(lim, self.clock)
        seen = []
        for pct, reset in ((34, five), (12, five), (None, None), (20, five - 5 * HOUR), (36, five + 60)):
            self.clock[0] += MIN
            lim['desk'] = self.old(45, pct, reset)     # a stale chat redraws 12, one with no window, one from the last
            f.poll()
            seen.append(f.summary()['five_hour'])
        self.assertEqual(seen, [34, 34, 34, 34, 36])
        # the window ends: nothing is held open past its reset, and the next window starts from what it shows
        self.clock[0] = five + 2 * MIN
        lim['desk'] = self.old(45)
        f.poll()
        s = f.summary()
        self.assertEqual((s['five_hour'], s['five_hour_reset']), (36, five + 60), 'the last one, and its reset says it is over')
        import budget
        st = budget.compute(budget.DEFAULTS, 45, RESET, s['five_hour'], s['five_hour_reset'], 5.0, self.clock[0], LA)
        self.assertEqual((st['five_hour'], st['five_hour_state'], st['over_five_hour']), (None, 'reset', False))
        self.assertIn('5 hours: no 5-hour window open', st['summary'])
        nxt = self.clock[0] + 5 * HOUR
        lim['desk'] = self.old(45, 3, nxt)
        f.poll()
        self.assertEqual((f.summary()['five_hour'], f.summary()['five_hour_reset']), (3, nxt))

    def test_a_restart_does_not_let_the_week_dip(self):
        # The held number lived in memory only: the first poll after a deploy showed whichever stale chat drew last.
        self.clock = [NOW]
        path = Path(mkdtemp()) / 'w.json'
        lim = {'desk': self.old(57)}
        f = self.fleet(lim, self.clock)
        f.history = fleet.WeekHistory(path, tz=LA)
        f.poll()
        self.clock[0] += 5 * MIN
        lim['desk'] = self.old(41)
        again = fleet.Fleet(MACH, f.transport, clock=lambda: self.clock[0])   # the server starts again
        again.history = fleet.WeekHistory(path, tz=LA)
        again.poll()
        self.assertEqual(again.summary()['seven_day'], 57)
        self.assertEqual([p for _, p in again.history.samples], [57.0])
        self.clock[0] += 15 * MIN
        lim['desk'] = self.old(58)
        again.poll()
        self.assertEqual(again.summary()['seven_day'], 58)
        # last week's file says nothing about a new week
        nxt = RESET + fleet.WEEK
        self.clock[0] = RESET + HOUR
        lim['desk'] = {'seven_day': 1, 'seven_day_reset': nxt, 'updated': self.clock[0] - 3}
        third = fleet.Fleet(MACH, f.transport, clock=lambda: self.clock[0])
        third.history = fleet.WeekHistory(path, tz=LA)
        third.poll()
        self.assertEqual(third.summary()['seven_day'], 1)

    def test_one_reading_that_cannot_be_true_does_not_stand_for_a_week(self):
        ok = {'seven_day': 40, 'seven_day_reset': RESET, 'five_hour': 10, 'five_hour_reset': NOW + HOUR, 'updated': NOW - 60}
        for odd in ({'seven_day': 97, 'seven_day_reset': NOW + 30 * DAY},            # a week that ends in a month
                    {'seven_day': 5000, 'seven_day_reset': RESET}, {'seven_day': -3, 'seven_day_reset': RESET},
                    {'seven_day': float('nan'), 'seven_day_reset': RESET},
                    {'five_hour': 99, 'five_hour_reset': NOW + 2 * DAY},             # five hours that end in two days
                    {'five_hour': 1e9, 'five_hour_reset': NOW + HOUR}):
            got = fleet.merge_limits([ok, dict(odd, updated=NOW - 5)], NOW)
            self.assertEqual((got['seven_day'], got['seven_day_reset'], got['five_hour'], got['five_hour_reset']),
                             (40, RESET, 10, NOW + HOUR), odd)
        # a little over the limit is a reading: it shows as all of it
        got = fleet.merge_limits([ok, {'seven_day': 103.5, 'seven_day_reset': RESET, 'updated': NOW}], NOW)
        self.assertEqual(got['seven_day'], 100)
        # and from an agent from before merged limits, it is not held either
        self.clock = [NOW]
        lim = {'desk': self.old(40)}
        f = self.fleet(lim, self.clock)
        f.history = fleet.WeekHistory(Path(mkdtemp()) / 'w.json', tz=LA)
        f.poll()
        for bad in ({'seven_day': 97, 'seven_day_reset': NOW + 30 * DAY, 'updated': NOW}, self.old(4000)):
            self.clock[0] += 16 * MIN
            lim['desk'] = bad
            f.poll()
            self.assertEqual((f.summary()['seven_day'], f.summary()['seven_day_reset']), (40, RESET))
        self.assertEqual({p for _, p in f.history.samples}, {40.0})
        self.assertEqual(f.history.reset, RESET)

    def test_the_week_file_forgets_a_reset_no_week_could_have(self):
        h = fleet.WeekHistory(Path(mkdtemp()) / 'w.json', tz=LA)
        self.assertFalse(h.record(97, NOW + 30 * DAY, NOW))
        self.assertFalse(h.record(900, RESET, NOW))
        h.reset, h.samples = NOW + 30 * DAY, [[NOW - HOUR, 97.0]]   # written by a server from before the check
        self.assertTrue(h.record(40, RESET, NOW))
        self.assertEqual((h.reset, h.samples), (RESET, [[float(NOW), 40.0]]))

    def test_no_window_open_is_not_a_plan_without_one(self):
        # One chat saw a 5-hour window this week; the newest reply, from another machine, shows none.
        rows = [{'seven_day': 40, 'seven_day_reset': RESET, 'five_hour': 80, 'five_hour_reset': NOW - DAY, 'updated': NOW - DAY - HOUR},
                {'seven_day': 41, 'seven_day_reset': RESET, 'updated': NOW - 5}]
        got = fleet.merge_limits(rows, NOW)
        self.assertEqual((got['seven_day'], got['five_hour'], got['five_hour_reset']), (41, 80, NOW - DAY))
        import budget
        st = budget.compute(budget.DEFAULTS, 41, RESET, got['five_hour'], got['five_hour_reset'], 5.0, NOW, LA)
        self.assertEqual(st['five_hour_state'], 'reset')
        self.assertNotIn('not reported on this plan', st['summary'])
        st = budget.compute(budget.DEFAULTS, 41, RESET, None, None, 5.0, NOW, LA)
        self.assertIn('5 hours: not reported on this plan', st['summary'])

    def test_odd_limits_never_break_it(self):
        f = self.fleet({'desk': {'seven_day': 'x', 'seven_day_reset': True, 'updated': None},
                        'laptop': 'nope', 'tower': {'seven_day': 40, 'updated': 'soon'}})
        s = f.summary()
        self.assertEqual((s['seven_day'], s['seven_day_reset'], s['five_hour']), (40, None, None))
        self.assertEqual(fleet.Fleet(MACH, lambda *a: (_ for _ in ()).throw(OSError('down'))).limits(), {})


class RunOutNoteTest(unittest.TestCase):
    def test_the_note_says_how_far_short_the_week_falls_not_how_long_until_the_reset(self):
        import notes
        # 63% used with 3 days left: on pace for 110%, so the week is used up 100/110 of the way through, 0.6 days
        # before the reset. It used to say "3 days before the week resets".
        sm = {'seven_day': 63, 'seven_day_reset': NOW + 3 * DAY, 'week_projected': fleet._projected(63, NOW + 3 * DAY, NOW)}
        self.assertEqual(sm['week_projected'], 110)
        n = notes.pick([], sm, NOW)
        self.assertEqual((n['kind'], n['tone']), ('week', 'bad'))
        self.assertTrue(n['text'].startswith("At this rate you'll run out 15 h before the week resets."), n['text'])
        # Never more than the time that is left.
        late = notes.pick([], {'seven_day': 99, 'seven_day_reset': NOW + HOUR, 'week_projected': 400}, NOW)
        self.assertIn("run out 1 h before the week resets", late['text'])


class NeedsTest(RpcBase):
    SHORT = 'aaaaaaaa'

    def job(self, **st):
        d = self.cdir / 'jobs' / self.SHORT
        d.mkdir(parents=True, exist_ok=True)
        (d / 'state.json').write_text(json.dumps(st))

    def row(self, **a):
        self.transcript(SID_A, str(self.root / 'proj'), RPC_NOW - 60)
        ag = dict({'id': self.SHORT, 'sessionId': SID_A, 'kind': 'background', 'pid': 4242, 'cwd': str(self.root / 'proj')}, **a)
        r = self.sb.rpc({'verb': 'list'}, RPC_NOW, agents=[ag])
        return next(s for s in r['sessions'] if s['sid'] == SID_A)

    def test_a_blocked_chat_says_what_it_needs_trimmed(self):
        self.job(state='blocked', needs='  Confirm option 1 ' + 'x' * 400, suggestedReply='Go with option 1' + 'y' * 300)
        s = self.row(status='idle', state='blocked')
        self.assertTrue(s['needs'].startswith('Confirm option 1 x'))
        self.assertEqual(len(s['needs']), 300)
        self.assertEqual(len(s['suggested_reply']), 200)

    def test_waiting_also_reads_it(self):
        self.job(state='blocked', needs='Pick one', suggestedReply='One')
        s = self.row(status='waiting', state='running')
        self.assertEqual((s['needs'], s['suggested_reply']), ('Pick one', 'One'))

    def test_not_blocked_or_waiting_has_neither(self):
        self.job(state='blocked', needs='Pick one', suggestedReply='One')
        s = self.row(status='busy', state='running')
        self.assertNotIn('needs', s)
        self.assertNotIn('suggested_reply', s)

    def test_missing_parts_are_left_out(self):
        self.job(state='blocked', needs='Pick one')
        s = self.row(status='idle', state='blocked')
        self.assertEqual(s['needs'], 'Pick one')
        self.assertNotIn('suggested_reply', s)
        self.assertNotIn('needs', self.row(status='idle', state='blocked', id='../../etc'))

    def test_the_server_passes_them_through(self):
        row = {'sid': SID_A, 'title': 't', 'mtime': NOW - 60, 'active': NOW - 60, 'kind': 'background', 'status': 'idle',
               'state': 'blocked', 'needs': 'Pick one', 'suggested_reply': 'One'}
        f = fleet.Fleet(MACH[:1], lambda *a: {'ok': True, 'sessions': [row]}, clock=lambda: NOW)
        state = Path(mkdtemp())
        app = server.App(f, state_dir=state, web_dir=state)
        f.poll()
        s = app.get('/api/sessions')['machines'][0]['sessions'][0]
        self.assertEqual((s['needs'], s['suggested_reply']), ('Pick one', 'One'))


if __name__ == '__main__':
    unittest.main()
