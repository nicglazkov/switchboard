"""Two changes to the budget at the same moment both land.

Each read the file, changed its own part and wrote the whole file back, with nothing between the read and the write
to make the other wait: two devices changing the budget, or a lift while the hard limit was being checked, lost one
of the two."""
import threading, time, unittest
from tests.test_budget import Base, NOW, at


class Slow:
    """Makes the first reader wait, with what it read in its hand, until a second one has had time to come and go."""

    def __init__(self, read):
        self.read, self.first, self.held, self.lock = read, True, threading.Event(), threading.Lock()

    def __call__(self):
        d = self.read()
        with self.lock:
            first, self.first = self.first, False
        if first:
            self.held.set()
            time.sleep(0.3)
        return d


class BudgetLockTest(Base):
    def both(self, a, b, slow):
        errs = []

        def run(fn):
            try:
                fn()
            except Exception as e:  # noqa: BLE001
                errs.append(e)
        ta = threading.Thread(target=run, args=(a,))
        ta.start()
        self.assertTrue(slow.held.wait(5))
        tb = threading.Thread(target=run, args=(b,))
        tb.start()
        ta.join(10)
        tb.join(10)
        self.assertEqual(errs, [])

    def test_two_devices_changing_the_budget_both_keep_their_change(self):
        slow = self.b.settings = Slow(self.b.settings)
        self.both(lambda: self.post(op='set', daily=10), lambda: self.post(op='set', five_hour=30), slow)
        s = self.app.get('/api/budget')['settings']
        self.assertEqual((s['daily'], s['five_hour']), (10.0, 30.0))

    def test_a_lift_during_a_change_keeps_both(self):
        slow = self.b.settings = Slow(self.b.settings)
        self.both(lambda: self.post(op='set', daily=10), lambda: self.post(op='lift'), slow)
        s = self.app.get('/api/budget')['settings']
        self.assertEqual((s['daily'], s['lifted_until'] is not None), (10.0, True))

    def test_a_lift_while_the_hard_limit_is_being_checked_is_not_lost(self):
        self.post(op='set', hard={'on': True, 'daily': 20})
        self.sample(at(2026, 10, 13, 0, 5), 18)
        self.sample(NOW, 40)  # 22 today: over
        day = self.b.status()['day']
        slow = self.b._state = Slow(self.b._state)
        self.both(self.b._check, lambda: self.post(op='lift'), slow)
        st = self.b._state.read()
        self.assertEqual((st.get('hard_day'), st.get('lift_day')), (day, day))


if __name__ == '__main__':
    unittest.main()
