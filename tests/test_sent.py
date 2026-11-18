"""Tasks a chat sent (run_on) told apart from the person's own chats: who sent each, what counts as waiting for the
person, the pass that follows them after each poll, and the stop once one has reported."""
import json, sys, time, unittest
from pathlib import Path
from tempfile import mkdtemp
sys.path.insert(0, str(Path(__file__).resolve().parents[1] / 'server'))
import features, fleet, notes, server  # noqa: E402
from tests.test_dispatch import CHILD, FROM, Box  # noqa: E402
from tests.test_server import MACH  # noqa: E402

OTHER = 'dd' * 4 + '-1111-2222-3333-444444444444'
MINE = 'ee' * 4 + '-1111-2222-3333-444444444444'


class Rows(Box):
    """Box, with more chats than the one the task started."""
    def __init__(self):
        super().__init__()
        self.more = []  # (machine, row)

    def __call__(self, target, req, timeout):
        r = super().__call__(target, req, timeout)
        if req['verb'] == 'list':
            name = 'desk' if target == 'me@x' else 'tower'
            r['sessions'] = r['sessions'] + [dict(row) for m, row in self.more if m == name]
        return r


class Sent(unittest.TestCase):
    def make(self):
        self.now = [1_000_000.0]
        self.f, self.pushes = Rows(), []
        self.state, cfg = Path(mkdtemp()), Path(mkdtemp())
        (cfg / 'caps.json').write_text(json.dumps({'desk': True, 'tower': True}))
        app = server.App(fleet.Fleet(MACH, self.f), state_dir=self.state, web_dir=self.state, sleep=lambda s: None, poll_every=0,
                         retry_waits=(0, 0), cfg_dir=cfg, pusher=lambda *a: self.pushes.append(a[:3]), clock=lambda: self.now[0])
        app._refresh_soon = lambda *a, **k: None
        app.fleet.poll()
        app.feat.dispatch._poll_soon = lambda name: app.fleet.poll_one(name)
        self.app = app
        return app

    def send(self, **kw):
        body = {'from': {'machine': 'tower', 'sid': FROM}, 'machine': 'desk', 'task': 'Build the iOS widget and run its tests.',
                'folder': 'switchboard', 'mode': 'normal'}
        body.update(kw)
        r = self.app.post('/api/dispatch', body)
        for _ in range(300):  # the launch job runs on its own thread
            if self.task(r['id']).get('sid'):
                return r
            time.sleep(0.01)
        self.fail('the task never got its chat')

    def task(self, did):
        with self.app.feat.dispatch.lock:
            return dict(self.app.feat.dispatch.d.get(did) or {})

    def row(self, sid=CHILD, machine='desk'):
        m = {m['name']: m for m in self.app.sessions()['machines']}[machine]
        return next(s for s in m['sessions'] if s['sid'] == sid)

    def poll(self, later=0):
        self.now[0] += later
        self.f.row and self.f.row.update(mtime=self.f.row.get('mtime', 1.0))
        self.app.fleet.poll()

    def test_the_row_says_who_sent_it(self):
        app = self.make()
        self.f.more = [('tower', {'sid': FROM, 'title': 'Ship the widget', 'status': 'busy', 'ctx': 5, 'mtime': self.now[0], 'kind': 'interactive'})]
        r = self.send()
        app.fleet.poll()
        s = self.row()['sent']
        self.assertEqual((s['machine'], s['label'], s['sid'], s['title'], s['id']), ('tower', 'Tower', FROM, 'Ship the widget', r['id']))
        self.assertNotIn('sent', self.row(FROM, 'tower'), "the person's own chat carries nothing")
        self.assertEqual(json.loads((self.state / 'origins.json').read_text())[f'desk/{CHILD}']['sid'], FROM)

    def test_it_outlives_a_rename_the_week_and_a_restart(self):
        app = self.make()
        self.send()
        app.post('/api/action', {'verb': 'rename', 'machine': 'desk', 'sid': CHILD, 'name': 'Widget work'})
        app.fleet.poll()
        self.assertEqual(self.row()['title'], 'Widget work')
        self.assertEqual(self.row()['sent']['machine'], 'tower')
        d = app.feat.dispatch
        with d.lock:  # the task itself is forgotten after a week
            d.d.clear()
            d._save()
        self.assertEqual(self.row()['sent']['sid'], FROM)
        again = features.Dispatch(app, app.feat.auto, server.HttpError)  # a server that starts again reads the file
        self.assertEqual(again.origin(f'desk/{CHILD}')['machine'], 'tower')

    def test_tasks_sent_before_origins_were_kept_are_filled_in(self):
        app = self.make()
        r = self.send()
        (self.state / 'origins.json').unlink()
        again = features.Dispatch(app, app.feat.auto, server.HttpError)
        self.assertEqual(again.origin(f'desk/{CHILD}')['id'], r['id'])
        self.assertTrue((self.state / 'origins.json').exists())

    def test_start_fresh_and_resume_keep_it_and_a_same_topic_guess_does_not(self):
        app = self.make()
        self.send()
        base = {'status': 'idle', 'ctx': 5, 'mtime': self.now[0], 'kind': 'background'}
        self.f.more = [('desk', dict(base, sid=OTHER, title='Widget part 2')), ('desk', dict(base, sid=MINE, title='Holiday plans'))]
        app.fleet.poll()
        app.feat.threads.record(f'desk/{OTHER}', f'desk/{CHILD}', 'fresh')
        s = self.row(OTHER)['sent']
        self.assertEqual((s['machine'], s['sid']), ('tower', FROM))
        self.assertNotIn('id', s, 'the task and its report belong to the first chat')
        self.assertNotIn('sent', self.row(MINE))
        # the links threads guess from a name (same-topic) carry nothing
        self.assertEqual(features.Dispatch._origin_of({'a': {'machine': 'tower'}}, 'b', {'b': ('a', 'same-topic')}), (None, False))
        self.assertEqual(features.Dispatch._origin_of({'a': {'machine': 'tower'}}, 'b', {'b': ('a', 'resumed')})[0], {'machine': 'tower'})

    def test_taking_it_over_makes_it_the_persons_chat_and_back(self):
        app = self.make()
        self.send()
        self.f.row.update(title='Build the iOS widget (for Tower)')
        app.fleet.poll()
        self.assertEqual(app.post('/api/action', {'verb': 'adopt', 'machine': 'desk', 'sid': CHILD})['mine'], True)
        self.assertNotIn('sent', self.row(), 'not even from its old "(for Tower)" name')
        self.assertTrue(self.row()['title'].endswith('(for Tower)'))
        app.post('/api/action', {'verb': 'adopt', 'machine': 'desk', 'sid': CHILD, 'on': False})
        self.assertEqual(self.row()['sent']['sid'], FROM)

    def test_an_old_name_tag_counts_only_for_a_real_machine(self):
        app = self.make()
        base = {'status': 'idle', 'ctx': 5, 'mtime': self.now[0], 'kind': 'background'}
        self.f.more = [('desk', dict(base, sid=OTHER, title='Nov 3 2:10p Check the build (for Tower)')),
                       ('desk', dict(base, sid=MINE, title='Gift ideas (for Mum)'))]
        app.fleet.poll()
        self.assertEqual(self.row(OTHER)['sent'], {'machine': 'tower', 'label': 'Tower'})
        self.assertNotIn('sent', self.row(MINE))

    def test_it_is_followed_and_stopped_without_anyone_checking(self):
        app = self.make()
        r = self.send()
        d = app.feat.dispatch
        self.f.row.update(status='busy', ctx=9000, mtime=self.now[0])
        self.poll()
        d.after_poll()
        self.assertEqual(self.task(r['id'])['state'], 'working')
        self.assertEqual(self.f.verbs('stop'), [])
        self.f.row.update(status='idle', ctx=21000)
        self.poll(30)
        d.after_poll()
        t = self.task(r['id'])
        self.assertEqual((t['state'], t['report']), ('done', self.f.report))
        self.assertEqual(self.f.verbs('stop'), [], 'not the moment it finishes')
        self.assertEqual(self.row()['sent']['done_at'], t['done_at'])
        self.poll(features.SENT_STOP_AFTER + 1)
        d.after_poll()
        self.assertEqual([(n, q['sid']) for n, q in self.f.verbs('stop')], [('desk', CHILD)])
        self.assertEqual(self.row()['sent']['stopped_at'], self.now[0])
        self.assertEqual(self.row()['status'], 'stopped', 'and the board shows it at once')
        d.after_poll()
        self.assertEqual(len(self.f.verbs('stop')), 1, 'once')
        # the sender asks later: the report is the one kept, and now it counts as read
        self.assertNotIn('read_at', self.row()['sent'])
        got = app.get(f'/api/dispatch/{r["id"]}')
        self.assertEqual((got['state'], got['report']), ('done', self.f.report))
        self.assertEqual(self.row()['sent']['read_at'], self.now[0])
        self.assertEqual(len(self.f.verbs('report')), 1)

    def finish(self):
        """A task that has reported and is old enough to stop."""
        r = self.send()
        self.f.row.update(status='idle', ctx=21000, mtime=self.now[0], bg_id='bg1')
        self.poll(30)
        self.app.feat.dispatch.after_poll()
        self.assertEqual(self.task(r['id'])['state'], 'done')
        self.poll(features.SENT_STOP_AFTER + 1)
        return r

    def test_a_task_someone_has_a_use_for_is_not_stopped(self):
        for why, setup in (
                ('marked Keep running', lambda app: app.post('/api/action', {'verb': 'keep', 'machine': 'desk', 'sid': CHILD, 'on': True})),
                ('open in a tab', lambda app: app.feat.reaper.set_tabs('mac-1', [f'desk:{CHILD}'])),
                ('taken over', lambda app: app.post('/api/action', {'verb': 'adopt', 'machine': 'desk', 'sid': CHILD})),
                ('open in a terminal', lambda app: self.f.row.update(kind='interactive')),
                ('stopped at a prompt', lambda app: self.f.row.update(status='waiting')),
                ('working again', lambda app: self.f.row.update(status='busy')),
                ('used since', lambda app: self.f.row.update(last_ts=self.now[0] - 5)),
                ('its subagents still run', lambda app: setattr(self.f, 'busy', {'live': True, 'subagents': ['a'], 'summary': 'One subagent is running.'}))):
            with self.subTest(why):
                app = self.make()
                self.finish()
                setup(app)
                app.fleet.poll()
                app.feat.dispatch.after_poll()
                self.assertEqual(self.f.verbs('stop'), [], why)

    def test_a_machine_that_is_away_is_asked_again_later(self):
        app = self.make()
        r = self.finish()
        self.f.down.add('desk')
        app.fleet.poll()
        app.feat.dispatch.after_poll()
        self.assertNotIn('stopped', self.task(r['id']))
        self.f.down.clear()
        app.fleet.poll()
        app.feat.dispatch.after_poll()
        self.assertEqual(len(self.f.verbs('stop')), 1)

    def test_its_launch_is_not_pushed_to_the_phone(self):
        self.make()
        self.send()
        time.sleep(0.05)
        self.assertEqual(self.pushes, [])

    def test_history_marks_and_filters_them(self):
        app = self.make()
        self.send()
        self.f.more = [('desk', {'sid': MINE, 'title': 'My chat', 'status': 'idle', 'ctx': 5, 'mtime': self.now[0], 'kind': 'interactive'})]
        app.fleet.poll()
        q = lambda **kw: app.feat.browse.query({k: [v] for k, v in kw.items()})
        both = q()
        self.assertEqual({r['sid']: bool(r.get('sent')) for r in both['rows']}, {CHILD: True, MINE: False})
        self.assertEqual(both['facets']['sent'], 1)
        self.assertEqual([r['sid'] for r in q(by='claude')['rows']], [CHILD])
        self.assertEqual([r['sid'] for r in q(by='you')['rows']], [MINE])
        with self.assertRaises(ValueError):
            q(by='robots')


class Note(unittest.TestCase):
    """The top line: a task a chat sent waits for the person only at a prompt."""
    def pick(self, *rows):
        m = {'name': 'desk', 'label': 'Desk', 'online': True, 'sessions': [dict({'stale': False, 'active': 10}, **r) for r in rows]}
        return notes.pick([m], {}, now=100.0)

    def test_a_question_in_its_report_is_not_the_persons(self):
        sent = {'sent': {'machine': 'tower'}, 'title': 'Verify the build', 'sid': 'a'}
        n = self.pick(dict(sent, activity='needs', status='idle', state='blocked'))
        self.assertNotEqual((n or {}).get('kind'), 'need')
        n = self.pick(dict(sent, activity='needs', status='idle', state='blocked'),
                      {'title': 'My chat', 'sid': 'b', 'activity': 'needs', 'status': 'idle', 'state': 'blocked'})
        self.assertEqual(n['text'], 'My chat is waiting for you on Desk.')

    def test_one_stopped_at_a_prompt_is_said_as_what_it_is(self):
        sent = {'sent': {'machine': 'tower'}, 'title': 'Verify the build', 'sid': 'a', 'activity': 'needs', 'status': 'waiting'}
        self.assertEqual(self.pick(sent)['text'], 'A task Claude sent is waiting on Desk: Verify the build.')
        n = self.pick(dict(sent, active=99), {'title': 'My chat', 'sid': 'b', 'activity': 'needs', 'status': 'waiting'})
        self.assertEqual(n['text'], 'My chat is waiting for you on Desk. 1 more needs you too.')


if __name__ == '__main__':
    unittest.main()
