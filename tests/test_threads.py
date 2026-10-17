"""Threads: chats on one topic are linked, the board shows the newest, History lists the rest, manual links win."""
import json, sys, unittest
from pathlib import Path
from tempfile import mkdtemp
sys.path.insert(0, str(Path(__file__).resolve().parents[1] / 'server'))
import features, fleet, server  # noqa: E402
from tests.test_server import MACH  # noqa: E402
from tests.test_reaper import Machine  # noqa: E402

NOW = 2_000_000_000.0
H = 3600


def sid(n):
    return f'{n:08x}-1111-2222-3333-444444444444'


def chat(n, title, started_h, ended_h, status=None, cwd='/Users/you/proj', **kw):
    """A chat started `started_h` hours ago and last used `ended_h` hours ago."""
    d = {'sid': sid(n), 'title': title, 'kind': None, 'status': status, 'cold': True, 'ctx': 1000 * n, 'cwd': cwd,
         'folder': cwd.rsplit('/', 1)[-1], 'started': NOW - started_h * H, 'active': NOW - ended_h * H,
         'mtime': NOW - ended_h * H, 'url': f'https://claude.ai/code/session_{n}'}
    d.update(kw)
    return d


class TopicTest(unittest.TestCase):
    def test_the_name_without_what_switchboard_adds(self):
        for title, want in [('Oct 12 12:50a Home Assistant (fresh)', 'Home Assistant'), ('Kitchen / remodel (2)', 'Kitchen / remodel'),
                            ('Remember PELICAN-42 Ready (new, web)', 'Remember PELICAN-42 Ready'),
                            ('Plan (fresh 2)', 'Plan'), ('Plan (from Laptop)', 'Plan'), ('Fix the build (for Desk)', 'Fix the build'),
                            ('YouTube Music app fresh fresh', 'YouTube Music app'), ('Camino (Kitchen remodel)', 'Camino (Kitchen remodel)'),
                            ('Oct 3 9:05p Lights (UltraCode, web)', 'Lights')]:
            self.assertEqual(features.topic_of(title), want, title)

    def test_jaccard_on_words(self):
        w = features.topic_words
        self.assertEqual(features.jaccard(w('Kitchen / remodel'), w('Kitchen remodel (fresh)')), 1.0)
        self.assertGreaterEqual(features.jaccard(w('Fix the iOS build'), w('Fix iOS build again')), 0.6)
        self.assertLess(features.jaccard(w('sb-test-1'), w('sb-test-term')), 0.6)
        self.assertLess(features.jaccard(w('Home Assistant lights'), w('Home Assistant blinds')), 0.6)
        self.assertEqual(features.jaccard(set(), set()), 0.0)

    def test_folder_names(self):
        for cwd, want in [('/Users/you', 'home folder'), ('C:\\Users\\you', 'home folder'), ('/home/you/', 'home folder'),
                          ('/Users/you/Desktop/Claude/switchboard', 'switchboard'), ('C:\\Users\\you\\code', 'code'), (None, '')]:
            self.assertEqual(features.folder_name(cwd), want, cwd)

    def test_worktrees_belong_to_their_repo(self):
        self.assertEqual(features.repo_key('/Users/you/sb/.claude/worktrees/feat'), features.repo_key('/Users/you/sb'))
        self.assertEqual(features.repo_key('C:\\Users\\you\\sb\\'), 'c:/users/you/sb')


class ThreadsTest(unittest.TestCase):
    def make(self):
        self.m = Machine()
        self.state = Path(mkdtemp())
        app = server.App(fleet.Fleet(MACH, self.m, clock=lambda: NOW), state_dir=self.state, web_dir=self.state,
                         clock=lambda: NOW, cfg_dir=Path(mkdtemp()))
        app._refresh_soon = lambda *a, **k: None
        app.feat.reaper.opened = lambda: set()
        self.app = app
        return app

    def poll(self):
        self.app.fleet.poll()
        self.app.feat.after_poll()
        return self.app.sessions()

    def board(self, snap, name='desk'):
        m = next(m for m in snap['machines'] if m['name'] == name)
        return {s['sid']: s for s in m['sessions']}, {s['sid']: s for s in m['replaced']}

    def test_same_topic_same_folder_is_one_thread(self):
        self.make()
        self.m.sessions['desk'] = [chat(1, 'Home Assistant', 50, 40), chat(2, 'Oct 12 9:00a Home Assistant (fresh)', 20, 1),
                                   chat(3, 'Home Assistant', 30, 25, cwd='/Users/you/other'),   # another folder
                                   chat(4, 'Blinds schedule', 10, 9)]                             # another topic
        board, repl = self.board(self.poll())
        self.assertEqual(set(board), {sid(2), sid(3), sid(4)})
        self.assertEqual((board[sid(2)]['thread'], board[sid(2)]['history']), (sid(1), 1))
        self.assertEqual((board[sid(3)]['thread'], board[sid(3)]['history']), (sid(3), 0))
        self.assertEqual(repl[sid(1)]['by'], {'machine': 'desk', 'sid': sid(2)})
        self.assertEqual(repl[sid(1)]['how'], 'same-topic')
        self.assertEqual(repl[sid(1)]['thread'], sid(1))
        t = self.app.get(f'/api/thread/desk/{sid(2)}')
        self.assertEqual([(c['sid'], c['how']) for c in t['chats']], [(sid(1), 'same-topic'), (sid(2), 'current')])
        self.assertEqual(t['thread'], sid(1))
        c = t['chats'][0]
        self.assertEqual((c['machine'], c['label'], c['title'], c['started'], c['ended'], c['ctx'], c['url']),
                         ('desk', 'Desk', 'Home Assistant', NOW - 50 * H, NOW - 40 * H, 1000, 'https://claude.ai/code/session_1'))

    def test_two_chats_on_one_topic_at_once_stay_apart(self):
        self.make()
        self.m.sessions['desk'] = [chat(1, 'Lights', 50, 2), chat(2, 'Lights', 20, 1),            # the older one is still used
                                   chat(5, 'Blinds', 50, 30, status='idle', cold=False, stale=False), chat(6, 'Blinds', 20, 10),  # open, warm
                                   chat(7, 'Plan', 50, 30, status='waiting'), chat(8, 'Plan', 20, 10)]       # waiting for the user
        board, _ = self.board(self.poll())
        self.assertEqual(set(board), {sid(n) for n in (1, 2, 5, 6, 7, 8)})
        self.assertTrue(all(s['history'] == 0 for s in board.values()))

    def test_a_working_older_chat_stays_on_the_board(self):
        self.make()
        self.m.sessions['desk'] = [chat(1, 'Lights', 50, 40), chat(2, 'Lights', 20, 1)]
        self.poll()
        self.m.sessions['desk'][0].update(status='busy', active=NOW, mtime=NOW)  # the old one is resumed and working
        board, _ = self.board(self.poll())
        self.assertIn(sid(1), board)
        self.assertIn(sid(2), board)

    def test_replaced_marks_and_jobs_link_across_machines(self):
        app = self.make()
        old = chat(1, 'Kitchen / remodel', 90, 80)
        self.m.replaced['desk'] = [dict(old, how='fresh', by={'machine': 'desk', 'sid': sid(2)})]
        self.m.sessions['desk'] = [chat(2, 'Kitchen / remodel (fresh)', 79, 30, cwd='/Users/you')]
        self.m.sessions['tower'] = [chat(3, 'Kitchen / remodel (from Desk)', 29, 1, cwd='C:\\Users\\you\\remodel')]
        self.poll()
        app.feat.job_done({'kind': 'continue', 'state': 'done', 'machine': 'tower', 'sid': sid(3),
                           'plan': {'src': 'desk', 'sid': sid(2), 'dst': 'tower'}})
        snap = self.app.sessions()
        board, repl = self.board(snap)
        wboard, _ = self.board(snap, 'tower')
        self.assertNotIn(sid(2), board)                      # the Desk chat now lives on the Tower
        self.assertEqual(repl[sid(2)]['by'], {'machine': 'tower', 'sid': sid(3)})
        self.assertEqual(repl[sid(2)]['how'], 'continued')
        self.assertEqual((wboard[sid(3)]['thread'], wboard[sid(3)]['history']), (sid(1), 2))
        t = app.get(f'/api/thread/tower/{sid(3)}')
        self.assertEqual([(c['machine'], c['how']) for c in t['chats']], [('desk', 'fresh'), ('desk', 'continued'), ('tower', 'current')])
        self.assertEqual(t['chats'][2]['label'], 'Tower')
        # the same thread from any of its chats
        self.assertEqual(app.get(f'/api/thread/desk/{sid(1)}')['chats'], [dict(c, this=c['sid'] == sid(1)) for c in t['chats']])

    def test_a_fresh_job_and_a_handoff_file(self):
        app = self.make()
        self.m.sessions['desk'] = [chat(1, 'Alpha', 50, 40), chat(2, 'Something else entirely', 39, 30),
                                   chat(3, 'Gamma', 29, 1, handoff_of=sid(2))]
        self.poll()
        app.feat.job_done({'kind': 'fresh', 'state': 'done', 'machine': 'desk', 'sid': sid(2), 'plan': {'req': {'sid': sid(1)}}})
        app.feat.job_done({'kind': 'fresh', 'state': 'error', 'machine': 'desk', 'sid': sid(9), 'plan': {'req': {'sid': sid(1)}}})
        t = app.get(f'/api/thread/desk/{sid(3)}')
        self.assertEqual([(c['sid'], c['how']) for c in t['chats']], [(sid(1), 'fresh'), (sid(2), 'handoff'), (sid(3), 'current')])
        stored = json.loads((self.state / 'threads.json').read_text())
        self.assertEqual(stored['auto'][f'desk/{sid(2)}']['to'], f'desk/{sid(1)}')

    def test_a_fresh_start_in_another_folder_joins_its_chat(self):
        # The Laptop case: Start fresh on a home-folder chat ran in the Claude folder, its handoff came in without
        # naming the file and the job link was gone; a /clear in the terminal later made a second fresh chat.
        self.make()
        self.m.sessions['desk'] = [chat(1, 'Kitchen / remodel', 90, 40, cwd='/Users/you'),
                                   chat(2, 'Kitchen remodel (fresh)', 30, 2, cwd='/Users/you/Claude', kind='background'),
                                   chat(3, 'Kitchen / remodel', 25, 20, cwd='/Users/you', handoff_of=sid(1)),
                                   chat(4, 'Kitchen notes', 20, 10, cwd='/Users/you/Claude')]   # not tagged fresh
        board, repl = self.board(self.poll())
        self.assertEqual(board[sid(2)]['thread'], sid(1))
        self.assertEqual(board[sid(4)]['thread'], sid(4))
        self.assertIn(sid(1), repl)
        t = self.app.get(f'/api/thread/desk/{sid(2)}')
        self.assertEqual({c['sid'] for c in t['chats']}, {sid(1), sid(2), sid(3)})

    def test_a_fresh_tag_does_not_link_a_chat_used_after_it_started(self):
        self.make()
        self.m.sessions['desk'] = [chat(1, 'Lights', 50, 1, cwd='/Users/you'), chat(2, 'Lights (fresh)', 20, 2, cwd='/Users/you/Claude')]
        board, _ = self.board(self.poll())
        self.assertEqual((board[sid(1)]['thread'], board[sid(2)]['thread']), (sid(1), sid(2)))

    def test_a_star_follows_its_chat_into_cleared_and_fresh_parts_and_unstars_them_all(self):
        app = self.make()
        old = chat(1, 'Lights', 90, 80)
        self.m.sessions['desk'] = [old]
        self.poll()
        self.assertTrue(app.post('/api/action', {'machine': 'desk', 'verb': 'star', 'sid': sid(1)})['star'])
        # /clear in the terminal: a new chat id, linked by its replaced mark
        self.m.replaced['desk'] = [dict(old, how='cleared', by={'machine': 'desk', 'sid': sid(2)})]
        self.m.sessions['desk'] = [chat(2, 'Lights', 79, 1)]
        board, _ = self.board(self.poll())
        self.assertTrue(board[sid(2)].get('star'))
        # a later manual link gives the thread an older first chat: still starred
        self.m.sessions['desk'] = [chat(2, 'Lights', 79, 1), chat(3, 'Wiring plan', 120, 100)]
        self.poll()
        app.post('/api/action', {'machine': 'desk', 'verb': 'link', 'sid': sid(1), 'to': {'machine': 'desk', 'sid': sid(3)}})
        board, _ = self.board(self.app.sessions())
        self.assertEqual(board[sid(2)]['thread'], sid(3))
        self.assertTrue(board[sid(2)].get('star'))
        # unstarring the newest part clears every part
        self.assertFalse(app.post('/api/action', {'machine': 'desk', 'verb': 'star', 'sid': sid(2)})['star'])
        board, repl = self.board(self.app.sessions())
        self.assertNotIn('star', board[sid(2)])
        self.assertTrue(all('star' not in r for r in repl.values()))

    def test_manual_link_wins_and_not_the_same_topic_sticks(self):
        app = self.make()
        self.m.sessions['desk'] = [chat(1, 'Lights', 50, 40), chat(2, 'Lights', 20, 1), chat(3, 'Wiring plan', 60, 55)]
        self.poll()
        # "Not the same topic": the heuristic link goes and stays gone
        r = app.post('/api/action', {'machine': 'desk', 'verb': 'unlink', 'sid': sid(2)})
        self.assertEqual([c['sid'] for c in r['chats']], [sid(2)])
        board, _ = self.board(self.poll())
        self.assertEqual(board[sid(2)]['history'], 0)
        self.assertIn(sid(1), board)
        # "This continues...": pick any older chat, even one on another topic
        r = app.post('/api/action', {'machine': 'desk', 'verb': 'link', 'sid': sid(2), 'to': {'machine': 'desk', 'sid': sid(3)}})
        self.assertEqual([(c['sid'], c['how']) for c in r['chats']], [(sid(3), 'linked'), (sid(2), 'current')])
        board, repl = self.board(self.app.sessions())
        self.assertNotIn(sid(3), board)
        self.assertEqual(repl[sid(3)]['how'], 'linked')
        stored = json.loads((self.state / 'threads.json').read_text())
        self.assertEqual(stored['links'][f'desk/{sid(2)}']['to'], f'desk/{sid(3)}')
        self.assertNotIn(f'desk/{sid(2)}', stored['unlinked'])

    def test_not_the_same_topic_from_the_older_chat(self):
        app = self.make()
        self.m.sessions['desk'] = [chat(1, 'Lights', 50, 40), chat(2, 'Lights', 20, 1)]
        _, repl = self.board(self.poll())
        self.assertEqual(repl[sid(1)]['how'], 'same-topic')
        app.post('/api/action', {'machine': 'desk', 'verb': 'unlink', 'sid': sid(1), 'older': True})
        board, repl = self.board(self.app.sessions())
        self.assertEqual(set(board), {sid(1), sid(2)})
        self.assertEqual(board[sid(2)]['history'], 0)

    def test_bad_links_are_refused(self):
        app = self.make()
        self.m.sessions['desk'] = [chat(1, 'A', 50, 40), chat(2, 'B', 20, 1)]
        self.poll()
        app.post('/api/action', {'machine': 'desk', 'verb': 'link', 'sid': sid(2), 'to': {'machine': 'desk', 'sid': sid(1)}})
        for body, msg in [({'sid': sid(1), 'to': {'machine': 'desk', 'sid': sid(2)}}, 'already continues'),
                          ({'sid': sid(1), 'to': {'machine': 'desk', 'sid': sid(1)}}, 'cannot continue itself'),
                          ({'sid': sid(1), 'to': {'machine': 'desk', 'sid': sid(7)}}, 'not one Switchboard has seen'),
                          ({'sid': sid(1), 'to': {'machine': 'nope', 'sid': sid(2)}}, 'unknown machine'),
                          ({'sid': 'x', 'to': {'machine': 'desk', 'sid': sid(2)}}, 'bad session id')]:
            with self.assertRaises(server.HttpError) as e:
                app.post('/api/action', dict(body, machine='desk', verb='link'))
            self.assertIn(msg, str(e.exception))
        with self.assertRaises(server.HttpError) as e:
            app.get(f'/api/thread/desk/{sid(7)}')
        self.assertEqual(e.exception.code, 404)

    def test_history_outlives_the_machines_lists(self):
        app = self.make()
        self.m.sessions['desk'] = [chat(1, 'Lights', 50, 40), chat(2, 'Lights', 20, 1)]
        self.poll()
        self.m.sessions['desk'] = [chat(2, 'Lights', 20, 1)]   # the old chat aged out of the list
        board, _ = self.board(self.poll())
        self.assertEqual(board[sid(2)]['history'], 1)
        app2 = server.App(fleet.Fleet(MACH, self.m, clock=lambda: NOW), state_dir=self.state, web_dir=self.state,
                          clock=lambda: NOW, cfg_dir=Path(mkdtemp()))   # and a restart
        self.assertEqual(len(app2.get(f'/api/thread/desk/{sid(2)}')['chats']), 2)

    def test_a_newest_chat_gone_from_disk_hides_nothing(self):
        self.make()
        self.m.sessions['desk'] = [chat(1, 'Lights', 50, 40), chat(2, 'Lights', 20, 1)]
        self.poll()
        self.m.sessions['desk'] = [chat(1, 'Lights', 50, 40)]
        board, _ = self.board(self.poll())
        self.assertIn(sid(1), board)

    def test_search_for_the_picker(self):
        app = self.make()
        self.m.sessions['desk'] = [chat(1, 'Lights', 50, 40), chat(2, 'Blinds', 20, 1)]
        self.m.sessions['tower'] = [chat(3, 'Lights on the Tower', 30, 2, cwd='C:\\Users\\you\\home')]
        self.poll()
        r = app.get('/api/chats', {'q': ['lig'], 'exclude': [f'desk/{sid(2)}']})
        self.assertEqual([(c['machine'], c['sid']) for c in r['chats']], [('desk', sid(1)), ('tower', sid(3))])  # its folder first
        self.assertEqual((r['chats'][1]['folder'], r['chats'][1]['label']), ('home', 'Tower'))
        self.assertEqual(len(app.get('/api/chats', {'q': ['Tower'], 'exclude': [f'desk/{sid(2)}']})['chats']), 1)  # by machine name
        self.assertEqual(app.get('/api/chats', {'exclude': [f'tower/{sid(3)}']})['chats'][0]['sid'], sid(1))   # only older ones
        self.assertEqual(len(app.get('/api/chats', {'exclude': [f'tower/{sid(3)}']})['chats']), 1)
        self.assertEqual([c['in_thread'] for c in r['chats']], [False, False])
        self.m.sessions['desk'].append(chat(4, 'Lights', 10, 0.5))   # 4 continues 1 (same topic, same folder)
        self.poll()
        r = app.get('/api/chats', {'q': ['lights'], 'exclude': [f'desk/{sid(4)}']})
        self.assertEqual({c['sid']: c['in_thread'] for c in r['chats']}, {sid(1): True, sid(3): False})

    def test_loops_are_cut(self):
        app = self.make()
        self.m.replaced['desk'] = [dict(chat(1, 'X', 50, 40), how='fresh', by={'machine': 'desk', 'sid': sid(2)}),
                                   dict(chat(2, 'Y', 20, 1), how='fresh', by={'machine': 'desk', 'sid': sid(1)})]
        self.poll()
        t = app.get(f'/api/thread/desk/{sid(1)}')
        self.assertEqual(len(t['chats']), 2)
        self.assertEqual(app.get(f'/api/thread/desk/{sid(2)}')['thread'], t['thread'])


class AgentHandoffTest(unittest.TestCase):
    """The agent says which chat's handoff file a chat started from, only when its first message names it."""
    def write(self, name, lines):
        p = Path(mkdtemp()) / f'{name}.jsonl'
        p.write_text(''.join(json.dumps(x) + '\n' for x in lines))
        return p

    def user(self, text):
        return {'type': 'user', 'timestamp': '2026-10-12T10:00:00Z', 'message': {'role': 'user', 'content': text}}

    def test_handoff_of(self):
        from tests.helpers import fresh_sb
        sb = fresh_sb(mkdtemp())
        old, me = sid(1), sid(2)
        win = f'C:\\Users\\you\\.claude\\switchboard\\handoffs\\{old}.md'
        for text, want in [(f'Read the whole file /Users/you/.claude/switchboard/handoffs/{old}.md first', old),
                           (f'Before anything, read {win}', old),
                           (f'Notes in handoffs/{me}.md', None),          # its own handoff file
                           ('Fix the build', None)]:
            self.assertEqual(sb._head_meta(self.write(me, [self.user(text)]))['handoff_of'], want, text)
        later = [self.user('Why did the handoff fail?'), {'type': 'assistant', 'message': {'content': 'x'}}] + \
                [self.user(f'Look at ~/.claude/switchboard/handoffs/{old}.md')]
        self.assertIsNone(sb._head_meta(self.write(me, later))['handoff_of'], 'a later mention linked the chat')


if __name__ == '__main__':
    unittest.main()
