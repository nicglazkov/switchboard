"""What each chat used this week ("Most used this week" in the usage panel): the helper's count from the
transcripts (agent/sb.py spend_report), what the server keeps of each machine's answer, and GET /api/usage `top`."""
import datetime, json, os, sys, unittest
from pathlib import Path
from tempfile import mkdtemp
from tests.test_rpc import RpcBase
sys.path.insert(0, str(Path(__file__).resolve().parents[1] / 'server'))
import fleet, server  # noqa: E402

HOUR, DAY = 3600, 86400
NOW = 1_800_000_000.0 - 1_800_000_000.0 % 600 + 300   # five minutes into a ten-minute bucket
WEEK = NOW - 3 * DAY - 300                            # the week began three days ago, on a bucket's edge
SID_A = 'aaaaaaaa-1111-2222-3333-444444444444'
SID_B = 'bbbbbbbb-1111-2222-3333-444444444444'
SID_C = 'cccccccc-1111-2222-3333-444444444444'
MACH = [{'name': 'studio', 'label': 'Studio', 'target': 'me@studio', 'os': 'mac'},
        {'name': 'laptop', 'label': 'Laptop', 'target': 'me@laptop', 'os': 'mac'}]


def iso(t):
    return datetime.datetime.fromtimestamp(t, datetime.timezone.utc).strftime('%Y-%m-%dT%H:%M:%S.%fZ')


def reply(t, mid, sid=SID_A, side=False, **usage):
    """One line of a reply, as Claude Code writes it."""
    u = {'input_tokens': 0, 'cache_creation_input_tokens': 0, 'cache_read_input_tokens': 0, 'output_tokens': 0}
    u.update(usage)
    return {'type': 'assistant', 'timestamp': iso(t), 'sessionId': sid, 'isSidechain': side, 'uuid': f'u-{mid}-{t}',
            'message': {'id': mid, 'role': 'assistant', 'model': 'claude-opus-5-5', 'content': [], 'usage': u}}


def said(t, text='do the thing'):
    return {'type': 'user', 'timestamp': iso(t), 'message': {'role': 'user', 'content': text}}


class SpendBase(RpcBase):
    def setUp(self):
        super().setUp()
        self.proj = self.cdir / 'projects' / 'p1'

    def write(self, path, rows, mode='w', mtime=NOW):
        path = Path(path)
        path.parent.mkdir(parents=True, exist_ok=True)
        with open(path, mode) as f:
            for r in rows:
                f.write((r if isinstance(r, str) else json.dumps(r)) + '\n')
        os.utime(path, (mtime, mtime))
        return path

    def chat(self, sid, rows, **kw):
        return self.write(self.proj / f'{sid}.jsonl', rows, **kw)

    def report(self, since=WEEK, now=NOW, **kw):
        return self.sb.spend_report({'since': since}, now, **kw)

    def used(self, **kw):
        r = self.report(**kw)
        return {c['sid']: c['w'] for c in r.get('chats') or []}


class WeightTest(SpendBase):
    def test_tokens_are_weighted_by_the_cost_ratios(self):
        w = self.sb.spend_weight
        self.assertEqual(w({'input_tokens': 100, 'output_tokens': 10, 'cache_read_input_tokens': 1000}), 100 + 50 + 100)
        # the cache write by how long it is kept: 1.25 for five minutes, 2 for an hour
        self.assertEqual(w({'cache_creation_input_tokens': 300,
                            'cache_creation': {'ephemeral_5m_input_tokens': 100, 'ephemeral_1h_input_tokens': 200}}), 125 + 400)
        self.assertEqual(w({'cache_creation_input_tokens': 400}), 500)   # no split: the shorter cache
        self.assertEqual(w({'input_tokens': None, 'output_tokens': 'x', 'cache_read_input_tokens': -5}), 0)


class CountTest(SpendBase):
    def test_a_reply_written_as_several_lines_counts_once(self):
        t = NOW - HOUR
        self.chat(SID_A, [said(t - 5), reply(t, 'm1', input_tokens=1000), reply(t, 'm1', input_tokens=1000),
                          reply(t + 1, 'm1', input_tokens=1000), reply(t + 60, 'm2', output_tokens=100)])
        r = self.report()
        self.assertEqual((r['total'], r['n'], r['more'], r['since']), (1500, 1, False, WEEK))
        self.assertEqual(r['chats'], [{'sid': SID_A, 'w': 1500}])
        json.dumps(r)

    def test_only_replies_since_the_week_began_count(self):
        self.chat(SID_A, [reply(WEEK - 601, 'old', input_tokens=7000), reply(WEEK + 1, 'new', input_tokens=30),
                          reply(NOW - 60, 'newer', input_tokens=5)])
        self.assertEqual(self.used(), {SID_A: 35})
        # The week's start moves (a reset): the same kept buckets are added up from the new start, nothing is read again.
        self.sb._spend_read = lambda *a, **k: self.fail('read the transcript again')
        self.assertEqual(self.used(since=WEEK - HOUR), {SID_A: 7035})
        self.assertEqual(self.used(since=NOW - 600), {SID_A: 5})
        self.assertEqual(self.report(since=NOW + HOUR)['chats'], [])

    def test_the_start_is_matched_to_the_nearest_ten_minutes(self):
        # A machine whose clock is a few seconds off gets a start a few seconds off the hour: still the same bucket.
        self.chat(SID_A, [reply(WEEK - 30, 'before', input_tokens=100), reply(WEEK + 30, 'after', input_tokens=7)])
        for since in (WEEK - 4, WEEK, WEEK + 4):
            self.assertEqual(self.used(since=since), {SID_A: 7}, since)

    def test_a_chats_subagents_count_as_the_chat(self):
        t = NOW - HOUR
        self.chat(SID_A, [reply(t, 'm1', input_tokens=100)])
        self.write(self.proj / SID_A / 'subagents' / 'agent-a1.jsonl', [reply(t + 5, 's1', side=True, input_tokens=40)])
        self.write(self.proj / SID_A / 'subagents' / 'workflows' / 'wf_1' / 'agent-a2.jsonl', [reply(t + 9, 's2', side=True, input_tokens=2)])
        # a forked subagent starts with copies of its parent's lines: those were counted already
        self.write(self.proj / SID_A / 'subagents' / 'agent-a3.jsonl', [reply(t, 'm1', side=True, input_tokens=100), reply(t + 20, 's3', side=True, input_tokens=1)])
        self.write(self.proj / SID_A / 'tool-results' / 'big.jsonl', [reply(t, 'not-a-transcript', input_tokens=9999)])
        # the parent's own transcript in another project folder than the subagents' (the chat changed folder)
        self.write(self.cdir / 'projects' / 'p2' / SID_B / 'subagents' / 'agent-b1.jsonl', [reply(t, 'b1', side=True, input_tokens=8)])
        self.assertEqual(self.used(), {SID_A: 143, SID_B: 8})

    def test_a_resumed_chats_copied_history_is_not_counted_twice(self):
        t = NOW - 5 * HOUR
        self.chat(SID_A, [reply(t, 'm1', input_tokens=100), reply(t + 60, 'm2', input_tokens=200)], mtime=NOW - 4 * HOUR)
        # resumed under a new id: the new file starts with the old one's lines, then goes on
        self.chat(SID_B, [reply(t, 'm1', sid=SID_B, input_tokens=100), reply(t + 60, 'm2', sid=SID_B, input_tokens=200),
                          reply(NOW - 60, 'm3', sid=SID_B, input_tokens=50)])
        self.assertEqual(self.used(), {SID_A: 300, SID_B: 50})

    def test_only_what_was_appended_is_read_the_next_time(self):
        t = NOW - HOUR
        p = self.chat(SID_A, [reply(t, 'm1', input_tokens=100)])
        self.assertEqual(self.used(), {SID_A: 100})
        seen = []
        real = self.sb._spend_read
        self.sb._spend_read = lambda path, off, *a, **k: seen.append((Path(path).name, off)) or real(path, off, *a, **k)
        self.assertEqual(self.used(), {SID_A: 100})
        self.assertEqual(seen, [])   # nothing changed: no file is opened
        size = p.stat().st_size
        with open(p, 'a') as f:
            f.write(json.dumps(reply(t + 60, 'm2', input_tokens=11)) + '\n' + json.dumps(reply(t + 90, 'm3', input_tokens=1))[:40])
        os.utime(p, (NOW, NOW))
        self.assertEqual(self.used(), {SID_A: 111})   # the half-written last line waits
        self.assertEqual(seen, [(f'{SID_A}.jsonl', size)])
        with open(p, 'a') as f:
            f.write(json.dumps(reply(t + 90, 'm3', input_tokens=1))[40:] + '\n')
        os.utime(p, (NOW, NOW))
        self.assertEqual(self.used(), {SID_A: 112})

    def test_a_deleted_chat_keeps_what_it_used_this_week(self):
        p = self.chat(SID_A, [reply(NOW - HOUR, 'm1', input_tokens=100)])
        self.chat(SID_B, [reply(NOW - HOUR, 'm2', input_tokens=5)])
        self.assertEqual(self.used(), {SID_A: 100, SID_B: 5})
        p.unlink()
        self.assertEqual(self.used(), {SID_A: 100, SID_B: 5})
        # ... and drops out with the week: 8 days on nothing of it is kept
        later = NOW + 8 * DAY
        self.assertEqual(self.used(since=later - DAY, now=later), {})
        kept = json.loads((self.sb.paths()['state'] / 'spend.json').read_text())
        self.assertEqual(kept['chats'], {})

    def test_a_transcript_untouched_all_week_is_never_opened(self):
        self.chat(SID_A, [reply(NOW - 20 * DAY, 'm1', input_tokens=100)], mtime=NOW - 9 * DAY)
        self.sb._spend_read = lambda *a, **k: self.fail('opened it')
        self.assertEqual(self.report()['chats'], [])

    def test_lines_that_are_not_replies_or_cannot_be_read_are_passed_over(self):
        t = NOW - HOUR
        self.chat(SID_A, [said(t, 'x' * 5000), '{"type":"assistant","usage" broken', 'not json at all "usage" "assistant"',
                          {'type': 'assistant', 'timestamp': iso(t), 'message': {'id': 'm0', 'usage': 'no'}},
                          {'type': 'assistant', 'timestamp': 'yesterday', 'message': {'id': 'm9', 'usage': {'input_tokens': 5}}},
                          {'type': 'user', 'timestamp': iso(t), 'message': {'content': '"assistant" "usage"', 'usage': {'input_tokens': 999}}},
                          reply(t, 'm1', input_tokens=3)])
        self.assertEqual(self.used(), {SID_A: 3})

    def test_a_line_longer_than_the_buffer_is_skipped_and_the_rest_still_read(self):
        self.sb.SPEND_BUF = 4096
        t = NOW - HOUR
        self.chat(SID_A, [reply(t, 'm1', input_tokens=3), said(t, 'y' * 20000), reply(t + 1, 'm2', input_tokens=4),
                          said(t, 'z' * 3000), said(t, 'z' * 3000), reply(t + 2, 'm3', input_tokens=5)])
        self.assertEqual(self.used(), {SID_A: 12})

    def test_a_first_count_that_takes_long_goes_on_at_the_next_poll_and_says_nothing_until_done(self):
        self.sb.SPEND_BUF = 4096
        t = NOW - HOUR
        self.chat(SID_A, [reply(t + i, f'm{i}', input_tokens=1) for i in range(60)], mtime=NOW - 60)
        self.chat(SID_B, [reply(t + i, f'n{i}', input_tokens=2) for i in range(60)])
        r = self.report(budget=0)   # out of time after the first buffer
        self.assertEqual((r['more'], 'chats' in r, 'total' in r), (True, False, False))
        for _ in range(40):
            r = self.report(budget=0)
            if not r['more']:
                break
        self.assertEqual({c['sid']: c['w'] for c in r['chats']}, {SID_A: 60, SID_B: 120})

    def test_two_polls_at_once_do_not_both_read(self):
        self.chat(SID_A, [reply(NOW - HOUR, 'm1', input_tokens=3)])
        self.sb.lock_take(self.sb.paths()['state'] / 'spend.lock', NOW, 60)
        r = self.report()
        self.assertEqual((r['more'], 'chats' in r), (True, False))
        self.sb.lock_drop(self.sb.paths()['state'] / 'spend.lock')
        self.assertEqual(self.used(), {SID_A: 3})

    def test_a_lost_state_file_starts_the_count_again_without_dropping_everything_as_seen(self):
        self.chat(SID_A, [reply(NOW - HOUR, 'm1', input_tokens=3)])
        self.assertEqual(self.used(), {SID_A: 3})
        (self.sb.paths()['state'] / 'spend.json').write_text('{broken')
        self.assertEqual(self.used(), {SID_A: 3})

    def test_old_ids_leave_the_ids_file(self):
        self.sb.SPEND_IDS_TIDY = 2
        ids = self.sb.paths()['state'] / 'spend.ids'
        self.chat(SID_A, [reply(NOW - HOUR + i, f'm{i}', input_tokens=1) for i in range(3)])
        self.assertEqual(self.used(), {SID_A: 3})
        self.assertEqual(ids.stat().st_size, 24)
        later = NOW + 9 * DAY
        self.chat(SID_B, [reply(later - 60, 'x1', input_tokens=1)], mtime=later)
        self.assertEqual(self.used(since=later - DAY, now=later), {SID_B: 1})
        self.assertEqual(ids.stat().st_size, 8)

    def test_the_biggest_forty_are_named_and_the_total_has_them_all(self):
        self.sb.SPEND_TOP = 2
        for i, sid in enumerate((SID_A, SID_B, SID_C)):
            self.chat(sid, [reply(NOW - HOUR, f'm{i}', input_tokens=10 * (i + 1))])
        r = self.report()
        self.assertEqual(([c['sid'] for c in r['chats']], r['total'], r['n']), ([SID_C, SID_B], 60, 3))

    def test_no_start_asked_for_is_no_answer(self):
        self.chat(SID_A, [reply(NOW - HOUR, 'm1', input_tokens=3)])
        for ask in ({}, {'since': None}, {'since': 'x'}, {'since': True}, {'since': -5}, None):
            r = self.sb.spend_report(ask, NOW)
            self.assertEqual((r['since'], 'chats' in r), (None, False), ask)


class ListTest(SpendBase):
    def test_list_answers_only_when_asked_and_says_it_can(self):
        self.transcript(SID_A, str(self.root / 'proj'), NOW - 50)
        with open(self.proj / f'{SID_A}.jsonl', 'a') as f:
            f.write(json.dumps(reply(NOW - 100, 'm1', input_tokens=500)) + '\n')
        os.utime(self.proj / f'{SID_A}.jsonl', (NOW - 50, NOW - 50))
        self.chat(SID_B, [said(NOW - 200, 'a chat nobody lists'), reply(NOW - 100, 'm2', sid=SID_B, input_tokens=20)], mtime=NOW - 50)
        (self.sb.paths()['state'] / 'dismissed.json').write_text(json.dumps({SID_B: NOW}))
        r = self.sb.rpc({'verb': 'list'}, NOW, agents=[])
        self.assertIn('spend', r['features'])
        self.assertIsNone(r['spend'])
        self.assertFalse((self.sb.paths()['state'] / 'spend.json').exists())   # nothing is read unless asked
        r = self.sb.rpc({'verb': 'list', 'spend': {'since': WEEK}}, NOW, agents=[])
        self.assertEqual([s['sid'] for s in r['sessions']], [SID_A])
        # a chat in the list needs no name here; one the list leaves out brings its own
        self.assertEqual(r['spend']['chats'], [{'sid': SID_A, 'w': 500}, {'sid': SID_B, 'w': 20, 'title': 'a chat nobody lists'}])
        json.dumps(r)

    def test_a_count_that_fails_does_not_break_the_list(self):
        self.transcript(SID_A, str(self.root / 'proj'), NOW - 50)
        self.sb.spend_report = lambda *a, **k: 1 / 0
        r = self.sb.rpc({'verb': 'list', 'spend': {'since': WEEK}}, NOW, agents=[])
        self.assertEqual((r['ok'], r['spend'], len(r['sessions'])), (True, None, 1))


# ---------------------------------------------------------------- the server
class WeekStartTest(unittest.TestCase):
    def test_seven_days_before_the_reset(self):
        self.assertEqual(fleet.week_start(NOW + 2 * DAY, NOW), NOW - 5 * DAY)

    def test_a_reset_that_has_passed_is_where_the_new_week_began(self):
        self.assertEqual(fleet.week_start(NOW - HOUR, NOW), NOW - HOUR)
        self.assertEqual(fleet.week_start(NOW - 8 * DAY, NOW), NOW - DAY)   # nobody replied for over a week

    def test_no_reset_or_one_that_cannot_be_is_no_week(self):
        for r in (None, 0, -1, 'x', True, NOW + 8 * DAY):
            self.assertIsNone(fleet.week_start(r, NOW), r)


def answer(chats, total=None, **kw):
    rows = [dict({'sid': s, 'w': w}, **({'title': t[0]} if t else {})) for s, w, *t in chats]
    return dict({'v': 1, 'since': 1.0, 'more': False, 'at': 2.0, 'total': sum(r['w'] for r in rows) if total is None else total,
                 'n': len(rows), 'chats': rows}, **kw)


class Machines:
    """Two machines that answer `list`; what each says about its chats' use is set per test."""
    def __init__(self, clock, reset=WEEK + 7 * DAY):
        self.clock = clock
        self.reset, self.spend, self.features, self.asked, self.down = reset, {}, {'studio': ['spend'], 'laptop': ['spend']}, [], set()
        self.rows, self.clock_off = {}, {}

    def __call__(self, target, req, timeout):
        name = req['as']
        if name in self.down:
            raise OSError('unreachable')
        there = self.clock[0] + self.clock_off.get(name, 0)
        if 'spend' in req:
            self.asked.append((name, req['spend']['since']))
        return {'ok': True, 'machine': name, 'sessions': self.rows.get(name, []), 'replaced': [], 'now': there, 'clock': there,
                'limits': {'merged': True, 'seven_day': 40, 'seven_day_reset': self.reset, 'five_hour': 5, 'five_hour_reset': None, 'updated': there},
                'features': self.features[name], 'spend': self.spend.get(name) if 'spend' in req else None}


def row(sid, title, active=NOW - 60, **kw):
    return dict({'sid': sid, 'title': title, 'kind': None, 'status': None, 'cold': True, 'ctx': 1000, 'cwd': '/Users/you/Claude',
                 'folder': 'Claude', 'started': NOW - DAY, 'active': active, 'mtime': active}, **kw)


class ServerBase(unittest.TestCase):
    def setUp(self):
        self.clock = [NOW]
        self.m, self.state = Machines(self.clock), Path(mkdtemp())
        self.f = fleet.Fleet(MACH, self.m, clock=lambda: self.clock[0])
        self.app = server.App(self.f, state_dir=self.state, web_dir=self.state)

    def poll(self, n=1, step=20):
        for _ in range(n):
            self.f.poll()
            self.clock[0] += step

    def top(self):
        return self.app.get('/api/usage')['top']


class AskTest(ServerBase):
    def test_the_week_is_asked_for_once_the_machines_say_they_can_then_every_two_minutes(self):
        self.poll()   # the first answer says what the machine can do and when the week resets
        self.assertEqual(self.m.asked, [])
        self.poll()
        self.assertEqual(sorted(self.m.asked), [('laptop', WEEK), ('studio', WEEK)])
        self.m.spend = {'studio': answer([(SID_A, 10)]), 'laptop': answer([(SID_B, 5)])}
        self.m.asked.clear()
        self.poll(4)  # 80 s more: not yet
        self.assertEqual(self.m.asked, [])
        self.poll(4)  # past two minutes: again
        self.assertEqual(sorted(self.m.asked), [('laptop', WEEK), ('studio', WEEK)])

    def test_the_start_goes_out_on_the_machines_own_clock(self):
        self.m.clock_off['laptop'] = -90   # its clock is a minute and a half behind
        self.poll(2)
        self.assertEqual(dict(self.m.asked), {'laptop': WEEK - 90, 'studio': WEEK})

    def test_a_machine_still_reading_is_asked_at_every_poll_and_what_it_said_before_stands(self):
        self.m.spend = {'studio': answer([(SID_A, 10)])}
        self.poll(2)
        self.assertEqual(self.top()['total'], 10)
        self.m.spend = {'studio': {'v': 1, 'since': 1.0, 'more': True, 'at': 2.0}}
        self.clock[0] += 200
        self.m.asked.clear()
        self.poll(3)
        self.assertEqual([n for n, _ in self.m.asked].count('studio'), 3)
        self.assertEqual(self.top()['total'], 10)

    def test_an_old_helper_is_never_asked_and_counts_for_nothing(self):
        self.m.features = {'studio': [], 'laptop': []}
        self.poll(3)
        self.assertEqual(self.m.asked, [])
        t = self.top()
        self.assertEqual((t['rows'], t['total'], t['able'], t['machines'], t['pending']), ([], 0, 0, 2, 0))
        self.assertEqual(t['since'], WEEK)

    def test_before_the_weeks_reset_is_known_nobody_is_asked(self):
        self.m.reset = None
        self.poll(3)
        self.assertEqual(self.m.asked, [])
        self.assertEqual((self.top()['since'], self.top()['rows']), (None, []))


class TopTest(ServerBase):
    def test_the_biggest_first_with_their_share_of_everything_seen(self):
        self.m.rows = {'studio': [row(SID_A, 'Kitchen remodel'), row(SID_B, 'Taxes')], 'laptop': [row(SID_C, 'Trip')]}
        self.m.spend = {'studio': answer([(SID_A, 600), (SID_B, 100)], total=800, n=5), 'laptop': answer([(SID_C, 200)])}
        self.poll(2)
        t = self.top()
        self.assertEqual((t['since'], t['total'], t['chats'], t['able'], t['machines'], t['pending']), (WEEK, 1000, 6, 2, 2, 0))
        self.assertEqual(t['rows'], [
            {'machine': 'studio', 'sid': SID_A, 'name': 'Kitchen remodel', 'label': 'Studio', 'weighted': 600, 'share': 60.0, 'alive': True, 'sent': False, 'parts': 1},
            {'machine': 'laptop', 'sid': SID_C, 'name': 'Trip', 'label': 'Laptop', 'weighted': 200, 'share': 20.0, 'alive': True, 'sent': False, 'parts': 1},
            {'machine': 'studio', 'sid': SID_B, 'name': 'Taxes', 'label': 'Studio', 'weighted': 100, 'share': 10.0, 'alive': True, 'sent': False, 'parts': 1}])
        json.dumps(t)

    def test_at_most_ten_rows(self):
        sids = [f'{i:08x}-1111-2222-3333-444444444444' for i in range(1, 15)]
        self.m.spend = {'studio': answer([(s, 100 - i, f'chat {i}') for i, s in enumerate(sids)])}
        self.poll(2)
        t = self.top()
        self.assertEqual((len(t['rows']), t['chats'], t['rows'][0]['name'], t['rows'][0]['alive']), (10, 14, 'chat 0', False))

    def test_a_machine_that_is_offline_keeps_its_last_answer_for_the_week(self):
        self.m.rows = {'laptop': [row(SID_C, 'Trip')]}
        self.m.spend = {'studio': answer([(SID_A, 600, 'Kitchen')]), 'laptop': answer([(SID_C, 200)])}
        self.poll(2)
        self.m.down.add('laptop')
        self.poll(8)
        t = self.top()
        self.assertEqual([(r['sid'], r['weighted'], r['label']) for r in t['rows']], [(SID_A, 600, 'Studio'), (SID_C, 200, 'Laptop')])
        # a server that restarts while the machine is away still has it
        f2 = fleet.Fleet(MACH, self.m, clock=lambda: self.clock[0])
        app2 = server.App(f2, state_dir=self.state, web_dir=self.state)
        f2.poll()
        f2.poll()
        self.assertEqual(sorted((r['sid'], r['weighted']) for r in app2.get('/api/usage')['top']['rows']), [(SID_A, 600), (SID_C, 200)])

    def test_two_machines_answering_at_once_are_both_kept(self):
        # The first writer is held up (a slow disk) while the second answers: the file still ends with both.
        import threading, time
        from unittest import mock
        real, first, n = fleet.durable.put, threading.Event(), [0]

        def slow(path, data, **kw):
            n[0] += 1
            if n[0] == 1:
                first.set()
                time.sleep(0.3)
            return real(path, data, **kw)
        self.m.spend = {'studio': answer([(SID_A, 600)]), 'laptop': answer([(SID_C, 200)])}
        self.poll(2)  # the week is known and both were asked
        f = self.app.fleet
        with mock.patch.object(fleet.durable, 'put', slow):
            a = threading.Thread(target=f._spend_got, args=('studio', answer([(SID_A, 601)]), self.clock[0]))
            a.start()
            self.assertTrue(first.wait(5))
            f._spend_got('laptop', answer([(SID_C, 201)]), self.clock[0])
            a.join()
        kept = json.loads(f.spend_file.read_text())
        self.assertEqual({k: v['total'] for k, v in kept.items()}, {k: v['total'] for k, v in f.spend.items()})
        self.assertEqual(sorted(kept), ['laptop', 'studio'])

    def test_the_weekly_reset_drops_what_an_absent_machine_said_about_last_week(self):
        self.m.spend = {'studio': answer([(SID_A, 600, 'Kitchen')]), 'laptop': answer([(SID_C, 200, 'Trip')])}
        self.poll(2)
        self.m.down.add('laptop')
        self.clock[0] = WEEK + 7 * DAY + 400   # the reset has passed: the new week began there, before any new number
        self.m.asked.clear()
        self.m.spend['studio'] = answer([(SID_A, 7, 'Kitchen')])
        self.poll(2)
        self.assertEqual(self.m.asked[-1], ('studio', WEEK + 7 * DAY))
        t = self.top()
        self.assertEqual(([(r['sid'], r['weighted'], r['share']) for r in t['rows']], t['total']), ([(SID_A, 7, 100.0)], 7))
        # the next number arrives with a reset a week on: the same week, nothing asked anew for it
        self.m.reset += 7 * DAY
        self.poll(2)
        self.assertEqual(self.top()['since'], WEEK + 7 * DAY)
        self.assertEqual(self.top()['total'], 7)

    def test_a_thread_is_one_row_under_the_chat_the_apps_list(self):
        # SID_A was started fresh as SID_B (the machine's replaced mark says so): one chat to the person
        self.m.rows = {'studio': [row(SID_B, 'Kitchen remodel (fresh)', started=NOW - HOUR)]}
        self.m.spend = {'studio': answer([(SID_A, 600, 'Kitchen remodel'), (SID_B, 150), (SID_C, 250, 'Something else')])}
        real = self.m.__call__

        def transport(target, req, timeout):
            r = real(target, req, timeout)
            if req['as'] == 'studio':
                r['replaced'] = [dict(row(SID_A, 'Kitchen remodel', active=NOW - 2 * HOUR, started=NOW - 2 * DAY), how='fresh',
                                      by={'machine': 'studio', 'sid': SID_B})]
            return r
        self.f.transport = transport
        self.poll(2)
        self.app.feat.threads.observe(self.f.snapshot()['machines'])
        t = self.top()
        self.assertEqual([(r['sid'], r['name'], r['weighted'], r['share'], r['alive'], r['parts']) for r in t['rows']],
                         [(SID_B, 'Kitchen remodel (fresh)', 750, 75.0, True, 2), (SID_C, 'Something else', 250, 25.0, False, 1)])
        self.assertEqual(t['chats'], 2)

    def test_a_chat_no_machine_lists_now_keeps_the_name_it_had(self):
        self.m.rows = {'studio': [row(SID_A, 'Kitchen remodel')]}
        self.m.spend = {'studio': answer([(SID_A, 600)])}
        self.poll(2)
        self.app.feat.threads.observe(self.f.snapshot()['machines'])
        self.m.rows = {}   # stopped and deleted: the helper still has its week, the server its name
        self.poll(2)
        r = self.top()['rows'][0]
        self.assertEqual((r['name'], r['alive'], r['weighted']), ('Kitchen remodel', False, 600))

    def test_a_machine_that_has_not_finished_its_first_count_is_pending(self):
        self.m.spend = {'studio': {'v': 1, 'since': 1.0, 'more': True, 'at': 2.0}}
        self.poll(3)
        t = self.top()
        self.assertEqual((t['rows'], t['able'], t['pending']), ([], 2, 2))

    def test_an_answer_of_the_wrong_shape_is_left_out(self):
        self.assertIsNone(fleet.clean_spend('x'))
        self.assertIsNone(fleet.clean_spend({'total': 'lots', 'chats': []}))
        self.assertEqual(fleet.clean_spend({'more': True}), {'more': True})
        got = fleet.clean_spend({'total': 5, 'n': 'x', 'chats': [{'sid': SID_A, 'w': 10, 'title': ' a\n b '}, {'sid': SID_A, 'w': 3},
                                                              {'sid': 7, 'w': 3}, {'sid': SID_B, 'w': -1}, 'x', {'sid': SID_C, 'w': True}]})
        self.assertEqual(got, {'total': 10, 'n': 1, 'chats': [{'sid': SID_A, 'w': 10, 'title': 'a b'}]})

    def test_a_sent_task_is_marked(self):
        self.m.rows = {'studio': [row(SID_A, 'Fix the build')]}
        self.m.spend = {'studio': answer([(SID_A, 600)])}
        self.poll(2)
        view = self.f.spend_view()
        listed = {f'studio/{SID_A}': dict(row(SID_A, 'Fix the build'), sent={'machine': 'laptop', 'label': 'Laptop'})}
        self.assertTrue(fleet.top_chats(view, listed)['rows'][0]['sent'])
        self.assertTrue(fleet.top_chats(view, {}, sent=lambda k: k == f'studio/{SID_A}')['rows'][0]['sent'])
        self.assertFalse(fleet.top_chats(view, {})['rows'][0]['sent'])


if __name__ == '__main__':
    unittest.main()
