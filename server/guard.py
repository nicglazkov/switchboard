"""Turns a guard block into one iPhone push, unless the session is answered first.

One push per held message: more messages typed into a chat that is already held say nothing new, so they stay
quiet until the hold is answered. A chat that moved on by itself (a prompt went through, Claude finished a turn)
is no longer held, and a hold nobody answered lapses after HELD_FOR, so an old chat never stays "held" for days."""
import threading, time

REPEAT_QUIET = 30 * 60  # after an answer, a new hold of the same chat pushes again only after this
HELD_FOR = 6 * 3600


class GuardTimer:
    def __init__(self, delay, send, timer=threading.Timer, clock=time.time):
        self.delay, self.send, self.timer, self.clock = delay, send, timer, clock
        self.pending, self.pushed, self.held_set = {}, {}, {}  # held_set: key -> when it was held
        self.lock = threading.Lock()

    def block(self, evt):
        key = (evt.get('machine'), evt.get('session_id'))
        now = self.clock()
        with self.lock:
            self._lapse(now)
            if key in self.pending:
                return
            if key in self.held_set or now - self.pushed.get(key, -1e9) < REPEAT_QUIET:
                self.held_set.setdefault(key, now)  # held, as the apps show it, but nothing new to push
                return
            t = self.timer(self.delay, lambda: self._fire(key, evt))
            self.pending[key] = t
        t.start()

    def _fire(self, key, evt):
        with self.lock:
            if self.pending.pop(key, None) is None:
                return
            now = self.clock()
            self.pushed[key] = now
            self.held_set[key] = now
        self.send(evt)

    def answer(self, evt):
        key = (evt.get('machine'), evt.get('session_id'))
        with self.lock:
            t = self.pending.pop(key, None)
            self.held_set.pop(key, None)
        if t:
            t.cancel()

    moved = answer  # a prompt went through or the turn ended: whatever was held is not any more

    def _lapse(self, now):
        for k in [k for k, at in self.held_set.items() if now - at > HELD_FOR]:
            del self.held_set[k]

    def held(self, machine, sid):
        with self.lock:
            self._lapse(self.clock())
            return (machine, sid) in self.held_set
