"""Exercises package `pko` as CPython sees it, against its sources and against the
package built whole (`tests/v012_class_once.rs`); the outputs must match.
argv[1] is the directory holding `pko`."""
import copy
import pickle
import sys

sys.path.insert(0, sys.argv[1])

from pko import base, derived  # noqa: E402


# Classes whose statement runs in CPython, their methods compiled: their
# `__static_attributes__` misses what the compiled methods store
# (`docs/DEVIATIONS.md`).
IN_CPYTHON = {"Counter", "Far", "Farther"}


def mro(cls):
    """The MRO without the root every compiled class derives from and without
    the template a class made from one follows (`docs/DEVIATIONS.md`; the
    test checks both apart)."""
    names = []
    for c in cls.__mro__:
        name = c.__module__ + "." + c.__qualname__
        if c.__module__ != "pycc" and (not names or names[-1] != name):
            names.append(name)
    return names


def attributes(cls, value):
    return "-" if cls.__name__ in IN_CPYTHON else value


def info(cls):
    own = vars(cls)
    return [
        cls.__name__, cls.__qualname__, cls.__module__, repr(cls), repr(cls.__doc__), mro(cls),
        type(cls).__name__, cls.__firstlineno__, attributes(cls, cls.__static_attributes__),
        own["__module__"], own["__firstlineno__"], attributes(cls, own["__static_attributes__"]),
        "__qualname__" in own, "__init_subclass__" in own, "__orig_bases__" in own,
        "__classcell__" in own,
    ]


classes = [base.Plain, base.Explicit, base.Counter, base.Child, base.Holder, base.Kept,
           derived.Far, derived.Farther]
for cls in classes:
    print(*info(cls))

p, e, c, ch = base.Plain(3), base.Explicit("hi"), base.Counter(), base.Child(4)
f, ff = derived.Far(5), derived.Farther(6)
k = base.Kept(7)
objs = [p, e, c, ch, f, ff, k]
for obj in objs:
    print(type(obj).__name__, type(obj) is obj.__class__, repr(obj).split(" at ")[0])
print(p.value(), p.show(), base.total(p), base.total(ch), base.total(f), base.total(ff))
print(derived.both(p), derived.both(ch), derived.both(ff), f.tag(), f.tagged)
print(e.text(), e.kind, e.LIMIT, base.Explicit.kind, base.Explicit.LIMIT)
print(base.Explicit.template, e.template)
e.template = 3
print(e.template, base.Explicit.template)
print(base.counted(c), c.bump(), c.count, k.get(), k.again(), base.Holder[int](8).get())

pairs = [(ch, base.Plain), (ff, base.Child), (ff, base.Plain), (f, base.Child), (p, derived.Far),
         (k, base.Holder), (e, base.Plain)]
print([isinstance(o, cls) for o, cls in pairs])
print(issubclass(base.Child, base.Plain), issubclass(derived.Farther, base.Plain),
      issubclass(derived.Far, base.Child), issubclass(base.Plain, object),
      issubclass(base.Kept, base.Holder), issubclass(base.Plain, base.Child))

for obj in objs:
    for q in (pickle.loads(pickle.dumps(obj)), copy.copy(obj), copy.deepcopy(obj)):
        print(type(q).__name__, q is obj, sorted(
            (name, getattr(q, name)) for name in ("n", "label", "s", "count", "item")
            if hasattr(q, name)))

base.Plain.extra = 5
print(p.extra, ch.extra, base.Plain(1).extra, base.Child.extra)
del base.Plain.extra
print(hasattr(p, "extra"))


class Named:
    def __set_name__(self, owner, name):
        print("set_name", owner.__name__, name)


class PySub(base.Plain):
    marker = Named()

    def __init_subclass__(cls, **kwargs):
        print("init_subclass", cls.__name__, sorted(kwargs))
        super().__init_subclass__()

    def value(self):
        return 1000 + self.n


class PySubSub(PySub, flag=1):
    pass


s = PySubSub(2)
print(base.total(s), derived.both(s), s.show(), mro(PySubSub), isinstance(s, base.Plain))
print(PySub.__static_attributes__, PySub.__firstlineno__ > 0)
s.free = 1
# A compiled attribute is not in `vars()` (`docs/DEVIATIONS.md`).
print(s.free, vars(s)["free"])
try:
    class Bad(base.Plain, flag=1):
        pass
except TypeError as err:
    print("TypeError", err)
try:
    base.Plain()
except TypeError as err:
    print("TypeError", "__init__" in str(err) or "argument" in str(err))
try:
    base.Plain.missing
except AttributeError as err:
    print("AttributeError", err)
try:
    p.missing
except AttributeError as err:
    print("AttributeError", err)

# A class body's own `__doc__` and `__firstlineno__` win over the compiler's,
# and are not inherited.
for cls in (base.Documented, base.GenericDoc, base.Undocumented):
    print(cls.__name__, repr(cls.__doc__), cls.__firstlineno__, repr(vars(cls).get("__doc__")))
# Replacing a field's class entry leaves the instances' storage reachable.
fld = base.Field(3)
base.Field.n = 99
for q in (pickle.loads(pickle.dumps(fld)), copy.deepcopy(fld), copy.copy(fld)):
    print(type(q).__name__, q.get(), base.Field.n)

# A class made from its template over a base made once: the base's own
# instances still cross as the base.
made, mixed = base.make_plain(4), base.make_mixed(2)
print(type(made).__name__, isinstance(made, base.Plain), type(made) is base.Plain,
      isinstance(mixed, base.Plain), mixed.value(), base.total(mixed), mro(base.Mixed))
for q in (pickle.loads(pickle.dumps(made)), copy.deepcopy(mixed)):
    print(type(q).__name__, q.n, q.value())
