# v0.9: Python code storing into and reading `v09_defaults`'s class-level
# defaults through instances, the class and subclasses whose `__init__`
# does not call the class's (`tests/v09_native_state.rs`).
import copy
import gc
import weakref

from m import Grid, Spec, describe

s = Spec()
s.template = "tpl"
print(s.template, s.encoding, Spec.template, describe(s))


class View(Spec):
    def __init__(self, name):
        self.name = name


v = View("v")
print(v.template, v.name, describe(v))
v.encoding = "utf-8"
print(describe(v), View.encoding, copy.copy(v).encoding)
g = Grid(3)
g.label = "x"
print(g.show(), Grid.label, g.label)
r = weakref.ref(View("gone"))
gc.collect()
print(r() is None)


class Named(Spec):
    template = "hello"


class NamedView(Spec):
    template = "hi"

    def __init__(self):
        self.extra = 1


print(Named().template, NamedView().template, describe(Named()), describe(NamedView()))
