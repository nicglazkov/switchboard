"""Tab groups: shared by every Mac through the server, keyed by thread so a group follows Start fresh, Continue and
Resume, shown on session rows and in the sessions snapshot."""
import json, sys, unittest
from pathlib import Path
from tempfile import mkdtemp
sys.path.insert(0, str(Path(__file__).resolve().parents[1] / 'server'))
import fleet, server  # noqa: E402
from tests.test_server import MACH  # noqa: E402
from tests.test_reaper import Machine  # noqa: E402
from tests.test_threads import chat, sid, NOW  # noqa: E402


class GroupsTest(unittest.TestCase):
    def setUp(self):
        self.m = Machine()
        self.cfg = Path(mkdtemp())
        app = server.App(fleet.Fleet(MACH, self.m, clock=lambda: NOW), state_dir=Path(mkdtemp()), web_dir=Path(mkdtemp()),
                         clock=lambda: NOW, cfg_dir=self.cfg)
        app._refresh_soon = lambda *a, **k: None
        app.feat.reaper.opened = lambda: set()
        self.app = app

    def op(self, **body):
        return self.app.post('/api/groups', body)

    def bad(self, code=400, **body):
        with self.assertRaises(server.HttpError, msg=body) as cm:
            self.op(**body)
        self.assertEqual(cm.exception.code, code, body)

    def stored(self):
        return json.loads((self.cfg / 'groups.json').read_text())

    def test_create_update_and_list(self):
        g = self.op(op='create', name='  Server   work ', color='blue', members=[sid(1), sid(2), sid(1)])['group']
        self.assertRegex(g['id'], r'^[0-9a-f]{8}$')
        self.assertEqual((g['name'], g['color'], g['members']), ('Server work', 'blue', [sid(1), sid(2)]))
        r = self.op(op='update', id=g['id'], name='Mac', color='pink')
        self.assertEqual([(x['name'], x['color']) for x in r['groups']], [('Mac', 'pink')])
        self.op(op='update', id=g['id'], name='')  # a group may have no name
        self.assertEqual(self.app.get('/api/groups')['groups'][0]['name'], '')
        self.assertEqual(self.app.get('/api/groups')['groups'][0]['color'], 'pink')
        self.assertEqual(self.stored()[0]['members'], [sid(1), sid(2)])

    def test_create_takes_members_from_other_groups(self):
        a = self.op(op='create', name='A', color='red', members=[sid(1)])['group']
        b = self.op(op='create', name='B', color='green', members=[sid(1), sid(2)])
        self.assertEqual([x['id'] for x in b['groups']], [b['group']['id']], 'A lost its only chat, so it is gone')
        self.assertNotIn(a['id'], [x['id'] for x in self.app.get('/api/groups')['groups']])

    def test_add_moves_between_groups_and_keeps_order(self):
        a = self.op(op='create', name='A', color='red', members=[sid(1), sid(2)])['group']
        b = self.op(op='create', name='B', color='green', members=[sid(3)])['group']
        r = self.op(op='add', id=b['id'], thread=sid(1), index=0)
        gs = {x['id']: x['members'] for x in r['groups']}
        self.assertEqual(gs, {a['id']: [sid(2)], b['id']: [sid(1), sid(3)]})
        r = self.op(op='add', id=b['id'], thread=sid(1), index=99)  # within a group: a reorder, past the end = last
        self.assertEqual({x['id']: x['members'] for x in r['groups']}[b['id']], [sid(3), sid(1)])
        r = self.op(op='add', id=b['id'], thread=sid(4))
        self.assertEqual({x['id']: x['members'] for x in r['groups']}[b['id']], [sid(3), sid(1), sid(4)])

    def test_remove_and_empty_groups_go(self):
        a = self.op(op='create', name='A', color='red', members=[sid(1), sid(2)])['group']
        self.op(op='remove', thread=sid(1))
        self.assertEqual(self.app.get('/api/groups')['groups'][0]['members'], [sid(2)])
        r = self.op(op='remove', thread=sid(2))
        self.assertEqual(r['groups'], [])
        self.op(op='remove', thread=sid(9))  # not in any group: nothing to do
        self.bad(404, op='update', id=a['id'], name='x')

    def test_delete_ungroups_and_move_reorders(self):
        ids = [self.op(op='create', name=n, color='grey', members=[sid(i)])['group']['id'] for i, n in enumerate('ABC', 1)]
        r = self.op(op='move', id=ids[2], index=0)
        self.assertEqual([x['name'] for x in r['groups']], ['C', 'A', 'B'])
        r = self.op(op='move', id=ids[2], index=50)
        self.assertEqual([x['name'] for x in r['groups']], ['A', 'B', 'C'])
        r = self.op(op='delete', id=ids[1])
        self.assertEqual([x['name'] for x in r['groups']], ['A', 'C'])
        self.assertEqual(self.m.calls, [], 'grouping never touches a machine (delete does not close chats)')

    def test_validation(self):
        g = self.op(op='create', name='A', color='red', members=[sid(1)])['group']
        for body in [dict(op='nuke'), dict(), dict(op='create', name='x' * 41, color='red'),
                     dict(op='create', name=5, color='red'), dict(op='create', name='a\x07b', color='red'),
                     dict(op='create', name='A', color='orange'), dict(op='create', name='A', color='red', members='x'),
                     dict(op='create', name='A', color='red', members=['../etc']),
                     dict(op='update', id='zz', name='x'), dict(op='update', id=g['id'], color='black'),
                     dict(op='add', id=g['id'], thread='nope'), dict(op='add', id=g['id'], thread=sid(2), index=-1),
                     dict(op='add', id=g['id'], thread=sid(2), index='1'), dict(op='add', id=g['id'], thread=sid(2), index=True),
                     dict(op='remove'), dict(op='move', id=g['id'], index=1.5), dict(op='delete', id=None)]:
            self.bad(**body)
        self.bad(404, op='delete', id='0badc0de')
        self.assertEqual(self.app.get('/api/groups')['groups'], [g], 'bad input changes nothing')

    def test_needs_the_switchboard_header(self):
        import threading, urllib.request, urllib.error
        srv = server.make_server('127.0.0.1', 0, self.app)
        threading.Thread(target=srv.serve_forever, daemon=True).start()
        try:
            url = f'http://127.0.0.1:{srv.server_address[1]}/api/groups'
            req = urllib.request.Request(url, data=json.dumps({'op': 'create', 'name': 'A', 'color': 'red'}).encode(),
                                         headers={'Content-Type': 'application/json', 'Host': '127.0.0.1'})
            with self.assertRaises(urllib.error.HTTPError) as cm:
                urllib.request.urlopen(req, timeout=5)
            self.assertEqual(cm.exception.code, 403)
            self.assertFalse((self.cfg / 'groups.json').exists())
        finally:
            srv.shutdown()

    def test_rows_and_snapshot_follow_the_thread(self):
        self.m.sessions['desk'] = [chat(1, 'Home Assistant', 50, 40), chat(2, 'Oct 12 9:00a Home Assistant (fresh)', 20, 1),
                                   chat(4, 'Blinds schedule', 10, 9)]
        self.app.fleet.poll()
        self.app.feat.after_poll()
        snap = self.app.sessions()
        self.assertEqual(snap['groups'], [])
        rows = {s['sid']: s for s in snap['machines'][0]['sessions']}
        self.assertEqual(rows[sid(2)]['thread'], sid(1))
        g = self.op(op='create', name='HA', color='cyan', members=[rows[sid(2)]['thread']])['group']
        snap = self.app.sessions()
        self.assertEqual(snap['groups'], [g])
        rows = {s['sid']: s for s in snap['machines'][0]['sessions']}
        repl = {s['sid']: s for s in snap['machines'][0]['replaced']}
        self.assertEqual(rows[sid(2)]['group'], g['id'], 'the fresh chat is in its thread\'s group')
        self.assertEqual(repl[sid(1)]['group'], g['id'])
        self.assertNotIn('group', rows[sid(4)])
        # a chat on no thread is keyed by its own sid
        self.op(op='add', id=g['id'], thread=sid(4))
        rows = {s['sid']: s for s in self.app.sessions()['machines'][0]['sessions']}
        self.assertEqual(rows[sid(4)]['group'], g['id'])

    def test_a_broken_file_reads_as_no_groups(self):
        (self.cfg / 'groups.json').write_text('{not json')
        self.assertEqual(self.app.get('/api/groups')['groups'], [])
        (self.cfg / 'groups.json').write_text(json.dumps([{'id': 'abcd1234', 'name': 3, 'color': 'mauve', 'members': [sid(1), 7]},
                                                          'junk']))
        self.assertEqual(self.app.get('/api/groups')['groups'],
                         [{'id': 'abcd1234', 'name': '', 'color': 'grey', 'members': [sid(1)]}])


if __name__ == '__main__':
    unittest.main()
