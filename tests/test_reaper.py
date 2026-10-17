"""Cleanup (was the reaper): which background chats it suggests stopping, what it never suggests, the once-a-day
notice, and that only a person's click stops anything."""
import json, os, sys, threading, unittest
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

    def __call__(self, target, req, timeout):
        name = 'desk' if target == 'me@x' else 'tower'
        self.calls.append((name, req))
        v = req['verb']
        if v == 'list':
            return {'ok': True, 'sessions': self.sessions.get(name, []), 'replaced': self.replaced.get(name, []),
                    'mem': self.mem.get(name), 'limits': None}
        if v == 'stop':
            if self.fail_stop:
                return {'ok': False, 'error': 'claude stop failed'}
            for s in self.sessions.get(name, []) + self.replaced.get(name, []):
                if s['sid'] == req['sid']:
                    s['status'] = 'stopped'
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
                         clock=lambda: self.now[0], cfg_dir=self.cfg, pusher=lambda *a: self.pushes.append(a))
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

    def test_nothing_stops_by_itself(self):
        app = self.make({'enabled': True, 'dry_run': False})  # even the old "live" setting stops nothing now
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
        self.assertEqual(self.pushes, [("2 sessions aren't in use", 'They hold 797 MB. Tap to look; nothing stops until you say so.',
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
        self.assertTrue(self.pushes[1][0].startswith('4 sessions'))
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


if __name__ == '__main__':
    unittest.main()
