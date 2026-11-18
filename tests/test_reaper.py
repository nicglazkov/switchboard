"""Cleanup (was the reaper): which background chats it suggests stopping, what it never suggests, the once-a-day
notice, and that only a person's click stops anything."""
import json, os, subprocess, sys, threading, unittest
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from tempfile import mkdtemp
sys.path.insert(0, str(Path(__file__).resolve().parents[1] / 'server'))
import features, fleet, server  # noqa: E402
from tests.test_server import MACH  # noqa: E402

NOW = 2_000_000_000.0
H = 3600


def sid(n):
    return f'{n:08x}-1111-2222-3333-444444444444'


def bg(n, status='idle', quiet_h=0.0, kind='background', bg_id=True, **kw):
    t = NOW - quiet_h * H
    d = {'sid': sid(n), 'title': f'chat {n}', 'kind': kind, 'status': status, 'bg_id': sid(n)[:8] if bg_id else None,
         'mtime': t, 'active': t, 'ctx': 1000, 'folder': 'p', 'cwd': '/p'}
    d.update(kw)
    return d


class Machine:
    def __init__(self):
        self.sessions, self.replaced, self.mem, self.calls, self.fail_stop = {}, {}, {}, [], False
        self.features, self.in_use = (), set()  # what the helper says it can do; chats it finds something running in
        self.stop_lies = False  # the stop goes through, but its answer says it did not (or never arrives)

    def __call__(self, target, req, timeout):
        name = 'desk' if target == 'me@x' else 'tower'
        self.calls.append((name, req))
        v = req['verb']
        if v == 'list':
            return {'ok': True, 'sessions': self.sessions.get(name, []), 'replaced': self.replaced.get(name, []),
                    'mem': self.mem.get(name), 'limits': None, 'features': list(self.features)}
        if v == 'stop':
            if self.fail_stop:
                return {'ok': False, 'error': 'claude stop failed'}
            if req.get('if_idle') and req['sid'] in self.in_use:
                return {'ok': False, 'error': '1 background command is still running.'}
            for s in self.sessions.get(name, []) + self.replaced.get(name, []):
                if s['sid'] == req['sid']:
                    s['status'] = 'stopped'
            if self.stop_lies:
                return {'ok': False, 'error': 'Claude Code could not confirm it stopped. Try again in a moment.'}
            return {'ok': True}
        if v == 'reap_orphans':
            return {'ok': True, 'killed': [{'pid': p, 'mb': 410, 'age_h': 30.0} for p in req.get('pids') or [] if p == 501],
                    'failed': []}
        return {'ok': True}


class CleanupTest(unittest.TestCase):
    def make(self, cfg=None, opened=()):
        self.m, self.pushes = Machine(), []
        self.state, self.cfg = Path(mkdtemp()), Path(mkdtemp())
        if cfg is not None:
            (self.cfg / 'reaper.json').write_text(cfg if isinstance(cfg, str) else json.dumps(cfg))
        self.now = [NOW]
        app = server.App(fleet.Fleet(MACH, self.m, clock=lambda: self.now[0]), state_dir=self.state, web_dir=self.state,
                         clock=lambda: self.now[0], cfg_dir=self.cfg, pusher=lambda *a: self.pushes.append(a[:3]))
        app._refresh_soon = lambda *a, **k: None
        app.feat.reaper.opened = lambda: set(opened)
        self.app = app
        return app

    def calls(self, verb):
        return [r for n, r in self.m.calls if r['verb'] == verb]

    def test_the_rules(self):
        app = self.make()
        self.m.sessions['desk'] = [
            bg(1, 'idle', 80, mb=365), bg(2, 'waiting', 75), bg(3, 'idle', 70),          # idle: three days at least
            bg(4, 'busy', 13), bg(5, 'busy', 11),                                         # stalled: 12 h busy, nothing written
            bg(6, 'idle', 999, kind='interactive'), bg(7, 'stopped', 99),                 # a terminal chat / already stopped
            bg(8, 'idle', 99, bg_id=False), bg(9, None, 99),                              # no bg id / status unknown
            bg(12, 'busy', 0, archived=NOW - 2 * H, last_ts=NOW - 2.5 * H, mb=432),      # archived in the Claude app
            bg(13, 'idle', 200, child_ts=NOW - 60),                                       # its subagent is working
            bg(14, 'idle', 0, last_ts=NOW - 100 * H),                                     # bookkeeping lines are not use
            bg(15, 'busy', 0, archived=NOW - 2 * H, last_ts=NOW - 10),                   # used again after it was archived
        ]
        self.m.replaced['desk'] = [bg(10, 'idle', 1.5), bg(11, 'idle', 0.5)]  # replaced: idle an hour
        app.fleet.poll()
        got = {d['sid']: d['reason'] for d in app.feat.reaper.decisions()}
        self.assertEqual(got, {sid(1): 'idle', sid(2): 'idle', sid(4): 'stalled', sid(10): 'replaced', sid(12): 'archived',
                               sid(14): 'idle'})
        ds = app.feat.reaper.decisions()
        self.assertEqual([d['sid'] for d in ds[:2]], [sid(12), sid(1)], 'the most memory first')
        d = next(d for d in ds if d['sid'] == sid(1))
        self.assertEqual((d['machine'], d['label'], d['bg_id'], d['idle_h'], d['mb'], d['why']),
                         ('desk', 'Desk', sid(1)[:8], 80, 365, 'Idle 3 days'))
        self.assertEqual(next(d for d in ds if d['sid'] == sid(14))['why'], 'Idle 4 days')

    def test_why_texts(self):
        os.environ['SWITCHBOARD_TZ'] = 'America/Los_Angeles'
        archived = 1790713899.0  # 2026-09-29 20:11:39 UTC
        self.assertEqual(features.why_text('archived', {'archived': archived}, archived + 86400 * 13),
                         'Archived in the Claude app Sep 29')
        self.assertEqual(features.span(19 * 86400 + 5), '19 days')
        self.assertEqual(features.span(3600), '1 hour')
        self.assertEqual(features.mb_text(797), '797 MB')
        self.assertEqual(features.mb_text(2048), '2.0 GB')

    def test_config_file(self):
        app = self.make({'idle_h': 48, 'dry_run': False})
        c = app.feat.reaper.config()
        self.assertEqual((c['enabled'], c['idle_h'], c['stalled_h']), (True, 72, 12), 'never under three days')
        self.assertNotIn('dry_run', c)
        app = self.make('{not json')
        self.assertFalse(app.feat.reaper.config()['enabled'], 'a broken reaper.json must not suggest anything')
        app = self.make({'enabled': False})
        self.m.sessions['desk'] = [bg(1, 'idle', 99)]
        app.fleet.poll()
        self.assertEqual(app.feat.reaper.decisions(), [])
        self.assertEqual(app.get('/api/cleanup')['count'], 0)

    def test_a_suggestion_never_stops_by_itself(self):
        # What does stop by itself is AutoStopTest's: a finished, cold chat, checked by its own machine. Not these
        # (no last message known, one still busy, a bare process), and never on a helper that cannot check.
        app = self.make({'enabled': True, 'dry_run': False})
        self.m.sessions['desk'] = [bg(1, 'idle', 99, mb=365), bg(2, 'busy', 30, mb=432)]
        self.m.mem['desk'] = {'claude_mb': 3100, 'procs': 7, 'orphans': [{'pid': 501, 'mb': 410, 'age_h': 30.0}]}
        app.fleet.poll()
        for _ in range(3):
            app.feat.after_poll()
        self.assertEqual(self.calls('stop'), [])
        self.assertEqual(self.calls('reap_orphans'), [])

    def test_one_notice_a_day_for_new_ones(self):
        app = self.make()
        self.m.sessions['desk'] = [bg(1, 'idle', 99, mb=365), bg(2, 'busy', 30, mb=432)]
        app.fleet.poll()
        app.feat.after_poll()
        self.assertEqual(self.pushes, [("2 chats aren't in use", "They're using 797 MB. Tap to take a look. These stop only if you say so.",
                                        '/?cleanup=1')])
        app.feat.after_poll()
        self.assertEqual(len(self.pushes), 1, 'the same ones again: no notice')
        self.now[0] += 3 * H
        self.m.sessions['desk'].append(bg(3, 'idle', 99))
        app.fleet.poll()
        app.feat.after_poll()
        self.assertEqual(len(self.pushes), 1, 'a new one within the day waits')
        self.now[0] += 22 * H
        app.fleet.poll()
        app.feat.after_poll()
        self.assertEqual(len(self.pushes), 1, 'nothing new since the last look')
        self.m.sessions['desk'].append(bg(4, 'idle', 99))
        app.fleet.poll()
        app.feat.after_poll()
        self.assertEqual(len(self.pushes), 2)
        self.assertTrue(self.pushes[1][0].startswith('4 chats'))
        self.assertEqual(self.calls('stop'), [])

    def test_stop_on_click(self):
        app = self.make()
        self.m.sessions['desk'] = [bg(1, 'idle', 99, mb=365), bg(2, 'idle', 1)]
        app.fleet.poll()
        lists = len(self.calls('list'))
        r = app.post('/api/cleanup', {'action': 'stop', 'items': [{'machine': 'desk', 'sid': sid(1)}]})
        self.assertEqual((r['ok'], r['stopped'], r['skipped'], r['failed']), (True, [f'desk/{sid(1)}'], [], []))
        self.assertEqual([c['sid'] for c in self.calls('stop')], [sid(1)])
        self.assertGreater(len(self.calls('list')), lists, 'the machine is asked again before anything stops')
        log = json.loads((self.state / 'reaped.json').read_text())
        self.assertEqual((log[-1]['sid'], log[-1]['by'], log[-1]['reason'], log[-1]['mb']), (sid(1), 'user', 'idle', 365))
        app.fleet.poll()
        snap = app.sessions()
        s = next(s for s in snap['machines'][0]['sessions'] if s['sid'] == sid(1))
        self.assertEqual((s['status'], s['reaped']['reason']), ('stopped', 'idle'))
        self.assertEqual([x['sid'] for x in snap['cleanup']['stopped']], [sid(1)], 'stopped ones stay on the card, with Resume')
        self.assertEqual(snap['cleanup']['count'], 0)

    def test_a_chat_used_since_is_skipped(self):
        app = self.make()
        self.m.sessions['desk'] = [bg(1, 'idle', 99)]
        app.fleet.poll()
        self.m.sessions['desk'] = [bg(1, 'busy', 0)]  # someone typed into it a moment ago
        r = app.post('/api/cleanup', {'action': 'stop', 'items': [{'machine': 'desk', 'sid': sid(1)}]})
        self.assertEqual((r['stopped'], r['skipped']), ([], [f'desk/{sid(1)}']))
        self.assertEqual(self.calls('stop'), [])

    def test_a_stop_that_fails(self):
        app = self.make()
        self.m.sessions['desk'] = [bg(1, 'idle', 99)]
        self.m.fail_stop = True
        app.fleet.poll()
        r = app.post('/api/cleanup', {'action': 'stop', 'items': [{'machine': 'desk', 'sid': sid(1)}]})
        self.assertIn('claude stop failed', r['failed'][0]['error'])
        log = json.loads((self.state / 'reaped.json').read_text())
        self.assertIn('claude stop failed', log[0]['error'])
        self.assertNotIn('reaped', app.sessions()['machines'][0]['sessions'][0])

    def test_bad_requests(self):
        app = self.make()
        bad = [{'action': 'stop'}, {'action': 'stop', 'items': []}, {'action': 'stop', 'items': [{'machine': 'nope', 'sid': sid(1)}]},
               {'action': 'stop', 'items': [{'machine': 'desk', 'sid': '../x'}]},
               {'action': 'stop', 'items': [{'machine': 'desk', 'sid': sid(1), 'pid': 5}]},
               {'action': 'stop', 'items': [{'machine': 'desk', 'pid': 1}]}, {'action': 'stop', 'items': [{'machine': 'desk'}]},
               {'action': 'stop', 'items': [{'machine': 'desk', 'pid': True}]},
               {'action': 'keep', 'machine': 'desk'}, {'action': 'keep', 'machine': 'desk', 'sid': 'x; rm'},
               {'action': 'nuke'}]
        for b in bad:
            with self.assertRaises(server.HttpError, msg=b) as cm:
                app.post('/api/cleanup', b)
            self.assertEqual(cm.exception.code, 400, b)
        self.assertEqual(self.calls('stop') + self.calls('reap_orphans'), [])

    def test_keep_and_open_terminals_and_offline_are_never_suggested(self):
        app = self.make(opened=[('desk', sid(1)[:8])])
        self.m.sessions['desk'] = [bg(1, 'idle', 99), bg(2, 'idle', 99), bg(5, 'idle', 99)]
        self.m.sessions['tower'] = [bg(3, 'idle', 99)]
        self.m.mem['desk'] = {'orphans': [{'pid': 501, 'mb': 410, 'age_h': 30.0}]}
        app.fleet.poll()
        self.assertEqual(app.post('/api/cleanup', {'action': 'keep', 'machine': 'desk', 'sid': sid(2)}), {'ok': True, 'keep': True})
        app.post('/api/action', {'machine': 'desk', 'verb': 'keep', 'sid': sid(5)})
        app.post('/api/cleanup', {'action': 'keep', 'machine': 'desk', 'pid': 501})
        app.fleet.state['tower']['online'] = False
        self.assertEqual(app.feat.reaper.decisions(), [])
        self.assertTrue(next(s for s in app.sessions()['machines'][0]['sessions'] if s['sid'] == sid(2))['keep'])

    def test_a_chat_open_in_an_apps_tab_is_never_suggested_connected_or_not(self):
        app = self.make()
        self.m.sessions['desk'] = [bg(1, 'idle', 99), bg(2, 'idle', 99)]
        app.fleet.poll()
        self.assertEqual(app.post('/api/tabs', {'device': 'studio', 'tabs': [f'desk:{sid(1)}']}), {'ok': True, 'tabs': 1})
        self.assertEqual([d['sid'] for d in app.feat.reaper.decisions()], [sid(2)])
        r = app.post('/api/cleanup', {'action': 'stop', 'items': [{'machine': 'desk', 'sid': sid(1)}], 'when': 'now'})
        self.assertEqual((r['stopped'], r['skipped']), ([], [f'desk/{sid(1)}']))
        # the tab closed: the app sends its new list, and the chat can be suggested again
        app.post('/api/tabs', {'device': 'studio', 'tabs': []})
        self.assertEqual(len(app.feat.reaper.decisions()), 2)
        # another app's tabs count too; one that has not reported in 30 days does not
        app.post('/api/tabs', {'device': 'laptop', 'tabs': [f'desk/{sid(2)}']})
        self.assertEqual([d['sid'] for d in app.feat.reaper.decisions()], [sid(1)])
        self.now[0] += 31 * 86400
        self.m.sessions['desk'] = [bg(1, 'idle', 99 + 31 * 24), bg(2, 'idle', 99 + 31 * 24)]
        app.fleet.poll()
        self.assertEqual(len(app.feat.reaper.decisions()), 2)
        self.assertEqual(app.post('/api/tabs', {'device': 'studio', 'tabs': ['desk:../x', f'desk:{sid(1)}']})['tabs'], 1)
        for b in ({'device': 'Bad!', 'tabs': []}, {'device': 'studio', 'tabs': 'x'}):
            with self.assertRaises(server.HttpError) as cm:
                app.post('/api/tabs', b)
            self.assertEqual(cm.exception.code, 400, b)

    def test_a_hub_that_does_not_answer_forgets_no_open_terminal(self):
        # Audit, server 18: while the hub restarted, "which terminals are open" came back empty, and a chat open in
        # a terminal was offered for stopping.
        app = self.make()
        self.m.sessions['desk'] = [bg(1, 'idle', 99), bg(2, 'idle', 99)]
        app.fleet.poll()
        r, said = app.feat.reaper, [{('desk', sid(1)[:8])}]
        r.opened = lambda: said[0]
        again = lambda: setattr(r, '_open', (0, r._open[1]))  # the 10 s between two questions have passed
        self.assertEqual([d['sid'] for d in r.decisions()], [sid(2)])
        said[0] = None
        again()
        self.assertEqual([d['sid'] for d in r.decisions()], [sid(2)], 'no answer: what it said last still counts')
        r._open_at -= features.OPEN_TRUST + 1
        again()
        self.assertEqual(len(r.decisions()), 2, 'but not for ever')
        said[0] = set()
        again()
        self.assertEqual(len(r.decisions()), 2)
        self.assertIsNone(features.hub_open_known('http://127.0.0.1:9/internal/open', timeout=0.5))

    def test_an_app_gone_for_a_month_is_dropped_from_the_tabs_file(self):
        app = self.make()
        app.post('/api/tabs', {'device': 'laptop', 'tabs': [f'desk/{sid(2)}']})
        (self.state / 'tabs.json').write_text(json.dumps(dict(json.loads((self.state / 'tabs.json').read_text()), odd='x')))
        self.now[0] += 31 * 86400
        app.post('/api/tabs', {'device': 'studio', 'tabs': []})
        self.assertEqual(list(json.loads((self.state / 'tabs.json').read_text())), ['studio'])

    def test_the_notice_is_written_down_before_it_is_sent(self):
        app = self.make()
        self.m.sessions['desk'] = [bg(1, 'idle', 99)]
        app.fleet.poll()
        at_push = []
        app._push = lambda *a: at_push.append(json.loads(app.feat.reaper.seen_path.read_text()).get('pushed'))
        app.feat.after_poll()
        self.assertEqual(at_push, [self.now[0]])

    def test_star_toggles_and_shows_on_the_row(self):
        app = self.make()
        self.m.sessions['desk'] = [bg(1, 'idle', 1), bg(2, 'idle', 1)]
        app.fleet.poll()
        self.assertEqual(app.post('/api/action', {'machine': 'desk', 'verb': 'star', 'sid': sid(1)})['star'], True)
        rows = {s['sid']: s for s in app.sessions()['machines'][0]['sessions']}
        self.assertTrue(rows[sid(1)].get('star'))
        self.assertNotIn('star', rows[sid(2)])
        self.assertEqual(app.post('/api/action', {'machine': 'desk', 'verb': 'star', 'sid': sid(1), 'on': False})['star'], False)
        self.assertNotIn('star', next(s for s in app.sessions()['machines'][0]['sessions'] if s['sid'] == sid(1)))

    def test_orphans(self):
        app = self.make()
        self.m.mem['desk'] = {'claude_mb': 3100, 'procs': 7, 'orphans': [{'pid': 501, 'mb': 410, 'age_h': 30.0},
                                                                          {'pid': 502, 'mb': 90, 'age_h': 2.0}]}
        app.fleet.poll()
        self.assertEqual(app.sessions()['machines'][0]['mem']['claude_mb'], 3100)
        ds = app.feat.reaper.decisions()
        self.assertEqual([(d['pid'], d['reason'], d['mb']) for d in ds], [(501, 'orphan', 410)], 'young ones are left alone')
        app.feat.after_poll()
        self.assertEqual(self.calls('reap_orphans'), [])
        r = app.post('/api/cleanup', {'action': 'stop', 'items': [{'machine': 'desk', 'pid': 501}, {'machine': 'desk', 'pid': 502}]})
        self.assertEqual(self.calls('reap_orphans'), [{'verb': 'reap_orphans', 'min_age_h': 6, 'pids': [501]}])
        self.assertEqual((r['stopped'], r['skipped']), (['desk/pid:501'], ['desk/pid:502']))

    def test_report_and_snapshot(self):
        app = self.make()
        self.m.sessions['desk'] = [bg(1, 'idle', 99, mb=365), bg(2, 'idle', 1)]
        app.fleet.poll()
        rep = app.get('/api/cleanup')
        self.assertEqual((rep['count'], rep['mb'], rep['items'][0]['sid']), (1, 365, sid(1)))
        self.assertEqual(app.get('/api/reaper')['count'], 1, 'the old address still answers')
        snap = app.sessions()
        self.assertEqual((snap['cleanup']['count'], snap['cleanup']['mb']), (1, 365))
        s1, s2 = snap['machines'][0]['sessions']
        self.assertEqual(s1['cleanup'], {'reason': 'idle', 'why': 'Idle 4 days'})
        self.assertNotIn('cleanup', s2)


def cold(n, quiet_h, **kw):
    """A background chat that finished its turn quiet_h hours ago and whose cache ran out an hour after that."""
    t = NOW - quiet_h * H
    return bg(n, kw.pop('status', 'idle'), quiet_h, **dict({'last_ts': t, 'state': 'done', 'cache_expires_at': t + H, 'mb': 600}, **kw))


class AutoStopTest(unittest.TestCase):
    """"Stop idle chats": a finished, cold chat left alone for hours stops by itself, and nothing else ever does."""
    make, calls = CleanupTest.make, CleanupTest.calls

    def auto(self, prefs=None, features=('busy', 'stop_if_idle'), **kw):
        app = self.make(**kw)
        self.m.features = features
        if prefs is not None:
            app.post('/api/prefs', prefs)
        return app

    def look(self, app, later=features.AUTO_EVERY + 1):
        self.now[0] += later
        app.fleet.poll()
        app.feat.after_poll()
        return [c['sid'] for c in self.calls('stop')]

    def test_an_idle_cold_chat_stops_after_the_wait_and_says_so(self):
        app = self.auto()
        self.assertEqual(app.prefs()['auto_stop_h'], 6, 'on, six hours, with no setting touched')
        self.m.sessions['desk'] = [cold(1, 7), cold(2, 5)]
        self.assertEqual(self.look(app), [sid(1)])
        self.assertEqual(self.calls('stop'), [{'verb': 'stop', 'sid': sid(1), 'if_idle': True, 'idle_s': 6 * H}],
                         'the machine has the last word, and is told the wait so it can check it on its own clock')
        self.assertEqual(self.pushes, [], 'quietly')
        log = json.loads((self.state / 'reaped.json').read_text())
        self.assertEqual((log[-1]['sid'], log[-1]['by'], log[-1]['reason'], log[-1]['idle_h']), (sid(1), 'auto', 'idle', 7))
        snap = app.sessions()
        row = next(x for x in snap['machines'][0]['sessions'] if x['sid'] == sid(1))
        self.assertEqual((row['status'], row['reaped']['reason'], row['reaped']['by']), ('stopped', 'idle', 'auto'))
        self.assertEqual([(x['sid'], x['by']) for x in snap['cleanup']['stopped']], [(sid(1), 'auto')], 'listed with Resume')
        self.assertEqual(snap['prefs']['auto_stop_h'], 6)
        self.assertEqual(self.look(app), [sid(1)], 'and only once')

    def test_what_never_stops_by_itself(self):
        app = self.auto(opened=[('desk', sid(20)[:8])])
        t = NOW - 30 * H
        rows = [
            cold(1, 30, status='busy'), cold(2, 30, status='waiting'),          # working / a permission prompt
            cold(3, 30, state='blocked'), cold(4, 30, needs='say which one'),   # it asked for something
            cold(5, 30, turn_open=True), cold(6, 30, child_ts=NOW - 60),        # a turn or a subagent still going
            cold(7, 30, cache_expires_at=NOW + 600),                            # warm: stopping would cost a re-read
            bg(8, 'idle', 30, last_ts=t, state='done'),                         # cache not known
            cold(9, 30, kind='interactive'), cold(10, 30, bg_id=False),         # a terminal chat
            cold(11, 30), cold(12, 30), cold(13, 30), cold(14, 30),             # starred, kept, in a tab, held
            cold(20, 30),                                                       # open in a terminal on the hub
            cold(21, 30, status='busy', last_ts=NOW - 13 * H),                  # "stalled" is a suggestion only
        ]
        self.m.sessions['desk'] = rows
        self.m.mem['desk'] = {'orphans': [{'pid': 501, 'mb': 410, 'age_h': 30.0}]}
        app.fleet.poll()
        app.post('/api/action', {'machine': 'desk', 'verb': 'star', 'sid': sid(11)})
        app.post('/api/action', {'machine': 'desk', 'verb': 'keep', 'sid': sid(12)})
        app.post('/api/tabs', {'device': 'studio', 'tabs': [f'desk:{sid(13)}']})
        app.guard.held = lambda machine, s: s == sid(14)
        for _ in range(4):
            self.assertEqual(self.look(app), [])
        self.assertEqual(self.calls('reap_orphans'), [])

    def test_off_or_an_older_helper_or_a_silent_hub_stops_nothing(self):
        for kw in ({'prefs': {'auto_stop_h': 0}}, {'features': ('busy',)}, {'cfg': {'enabled': False}}):
            app = self.auto(**kw)
            self.m.sessions['desk'] = [cold(1, 99)]
            self.assertEqual(self.look(app), [], kw)
        app = self.auto()
        app.feat.reaper.opened = lambda: None  # the hub does not answer: a terminal may be open on that chat
        self.m.sessions['desk'] = [cold(1, 99)]
        self.assertEqual(self.look(app), [])
        self.m.sessions['tower'] = [cold(2, 99)]
        app = self.auto()
        self.m.sessions['desk'] = []
        app.fleet.poll()
        app.fleet.state['tower']['online'] = False
        app.feat.after_poll()
        self.assertEqual(self.calls('stop'), [], 'an offline machine')

    def test_the_wait_is_the_setting(self):
        app = self.auto({'auto_stop_h': 24})
        self.m.sessions['desk'] = [cold(1, 7)]
        self.assertEqual(self.look(app), [])
        self.assertEqual(app.post('/api/prefs', {'auto_stop_h': 2})['auto_stop_h'], 2)
        self.assertEqual(self.look(app), [sid(1)])
        for bad in (1, -3, 'soon', True, 100000, None):
            with self.assertRaises(server.HttpError, msg=bad) as cm:
                app.post('/api/prefs', {'auto_stop_h': bad})
            self.assertEqual(cm.exception.code, 400, bad)
        self.assertEqual(app.prefs()['auto_stop_h'], 2)
        self.assertEqual([features.auto_hours(v) for v in (None, 'x', True, 1, 0, 12, 9999)], [6, 6, 6, 6, 0, 12, 6])

    def test_a_machine_that_finds_something_running_is_believed_and_not_asked_again_at_once(self):
        app = self.auto()
        self.m.sessions['desk'] = [cold(1, 9)]
        self.m.in_use = {sid(1)}
        self.assertEqual(len(self.look(app)), 1, 'asked once')
        self.assertNotIn('status', [x.get('status') for x in self.m.sessions['desk'] if x['status'] == 'stopped'])
        self.assertFalse((self.state / 'reaped.json').exists(), 'nothing is written down as stopped')
        self.assertNotIn('reaped', app.sessions()['machines'][0]['sessions'][0])
        self.assertEqual(len(self.look(app)), 1, 'left alone for a while')
        self.m.in_use = set()
        self.assertEqual(len(self.look(app, features.AUTO_RETRY + 1)), 2, 'then asked again')

    def test_a_chat_used_between_the_two_looks_is_left(self):
        app = self.auto()
        self.m.sessions['desk'] = [cold(1, 9)]
        app.fleet.poll()
        self.m.sessions['desk'] = [cold(1, 0, status='busy')]  # the machine's fresh list, asked just before the stop
        self.now[0] += features.AUTO_EVERY + 1
        app.feat.after_poll()
        self.assertEqual(self.calls('stop'), [])

    def test_a_task_someone_waits_on_and_a_job_on_its_way_are_left(self):
        app = self.auto()
        self.m.sessions['desk'] = [cold(1, 9), cold(2, 9), cold(3, 9)]
        app.feat.dispatch.d['d-1'] = {'machine': 'desk', 'sid': sid(1), 'state': 'working', 'created': NOW}
        app.feat.dispatch.d['d-2'] = {'machine': 'other', 'sid': sid(9), 'state': 'working', 'created': NOW,
                                      'from': {'machine': 'desk', 'sid': sid(3)}}  # the chat that sent a task and waits for it
        app.jobs['s-1'] = {'kind': 'resume', 'machine': 'desk', 'state': 'queued', 'plan': {'req': {'verb': 'resume', 'sid': sid(2)}}}
        self.assertEqual(self.look(app), [])
        app.feat.dispatch.d['d-1']['state'] = 'done'
        app.jobs['s-1'].update(state='done', finished=self.now[0])
        self.assertEqual(self.look(app), [sid(1)], 'a chat just resumed was just used: its transcript says nothing of it')
        app.feat.dispatch.d['d-2']['state'] = 'failed'
        self.assertEqual(self.look(app)[1:], [sid(3)])

    def test_a_chat_resumed_or_relaunched_is_left_for_the_whole_wait_again(self):
        for kind in ('resume', 'relaunch', 'start'):
            app = self.auto()
            self.m.sessions['desk'] = [cold(1, 9)]
            job = {'kind': kind, 'machine': 'desk', 'state': 'done', 'finished': self.now[0], 'plan': {'req': {'verb': kind}}}
            job.update({'sid': sid(1)} if kind == 'start' else {'plan': {'req': {'verb': kind, 'sid': sid(1)}}})
            app.jobs['s-1'] = job
            self.assertEqual(self.look(app), [], kind)
            del app.jobs['s-1']  # the server forgets a finished job after an hour; the chat is still left
            self.assertEqual(self.look(app, 5 * H), [], kind)
            self.assertEqual(self.look(app, H), [sid(1)], kind)
        app = self.auto()
        self.m.sessions['desk'] = [cold(1, 9), cold(2, 9)]
        app.jobs['s-1'] = {'kind': 'stop', 'machine': 'desk', 'state': 'error', 'finished': self.now[0],
                           'plan': {'req': {'verb': 'stop', 'sid': sid(1)}}}  # a stop that did not happen is not use
        app.jobs['s-2'] = {'kind': 'resume', 'machine': 'desk', 'state': 'error', 'finished': self.now[0],
                           'plan': {'req': {'verb': 'resume', 'sid': sid(2)}}}  # nor a resume that failed
        self.assertEqual(sorted(self.look(app)), [sid(1), sid(2)])

    def test_a_stop_that_went_through_but_said_it_failed_is_still_written_down(self):
        app = self.auto()
        self.m.sessions['desk'] = [cold(1, 9)]
        self.m.stop_lies = True
        self.assertEqual(self.look(app), [sid(1)])
        log = json.loads((self.state / 'reaped.json').read_text())
        self.assertEqual((log[-1]['sid'], log[-1]['by'], log[-1]['reason']), (sid(1), 'auto', 'idle'))
        row = app.sessions()['machines'][0]['sessions'][0]
        self.assertEqual((row['status'], row['reaped']['by']), ('stopped', 'auto'), 'its card says why it stopped')

    def test_replaced_and_archived_chats_wait_as_long_as_the_setting(self):
        app = self.auto({'auto_stop_h': 24})
        self.m.sessions['desk'] = [cold(1, 9, archived=NOW - 8 * H)]
        self.m.replaced['desk'] = [cold(2, 9)]
        self.assertEqual(self.look(app), [])
        self.assertEqual(sorted(self.look(app, 16 * H)), [sid(1), sid(2)])
        self.assertEqual({c['idle_s'] for c in self.calls('stop')}, {24 * H})

    def test_a_few_at_a_time_and_replaced_and_archived_ones_too(self):
        app = self.auto()
        self.m.sessions['desk'] = [cold(n, 9, mb=100 * n) for n in (1, 2, 3)] + [cold(4, 7, archived=NOW - 6.5 * H)]
        self.m.replaced['desk'] = [cold(5, 7, mb=50), cold(6, 5, mb=50)]
        self.assertEqual(self.look(app), [sid(4), sid(3)], 'two in one look, the most memory first')
        self.assertEqual(sorted(self.look(app)[2:]), [sid(1), sid(2)])
        self.assertEqual(self.look(app)[4:], [sid(5)], 'a replaced one, but not inside the wait')

    def test_a_chat_stopped_by_itself_stays_on_the_board_as_long_as_a_running_one(self):
        app = self.auto()
        self.m.sessions['desk'] = [cold(1, 60), cold(2, 60, status='stopped')]
        self.assertEqual(self.look(app), [sid(1)])
        rows = {x['sid']: x for x in app.sessions()['machines'][0]['sessions']}
        self.assertEqual((rows[sid(1)]['status'], rows[sid(1)]['stale']), ('stopped', False), 'not moved to Older by the stop')
        self.assertTrue(rows[sid(2)]['stale'], 'one the user stopped goes after two days, as before')

    def test_the_notice_no_longer_promises_that_nothing_stops(self):
        self.assertIn('These stop only if you say so.', features.Reaper.notice([{'mb': 5}], True)[1])
        self.assertIn('Nothing stops unless you say so.', features.Reaper.notice([{'mb': 5}])[1])


class HubOpenTest(unittest.TestCase):
    def test_reads_pairs_and_survives_a_down_hub(self):
        body = json.dumps({'open': [{'machine': 'desk', 'bg': 'abcd1234'}, {'machine': 'laptop', 'bg': 'ffff0000'}]}).encode()

        class H(BaseHTTPRequestHandler):
            def do_GET(self):
                self.send_response(200)
                self.send_header('Content-Length', str(len(body)))
                self.end_headers()
                self.wfile.write(body)

            def log_message(self, *a):
                pass
        srv = ThreadingHTTPServer(('127.0.0.1', 0), H)
        threading.Thread(target=srv.serve_forever, daemon=True).start()
        try:
            got = features.hub_open(f'http://127.0.0.1:{srv.server_address[1]}/internal/open')
        finally:
            srv.shutdown()
        self.assertEqual(got, {('desk', 'abcd1234'), ('laptop', 'ffff0000')})
        self.assertEqual(features.hub_open('http://127.0.0.1:9/internal/open', timeout=0.5), set())

    def test_asks_the_hub_on_its_own_port(self):
        # A hub on SWITCHBOARD_HUB_PORT: the open list comes from that port, not 8132.
        code = ('import sys; sys.path.insert(0, "server"); import features\n'
                'seen = []\n'
                'def fake(url, timeout): seen.append(url); raise OSError("no hub")\n'
                'features.urllib.request.urlopen = fake\n'
                'features.hub_open(); print(seen[0])')
        env = {k: v for k, v in os.environ.items() if k != 'SWITCHBOARD_HUB_URL'}
        env['SWITCHBOARD_HUB_PORT'] = '9000'
        out = subprocess.run([sys.executable, '-c', code], cwd=Path(__file__).resolve().parents[1], env=env,
                             capture_output=True, text=True, timeout=30)
        self.assertEqual(out.stdout.strip(), 'http://127.0.0.1:9000/internal/open', out.stderr)


if __name__ == '__main__':
    unittest.main()
