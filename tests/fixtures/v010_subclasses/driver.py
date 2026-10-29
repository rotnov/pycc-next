"""Exercises package `pks` as CPython sees it, against its sources and against the
package built whole (`tests/v010_subclasses.rs`); the outputs must match.
argv[1] is the directory holding `pks`."""
import copy
import gc
import pickle
import sys

sys.path.insert(0, sys.argv[1])

import pks  # noqa: E402
from pks import base, more, shapes  # noqa: E402

compiled = pks.__file__.endswith((".so", ".pyd"))


def mro(cls):
    """The MRO without the compiled templates (`docs/DEVIATIONS.md`)."""
    seen = []
    for c in cls.__mro__:
        name = c.__module__ + "." + c.__qualname__
        if name not in seen and name != "pycc.compiled":
            seen.append(name)
    return seen


print(shapes.run())
sq = shapes.Square("a", 3)
r = shapes.Rect("b", 2, 7)
cube = more.Cube("c", 2)
for obj in (sq, r, cube, base.Shape("d", 5)):
    print(type(obj).__name__, obj.describe(), base.describe(obj), shapes.kinds(obj),
          base.weigh(obj), obj.name, obj.size)
print(isinstance(sq, base.Shape), isinstance(r, shapes.Square), isinstance(cube, base.Shape),
      isinstance(base.Shape("e", 1), shapes.Square), issubclass(shapes.Rect, base.Shape))
print(mro(shapes.Rect), mro(more.Cube))
print(base.Shape.describe(r), shapes.Square.kind(r), base.Shape.kind(r), base.total(sq, cube))
print(r.side, r.h, sq.side, sorted(vars(sq).items()))
r.side = 4
r.size = 9
print(r.describe(), base.describe(r), r.area())
c = copy.copy(r)
print(type(c).__name__, c.describe(), c is r)
for obj in (sq, r, cube):
    p = pickle.loads(pickle.dumps(obj))
    print(type(p).__name__, p.describe(), base.describe(p), p.side, p.size, p is obj)
d = shapes.Doubled(5)
print(d.value(), base.Plain.value(d), isinstance(d, base.Plain))
print(shapes.Doubled(n=4).value(), shapes.Square(name="kw", side=2).describe())
f = shapes.Fixed("f", 2)
print(f.describe(), base.describe(f), base.weigh(f), shapes.kinds(f), f.kind(), f.factor())


class Tri(shapes.Square):
    def kind(self):
        return "tri"


t = Tri("t", 4)
print(t.describe(), base.describe(t), shapes.kinds(t), base.weigh(t), mro(Tri))
try:
    shapes.Square("x")
except TypeError as e:
    print("TypeError", "__init__" in str(e) or "argument" in str(e))

# Leak probe: the compiled tree frees what it allocates.
if compiled:
    import ctypes

    lib = ctypes.CDLL(pks.__file__)

    def live():
        # A Python subclass's object and the instance it carries are a
        # cycle the collector frees.
        gc.collect()
        return (lib.pycc_rt_str_live_objects(), lib.pycc_rt_instance_live_objects())

    def work():
        shapes.run()
        s = shapes.Rect("w", 1, 2)
        base.describe(s)
        shapes.kinds(more.Cube("q", 1))
        base.weigh(s)

    for _ in range(100):
        work()
    before = live()
    for _ in range(1000):
        work()
    print("leaks", "none" if live() == before else f"{before} -> {live()}")
else:
    print("leaks", "none")
