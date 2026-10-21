# v0.9: Python subclasses of `v09_pysub`'s compiled class: compiled calls run
# their overrides, their objects keep their state while compiled code holds
# their instances, and the garbage collector frees them when it does not
# (`tests/v09_native_state.rs` imports the fixture as `m`, compiled or not).
import copy
import gc
import weakref

import m


class Py(m.Base):
    def __init__(self, n, extra):
        super().__init__(n)
        self.extra = extra

    def name(self):
        return "py-" + self.extra

    @property
    def label(self):
        return "P" + str(self.n)


class Plain(m.Base):
    pass


class Deeper(Py):
    def size(self):
        return 100 + super().size()


p = Py(3, "x")
print(p.show(), m.show_it(p), m.Base.show(p), p.extra, isinstance(p, m.Base))
print(Plain(2).show(), m.show_it(Plain(4)), type(Plain(1)).__mro__[1] is m.Base)
print(m.show_it(Deeper(7, "d")), [c.__name__ for c in Deeper.__mro__[:3]])
c = copy.copy(p)
print(type(c).__name__, c.extra, c.show(), c is p)
r = weakref.ref(Py(5, "gone"))
gc.collect()
print("collected", r() is None)
q = Py(6, "kept")
m.keep(q)
r2 = weakref.ref(q)
del q
gc.collect()
print("held", r2() is not None, r2().extra, m.held_show())
m.release()
gc.collect()
print("released", r2() is None)
