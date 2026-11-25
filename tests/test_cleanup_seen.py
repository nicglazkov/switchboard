"""cleanup_seen.json is written when what it holds changed, not after every poll.

Each write is forced to disk, and the poll runs every 20 seconds for as long as the server is up."""
import unittest
from tests import test_reaper
from tests.test_reaper import bg
from unittest import mock
import features


class SeenWriteTest(unittest.TestCase):
    make = test_reaper.CleanupTest.make

    def writes(self, app, n=1):
        with mock.patch.object(features, 'jsave', wraps=features.jsave) as save:
            for _ in range(n):
                app.fleet.poll()
                app.feat.reaper.run()
        return [c for c in save.call_args_list if c[0][0] == app.feat.reaper.seen_path]

    def test_nothing_new_writes_nothing(self):
        app = self.make()
        self.m.sessions['desk'] = [bg(1, 'idle', 99, mb=365)]
        self.assertEqual(len(self.writes(app)), 1, 'the first look is written down')
        self.assertEqual(len(self.writes(app, 5)), 0)
        self.assertEqual(len(self.pushes), 1)

    def test_a_new_one_or_one_that_went_away_is_written(self):
        app = self.make()
        self.m.sessions['desk'] = [bg(1, 'idle', 99, mb=365)]
        self.writes(app)
        self.m.sessions['desk'] = [bg(1, 'idle', 99, mb=365), bg(2, 'idle', 99)]
        self.assertEqual(len(self.writes(app, 2)), 1)
        self.m.sessions['desk'] = []
        self.assertEqual(len(self.writes(app, 2)), 1)

    def test_no_chats_at_all_writes_nothing(self):
        app = self.make()
        self.writes(app)
        self.assertEqual(len(self.writes(app, 3)), 0)


if __name__ == '__main__':
    unittest.main()
