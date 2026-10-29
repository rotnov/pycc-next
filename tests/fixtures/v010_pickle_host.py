"""Pickles instances of `m`'s compiled classes (`tests/fixtures/v010_pickle.py`)."""
# The pickled data is this test's own round trip.
import copy
import pickle

import m

p = m.Point(3, 1.5, "a", True)
for proto in range(pickle.HIGHEST_PROTOCOL + 1):
    q = pickle.loads(pickle.dumps(p, proto))
    print(proto, type(q).__name__, q.show(), q is p)
r = m.Ring(m.Point(1, 2.0, "r", False), 7)
back = pickle.loads(pickle.dumps(r))
print(back.total(), m.total_ring(back), back.start.show(), back.step)
deep = copy.deepcopy(r)
print(deep.total(), deep.start.show(), deep.start is r.start)
pair = pickle.loads(pickle.dumps([p, p]))
print(pair[0] is pair[1], pair[0].show())
lazy = pickle.loads(pickle.dumps(m.Lazy(0)))
print(lazy.n)
try:
    lazy.get()
except AttributeError as e:
    print("AttributeError", e)
print(pickle.loads(pickle.dumps(m.Lazy(9))).get())


class Sub(m.Point):
    def show(self):
        return "sub " + super().show() + " " + self.extra


s = Sub(1, 2.0, "s", False)
s.extra = "x"
t = pickle.loads(pickle.dumps(s))
print(type(t).__name__, t.show(), t.extra)
