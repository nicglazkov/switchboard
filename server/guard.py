"""Turns a guard block into one iPhone push, unless the session is answered first."""
import threading, time

REPEAT_QUIET = 120


class GuardTimer:
    def __init__(self, delay, send, timer=threading.Timer, clock=time.time):
        self.delay, self.send, self.timer, self.clock = delay, send, timer, clock
        self.pending, self.pushed, self.held_set = {}, {}, set()
        self.lock = threading.Lock()

    def block(self, evt):
        key = (evt.get('machine'), evt.get('session_id'))
        with self.lock:
            if key in self.pending or self.clock() - self.pushed.get(key, -1e9) < REPEAT_QUIET:
                return
            t = self.timer(self.delay, lambda: self._fire(key, evt))
            self.pending[key] = t
        t.start()

    def _fire(self, key, evt):
        with self.lock:
            if self.pending.pop(key, None) is None:
                return
            self.pushed[key] = self.clock()
            self.held_set.add(key)
        self.send(evt)

    def answer(self, evt):
        key = (evt.get('machine'), evt.get('session_id'))
        with self.lock:
            t = self.pending.pop(key, None)
            self.held_set.discard(key)
        if t:
            t.cancel()

    def held(self, machine, sid):
        return (machine, sid) in self.held_set
