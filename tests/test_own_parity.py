"""The helper's copy of the live-state rule (agent/sb.py own_derive, own_mark_age) against the server's
(server/fleet.py derive, mark_age): one table of cases through both. The helper answers with its copy when the home
computer is away (keys phase 4), and the two must never disagree about a chat."""
import itertools, sys, tempfile, unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / 'server'))
import fleet  # noqa: E402
from tests.helpers import fresh_sb  # noqa: E402

NOW = 1_800_000_000.0
H = 3600
SID = 'aaaaaaaa-1111-2222-3333-444444444444'


def cases():
    """Every mix of what Claude Code can say about a chat, how old it is, what a hook said last and whether a
    message is held, plus the cache readings of new and old helpers."""
    statuses = (None, 'idle', 'busy', 'waiting', 'stopped', 'done')
    states = (None, 'running', 'blocked', 'done')
    kinds = (None, 'interactive', 'background')
    turns = (None, True, False)
    ages = (60, 5 * H, 7 * H, 30 * H, 3 * 86400, 8 * 86400)
    hooks = (None, {'why': 'prompt', 'at': NOW - 1}, {'why': 'stop', 'at': NOW - 1}, {'why': 'wait', 'at': NOW - 1},
             {'why': None, 'at': NOW - 1, 'cold_at': NOW + 200}, {'why': 'prompt', 'at': NOW - 1, 'cold_at': NOW - 5})
    for status, state, kind, turn, age, hook in itertools.product(statuses, states, kinds, turns, ages, hooks):
        for held in (False, True):
            yield ({'sid': SID, 'status': status, 'state': state, 'kind': kind, 'turn_open': turn, 'active': NOW - age,
                    'mtime': NOW - age, 'ctx': 40_000}, {'held': held, 'hook': hook})
    base = {'sid': SID, 'status': 'busy', 'kind': 'background', 'active': NOW - 9 * H, 'mtime': NOW - 60, 'ctx': 150_000}
    extra = [
        dict(base, child_ts=NOW - 60), dict(base, child_ts=NOW - 16 * 60, turn_open=False), dict(base, last_ts=NOW - 9 * H),
        dict(base, last_ts=NOW - 30), dict(base, cache_expires_at=NOW + 100), dict(base, cache_expires_at=NOW - 100),
        dict(base, cache_expires_at=None), dict(base, cache_left=120), dict(base, cold=True), dict(base, cache_left=0, cold=True),
        dict(base, ctx=149_999), dict(base, ctx=None), dict(base, active=None), dict(base, active=None, mtime=None),
        {'sid': SID},
    ]
    for s in extra:
        for kw in ({}, {'skew': 30.0}, {'skew': -45.5, 'updated': NOW - 3}, {'updated': NOW - 10},
                   {'hook': {'why': 'prompt', 'at': NOW - 2, 'cold_at': NOW + 50}}):
            yield s, kw


class OwnParityTest(unittest.TestCase):
    def setUp(self):
        self.sb = fresh_sb(tempfile.mkdtemp())

    def test_the_numbers_the_rule_uses_are_the_same(self):
        sb = self.sb
        self.assertEqual((sb.OWN_STALE_CLOSED, sb.OWN_STALE_OPEN, sb.OWN_STALLED, sb.OWN_CHILD_WORK, sb.OWN_PARKED, sb.GUARD_CTX),
                         (fleet.STALE_CLOSED, fleet.STALE_OPEN, fleet.STALLED, fleet.CHILD_WORK, fleet.PARKED, fleet.GUARD_CTX))

    def test_one_table_through_both(self):
        n = 0
        for s, kw in cases():
            n += 1
            want = fleet.derive(dict(s), NOW, **kw)
            got = self.sb.own_derive(dict(s), NOW, **kw)
            if want != got:
                self.fail(f'derive differs for {s} {kw}: server {want}, helper {got}')
            row = dict(s, **want)
            a, b = fleet.mark_age(dict(row), NOW), self.sb.own_mark_age(dict(row), NOW)
            if a != b:
                self.fail(f'mark_age differs for {row}: server {a}, helper {b}')
            # and for a row with no live state yet, as mark_age also takes
            self.assertEqual(fleet.mark_age(dict(s), NOW), self.sb.own_mark_age(dict(s), NOW), s)
        self.assertGreater(n, 5000)
        seen = {fleet.derive(dict(s), NOW, **kw)['activity'] for s, kw in cases()}
        self.assertEqual(seen, set(fleet.ACTIVITIES), 'the table reaches every activity')


if __name__ == '__main__':
    unittest.main()
