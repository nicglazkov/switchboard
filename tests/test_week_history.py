"""The week's history for the iPhone widgets: the sampler, its file, and the summary fields built from it."""
import json, sys, unittest
from datetime import datetime
from pathlib import Path
from tempfile import mkdtemp
from zoneinfo import ZoneInfo
sys.path.insert(0, str(Path(__file__).resolve().parents[1] / 'server'))
import fleet, server  # noqa: E402

LA = ZoneInfo('America/Los_Angeles')
MIN, HOUR, DAY = 60, 3600, 86400


def at(*args):
    return datetime(*args, tzinfo=LA).timestamp()


RESET = at(2026, 10, 15, 9)        # Thursday 9 AM
START = RESET - fleet.WEEK         # Thursday 9 AM, a week earlier
NOW = at(2026, 10, 11, 15, 40)     # Sunday 3:40 PM


def day_letters(spans):
    return [datetime.fromtimestamp((a + b) / 2, LA).strftime('%a') for a, b in spans]


class WeekDaysTest(unittest.TestCase):
    def test_a_morning_reset_folds_the_last_morning_into_wednesday(self):
        spans = fleet.week_days(RESET, LA)
        self.assertEqual(len(spans), 7)
        self.assertEqual(spans[0], (START, at(2026, 10, 9)))             # Thu 9 AM to midnight
        self.assertEqual(spans[-1], (at(2026, 10, 14), RESET))          # Wed midnight to Thu 9 AM
        self.assertEqual(day_letters(spans), ['Thu', 'Fri', 'Sat', 'Sun', 'Mon', 'Tue', 'Wed'])
        self.assertTrue(all(a < b for a, b in spans))
        self.assertTrue(all(spans[i][1] == spans[i + 1][0] for i in range(6)))

    def test_an_evening_reset_folds_the_first_evening_into_the_next_day(self):
        reset = at(2026, 10, 13, 14)   # Tuesday 2 PM: 10 hours left of the first day, 14 of the last
        spans = fleet.week_days(reset, LA)
        self.assertEqual(spans[0], (reset - fleet.WEEK, at(2026, 10, 8)))   # Tue 2 PM through Wednesday
        self.assertEqual(day_letters(spans), ['Wed', 'Thu', 'Fri', 'Sat', 'Sun', 'Mon', 'Tue'])

    def test_a_midnight_reset_is_seven_whole_days(self):
        spans = fleet.week_days(at(2026, 10, 15), LA)
        self.assertEqual([b - a for a, b in spans], [DAY] * 7)

    def test_a_daylight_saving_week_still_has_seven_days(self):
        reset = at(2026, 11, 5, 9)     # the clocks go back on Sunday Nov 1
        spans = fleet.week_days(reset, LA)
        self.assertEqual(len(spans), 7)
        self.assertEqual(spans[0][0], reset - fleet.WEEK)
        self.assertEqual(spans[-1][1], reset)
        self.assertEqual(day_letters(spans), ['Thu', 'Fri', 'Sat', 'Sun', 'Mon', 'Tue', 'Wed'])


def steady(per_day, until, every=15 * MIN):
    """Samples every 15 minutes from the reset until `until`, spending `per_day[i]` evenly over column i."""
    spans, out, t, total = fleet.week_days(RESET, LA), [], START + every, 0.0
    while t <= until:
        i = next(k for k, (a, b) in enumerate(spans) if a < t <= b)
        a, b = spans[i]
        total += per_day[i] * every / (b - a)
        out.append((t, round(total, 3)))
        t += every
    return out


class WeekDailyTest(unittest.TestCase):
    def test_days_add_up_to_the_week_so_far(self):
        samples = steady([8, 11, 7, 5 / (NOW - at(2026, 10, 11)) * DAY], NOW)
        daily = fleet.week_daily(samples, samples[-1][1], RESET, NOW, LA)
        self.assertEqual(len(daily), 4)                       # Thu, Fri, Sat and today, Sunday
        self.assertEqual(daily[:3], [8.0, 11.0, 7.0])
        self.assertAlmostEqual(daily[3], 5, delta=0.3)
        self.assertAlmostEqual(sum(daily), samples[-1][1], delta=0.3)

    def test_today_counts_up_to_the_percent_now(self):
        samples = steady([8, 11, 7, 0], at(2026, 10, 11, 15))
        daily = fleet.week_daily(samples, 30, RESET, NOW, LA)  # 4% more since the last sample
        self.assertAlmostEqual(daily[-1], 4, delta=0.1)

    def test_a_fresh_install_does_not_make_up_the_days_before_it(self):
        samples = [(NOW - 30 * MIN, 30.0), (NOW - 15 * MIN, 30.5)]
        self.assertEqual(fleet.week_daily(samples, 31, RESET, NOW, LA), [None, None, None, None])
        # A day later, Monday is whole again.
        mon = at(2026, 10, 12, 20)
        later = samples + [(t, 31 + (t - NOW) / (mon - NOW) * 6) for t in range(int(NOW), int(mon), 15 * MIN)]
        daily = fleet.week_daily(later, 37, RESET, mon, LA)
        self.assertEqual(daily[:4], [None, None, None, None])
        self.assertIsNotNone(daily[4])

    def test_a_quiet_outage_is_still_known_but_a_busy_one_is_not(self):
        thu, fri = at(2026, 10, 8, 22), at(2026, 10, 9, 4)       # the server was down across midnight
        before = [(START + HOUR, 2.0), (START + 2 * HOUR, 5.0), (thu, 5.0)]
        after = [(fri, 5.0), (fri + HOUR, 9.0)]
        quiet = fleet.week_daily(before + after, 9, RESET, NOW, LA)
        self.assertEqual(quiet[0], 5.0)                         # nothing was used while it was down
        busy = fleet.week_daily(before + [(fri, 7.0), (fri + HOUR, 9.0)], 9, RESET, NOW, LA)
        self.assertEqual(busy[:2], [None, None])                # 2% went somewhere between Thursday and Friday

    def test_the_first_hours_after_a_reset(self):
        now = START + 2 * HOUR
        self.assertEqual(fleet.week_daily([(START + HOUR, 1.0)], 2, RESET, now, LA), [2.0])
        self.assertEqual(fleet.week_daily([], 0, RESET, START + 10 * MIN, LA), [0.0])


class WeekTraceTest(unittest.TestCase):
    def test_thinned_to_one_an_hour(self):
        samples = steady([8, 11, 7, 5], NOW)
        trace = fleet.week_trace(samples, RESET)
        hours = [int((t - START) // HOUR) for t, _ in trace]
        self.assertEqual(hours, sorted(set(hours)))
        self.assertLessEqual(len(trace), 7 * 24)
        self.assertEqual(trace[-1], [int(samples[-1][0]), round(samples[-1][1], 1)])   # ends on the newest sample
        self.assertTrue(all(isinstance(t, int) for t, _ in trace))


class WeekHistoryTest(unittest.TestCase):
    def setUp(self):
        self.path = Path(mkdtemp()) / 'week_history.json'
        self.h = fleet.WeekHistory(self.path, tz=LA)

    def test_samples_at_most_every_fifteen_minutes(self):
        self.assertTrue(self.h.record(30, RESET, NOW))
        self.assertFalse(self.h.record(30.2, RESET, NOW + 5 * MIN))
        self.assertFalse(self.h.record(30.4, RESET, NOW + 14 * MIN))
        self.assertTrue(self.h.record(30.6, RESET, NOW + 15 * MIN))
        self.assertEqual(self.h.samples, [[NOW, 30.0], [NOW + 15 * MIN, 30.6]])
        self.assertEqual(json.loads(self.path.read_text())['samples'], [[NOW, 30.0], [NOW + 15 * MIN, 30.6]])

    def test_nothing_is_kept_without_numbers_or_from_a_week_that_is_over(self):
        self.assertFalse(self.h.record(None, RESET, NOW))
        self.assertFalse(self.h.record(30, None, NOW))
        self.assertFalse(self.h.record(30, NOW - HOUR, NOW))   # the newest numbers are from before the last reset
        self.assertFalse(self.path.exists())
        self.assertEqual(self.h.view(30, NOW - HOUR, NOW), ([], []))

    def test_a_new_week_drops_the_last_one(self):
        for k in range(8):
            self.h.record(60 + k, RESET, RESET - 3 * HOUR + k * 15 * MIN)
        nxt = RESET + fleet.WEEK
        self.assertTrue(self.h.record(1, nxt, RESET + 20 * MIN))
        self.assertEqual(self.h.samples, [[RESET + 20 * MIN, 1.0]])
        self.assertEqual(json.loads(self.path.read_text())['reset'], nxt)
        daily, trace = self.h.view(1, nxt, RESET + 25 * MIN)
        self.assertEqual(trace, [[int(RESET + 20 * MIN), 1.0]])
        self.assertEqual(len(daily), 1)

    def test_an_early_reset_also_starts_over(self):
        self.h.record(70, RESET, NOW)
        moved = NOW + fleet.WEEK                                   # the limit was reset early, today
        self.assertTrue(self.h.record(0, moved, NOW + MIN))
        self.assertEqual(self.h.samples, [[NOW + MIN, 0.0]])

    def test_a_reset_that_wobbles_by_seconds_is_the_same_week(self):
        self.h.record(30, RESET, NOW)
        self.h.record(31, RESET + 1, NOW + 20 * MIN)
        self.assertEqual(len(self.h.samples), 2)
        self.assertEqual(len(self.h.view(31, RESET - 1, NOW + 21 * MIN)[1]), 2)

    def test_samples_from_before_the_week_are_dropped(self):
        self.path.write_text(json.dumps({'reset': RESET, 'samples': [[START - DAY, 90], [START + HOUR, 1], [NOW - HOUR, 30]]}))
        h = fleet.WeekHistory(self.path, tz=LA)
        h.record(31, RESET, NOW)
        self.assertEqual([s[0] for s in h.samples], [START + HOUR, NOW - HOUR, NOW])

    def test_a_restart_keeps_the_week(self):
        for k in range(12):
            self.h.record(20 + k, RESET, NOW - 3 * HOUR + k * 15 * MIN)
        again = fleet.WeekHistory(self.path, tz=LA)
        self.assertEqual(again.samples, self.h.samples)
        self.assertEqual(again.view(32, RESET, NOW), self.h.view(32, RESET, NOW))
        self.assertEqual(len(again.view(32, RESET, NOW)[1]), 4)   # 75:40 to 78:25 after the reset: four hours
        self.assertFalse(again.record(32, RESET, NOW - 3 * HOUR + 11 * 15 * MIN + MIN))  # still 15 minutes apart
        self.assertEqual(list(self.path.parent.iterdir()), [self.path])            # written in place, no temp left

    def test_a_broken_file_starts_empty(self):
        for text in ('', '{', '[]', '{"reset": "x", "samples": []}', '{"reset": 1, "samples": [[1]]}'):
            self.path.write_text(text)
            h = fleet.WeekHistory(self.path, tz=LA)
            self.assertEqual((h.reset, h.samples), (None, []), text)
            self.assertTrue(h.record(30, RESET, NOW))

    def test_the_file_stays_small(self):
        h = fleet.WeekHistory(self.path, every=60, tz=LA)
        for k in range(1500):
            h.record(k / 100, RESET, START + 60 + k * 60)
        self.assertEqual(len(h.samples), fleet.WeekHistory.KEEP)
        self.assertEqual(len(json.loads(self.path.read_text())['samples']), fleet.WeekHistory.KEEP)


MACH = [{'name': 'desk', 'label': 'Desk', 'target': 'me@x', 'os': 'mac'},
        {'name': 'laptop', 'label': 'Laptop', 'target': 'me@m', 'os': 'mac'},
        {'name': 'tower', 'label': 'Tower', 'target': 'me@y', 'os': 'windows'}]


def sess(sid, **kw):
    d = {'sid': sid, 'title': 't', 'ctx': 20_000, 'cold': False, 'mtime': NOW - 60, 'kind': None, 'folder': 'p'}
    d.update(kw)
    return d


class Machines:
    def __init__(self):
        self.down = {'tower'}
        self.week = 31
        self.sessions = {
            'desk': [sess('a', state='running', kind='interactive', status='busy'),
                     sess('b', cold=True, ctx=400_000), sess('c', cold=True, ctx=40_000)],
            'laptop': [sess('d', state='running', kind='interactive', status='busy'),
                        sess('e', state='running', kind='interactive', status='busy', mtime=NOW - 9 * HOUR)],  # stalled
            'tower': []}

    def __call__(self, target, req, timeout):
        name = {'me@x': 'desk', 'me@m': 'laptop', 'me@y': 'tower'}[target]
        if name in self.down:
            raise OSError('ssh: connect timed out')
        return {'ok': True, 'sessions': self.sessions[name],
                'limits': {'five_hour': 12, 'seven_day': self.week, 'seven_day_reset': RESET,
                           'updated': NOW - (5 if name == 'desk' else 50)}}


class SummaryFieldsTest(unittest.TestCase):
    def setUp(self):
        self.clock = [NOW]
        self.m = Machines()
        self.f = fleet.Fleet(MACH, self.m, clock=lambda: self.clock[0])

    def test_machine_list(self):
        self.f.poll()
        s = self.f.summary()
        self.assertEqual(s['machine_list'], [
            {'name': 'desk', 'label': 'Desk', 'online': True, 'os': 'mac', 'icon': 'desktop', 'running': 1, 'cold': 1},
            {'name': 'laptop', 'label': 'Laptop', 'online': True, 'os': 'mac', 'icon': 'desktop', 'running': 1, 'cold': 0},
            {'name': 'tower', 'label': 'Tower', 'online': False, 'os': 'windows', 'icon': 'tower', 'running': 0, 'cold': 0}])
        self.assertEqual((s['running'], s['cold'], s['online']), (2, 1, 2))   # the totals agree with the list

    def test_machine_list_before_the_first_poll(self):
        self.assertEqual([m['online'] for m in self.f.summary()['machine_list']], [False, False, False])

    def test_without_a_history_the_week_fields_are_empty(self):
        self.f.poll()
        s = self.f.summary()
        self.assertEqual((s['week_daily'], s['week_trace']), ([], []))

    def test_polls_fill_the_week(self):
        self.f.history = fleet.WeekHistory(Path(mkdtemp()) / 'w.json', tz=LA)
        for k in range(10):                    # 30 minutes of polls every 3 minutes
            self.m.week = 30 + k * 0.1
            self.clock[0] = NOW + k * 3 * MIN
            self.f.poll()
        self.assertEqual([round(p, 1) for _, p in self.f.history.samples], [30.0, 30.5])   # one per 15 minutes
        s = self.f.summary()
        self.assertEqual(s['week_trace'], [[int(NOW + 15 * MIN), 30.5]])   # both in the same hour: the later one
        self.assertEqual(len(s['week_daily']), 4)        # Thu, Fri, Sat, Sun; the days before the samples are unknown
        self.assertEqual(s['week_daily'][:3], [None, None, None])
        json.dumps(s)                                    # plain JSON, nulls and all

    def test_the_server_keeps_the_history_in_its_state_dir(self):
        state = Path(mkdtemp())
        app = server.App(self.f, state_dir=state, web_dir=state)
        self.f.poll()
        self.assertTrue((state / 'week_history.json').exists())
        s = app.get('/api/summary')
        self.assertEqual(s['week_trace'], [[int(NOW), 31.0]])
        self.assertEqual(s['machine_list'][0]['label'], 'Desk')
        # A restart picks the week back up.
        f2 = fleet.Fleet(MACH, self.m, clock=lambda: NOW + MIN)
        server.App(f2, state_dir=state, web_dir=state)
        f2.poll()
        self.assertEqual(f2.summary()['week_trace'], [[int(NOW), 31.0]])


if __name__ == '__main__':
    unittest.main()
