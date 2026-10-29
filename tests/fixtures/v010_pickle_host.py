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


class SlotSub(m.Point):
    """A Python subclass's own `__slots__` pickle with the compiled attributes."""

    __slots__ = ("extra",)


class Hooked(m.Point):
    """`__getstate__` / `__setstate__` of a subclass are used, in their format."""

    def __getstate__(self):
        return {"label": self.label, "x": self.x}

    def __setstate__(self, state):
        self.label = state["label"] + "!"
        self.x = state["x"] * 2
        self.y = 0.5
        self.on = True


class Newed(m.Point):
    """A `__new__` taking only the class sees no extra argument on unpickling."""

    def __new__(cls):
        return super().__new__(cls)

    def __init__(self):
        super().__init__(5, 1.0, "n", False)


class Tagged(m.Point):
    """`__getnewargs_ex__` feeds `__new__` as in CPython."""

    def __new__(cls, tag, *, mark="-"):
        obj = super().__new__(cls)
        obj.tag = tag + mark
        return obj

    def __init__(self, tag, *, mark="-"):
        super().__init__(len(tag), 0.25, tag, True)

    def __getnewargs_ex__(self):
        return (self.label,), {"mark": "+"}


ss = SlotSub(2, 0.5, "ss", True)
ss.extra = [1, 2]
for proto in range(pickle.HIGHEST_PROTOCOL + 1):
    try:
        back = pickle.loads(pickle.dumps(ss, proto))
    except TypeError as e:
        print(proto, "TypeError", e)
    else:
        print(proto, type(back).__name__, back.show(), back.extra)
deep = copy.deepcopy(ss)
print(deep.show(), deep.extra, deep.extra is ss.extra)
bare = SlotSub(4, 1.0, "bare", False)
print(hasattr(pickle.loads(pickle.dumps(bare)), "extra"))
h = Hooked(3, 2.0, "h", False)
for back in (pickle.loads(pickle.dumps(h)), copy.deepcopy(h), pickle.loads(pickle.dumps(h, 0))):
    print(type(back).__name__, back.show())
n = Newed()
for back in (pickle.loads(pickle.dumps(n)), copy.deepcopy(n), pickle.loads(pickle.dumps(n, 1))):
    print(type(back).__name__, back.show())
tg = Tagged("tg")
# Protocols 0 and 1 make the object without `__new__` (`copyreg._reconstructor`).
for back in [pickle.loads(pickle.dumps(tg, proto)) for proto in range(6)] + [copy.deepcopy(tg)]:
    print(type(back).__name__, back.show(), back.tag)


class Args(m.Point):
    """`__getnewargs__`'s arguments reach the compiled class's own `__new__`."""

    def __getnewargs__(self):
        return (self.x,)


ar = Args(7, 3.5, "ar", False)
for back in [pickle.loads(pickle.dumps(ar, proto)) for proto in range(6)] + [copy.deepcopy(ar)]:
    print(type(back).__name__, back.show())
print(type(m.Point.__new__(m.Point, 1, 2)).__name__)


class Shifted(m.Point):
    """The state holds what is stored, not what `__getattribute__` answers."""

    x = "class-level"

    def __getattribute__(self, name):
        value = super().__getattribute__(name)
        return value + 1 if name == "x" else value


sh = Shifted(10, 0.5, "sh", True)
for back in (pickle.loads(pickle.dumps(sh)), copy.deepcopy(sh), pickle.loads(pickle.dumps(sh, 0))):
    print(type(back).__name__, back.x, object.__getattribute__(back, "x"))
fresh = m.Point.__new__(m.Point)
try:
    fresh.x
except AttributeError:
    print("AttributeError")
