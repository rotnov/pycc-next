"""Exercises package `pkw` as CPython sees it; run against the sources and against the
package built whole into one extension (`tests/package_whole.rs`), the outputs must match.
argv[1] is the directory holding `pkw`."""
import importlib
import os
import pickle
import sys

root = sys.argv[1]
sys.path.insert(0, root)

import pkw  # noqa: E402

compiled = pkw.__file__.endswith((".so", ".pyd"))


def where(m):
    """A module's file relative to the tree, without its suffix."""
    rel = os.path.relpath(m.__file__, root).replace(os.sep, "/")
    return rel.split(".")[0]


def show(m):
    spec = m.__spec__
    path = getattr(m, "__path__", None)
    print(m.__name__, where(m), spec.name, spec.parent, m.__package__,
          [os.path.relpath(p, root).replace(os.sep, "/") for p in path] if path else None,
          spec.has_location, spec.loader is m.__loader__)


print("doc", pkw.__doc__)
for name in ["pkw", "pkw.base", "pkw.sub", "pkw.sub.circle"]:
    show(sys.modules[name])
from pkw.sub import circle  # noqa: E402
from pkw import base  # noqa: E402

c = circle.Circle(2.0)
print(c.describe(), isinstance(c, pkw.Shape), issubclass(circle.Circle, base.Shape))
print(pkw.area_sum([c, circle.Circle(1.0)]), pkw.describe_all([c]))
try:
    base.Shape("x")
except TypeError as e:
    print("abstract", type(e).__name__)
print(circle.IntBox(21).doubled(), base.Box("s").get())
print(circle.safe(3), circle.safe(-1), circle.safe(200))
print(circle.safe_value(-2), circle.safe_value(500))
print(circle.safe_plain(-2), circle.safe_plain(500), circle.safe_plain(1))
try:
    circle.safe_key(-3)
except ValueError as e:
    print("not a key error", type(e).__name__)
try:
    base.check(-5)
except ValueError as e:
    print("raised", type(e).__name__, type(e).__module__, e)
print(circle.current_version(), base.label(4), circle.far(base.Point(1, -2)))
print(base.Point.__module__, base.Point.__qualname__, base.check.__module__,
      circle.Circle.__module__)

# Pickling by reference: functions, classes, exceptions (round trips of data this
# script made itself).
print(pickle.loads(pickle.dumps(base.check))(4))
print(pickle.loads(pickle.dumps(base.BaseError)) is base.BaseError)
err = pickle.loads(pickle.dumps(base.DeepError("x")))
print(type(err).__name__, err.args)
print(pickle.loads(pickle.dumps(circle.Circle(3.0))).area())

# Data files beside the modules (`importlib.resources`).
import importlib.resources  # noqa: E402

print(importlib.resources.files("pkw.sub").joinpath("data.txt").read_text().strip())
print(importlib.resources.files("pkw").joinpath("sub", "data.txt").is_file(),
      importlib.resources.files("pkw.sub.circle").joinpath("data.txt").is_file())

# PEP 562.
print(pkw.sub.lazy)
try:
    pkw.sub.missing
except AttributeError as e:
    print("attr", e)

# Circular imports, in import order.
import pkw.cyc_a  # noqa: E402

print(pkw.cyc_a.a_func(), pkw.cyc_b.uses_a())

# State, rebinding from Python, reload.
import pkw.state as state  # noqa: E402

print(state.bump(), state.bump(), state.limit())
state.LIMIT = 7
print(state.limit(), state.COUNTER)
again = importlib.reload(state)
print(again is state, state.COUNTER, state.bump(), state.limit())
print(importlib.import_module("pkw.sub.circle") is circle)

# Calls into other modules (native in the compiled tree while the names bind the
# published functions), then through rebound names, then after a reload.
import traceback  # noqa: E402

import pkw.calls as calls  # noqa: E402
import pkw.util as util  # noqa: E402

print(calls.use(2, "ab"), calls.loop(10), calls.keyword("x"), calls.private("p"))
print(calls.token("u", [1]))
for call, arg in [(calls.failing, "k"), (calls.div, 0.0)]:
    try:
        call(arg) if call is calls.failing else call(1.0, arg)
    except (KeyError, ZeroDivisionError) as e:
        print(type(e).__name__, e, [f.name for f in traceback.extract_tb(e.__traceback__)][1:])
calls.shout = lambda s: "patched " + s
calls._shout = lambda s: "hidden " + s
print(calls.use(1, "q"), calls.private("p"), calls.keyword("y"))
calls.shout = util.shout
calls._shout = util.shout
print(calls.use(1, "r"), calls.private("s"))
reloaded = importlib.reload(util)
print(reloaded is util, calls.scaled is util.scaled, calls.use(3, "t"), calls.loop(4))

# A class another module subclasses through a third module's re-export.
import pkw.sub.tagged as tagged  # noqa: E402

print(tagged.Bold("x").show(), base.Tag("y").show(), isinstance(tagged.Bold("z"), base.Tag))

# Instances of another module's class, read natively in the compiled tree.
import pkw.cursor as cursor  # noqa: E402
import pkw.scan as scan  # noqa: E402

cur = cursor.make("one two  three")
print(scan.words(cur), cur.pos, scan.marks(cur), scan.same(cur) is cur)
kid = scan.child_of(cur)
print(type(kid) is cursor.Cursor, kid.depth, kid.pos, kid.parent is cur, scan.depth_chain(kid),
      scan.depth_chain(scan.child_of(kid)), scan.child_of(kid).parent is kid)
fresh = cursor.Cursor("walk this")
print(scan.Walker().walk(fresh, 4), fresh.pos, scan.Walker().walk(fresh, 99), fresh.at_end())
holder = scan.Holder(fresh)
print(holder.at(), holder.c is fresh)
holder.c = cur
print(holder.at(), holder.c is cur)
for bad in (5, "text", None):
    try:
        scan.words(bad)
    except (TypeError, AttributeError):
        print("refused", type(bad).__name__)

try:
    import pkw.missing  # noqa: F401
except ModuleNotFoundError as e:
    print("missing", e)
try:
    from pkw import nothing  # noqa: F401
except ImportError as e:
    print("from", type(e).__name__)

# Leak probe: the compiled tree frees what it allocates.
if compiled:
    import ctypes

    lib = ctypes.CDLL(pkw.__file__)

    def live():
        return (lib.pycc_rt_str_live_objects(), lib.pycc_rt_instance_live_objects(),
                lib.pycc_rt_exception_live_objects())

    def work():
        base.label(3)
        circle.safe(-1)
        base.Point(1, 2).shifted(1).norm1()
        calls.private("x")
        calls.div(1.0, 2.0)
        c = cursor.make("a b c")
        scan.words(c)
        scan.depth_chain(scan.child_of(scan.child_of(c)))
        scan.Walker().walk(cursor.Cursor("xyz"), 2)
        scan.Holder(c).at()

    for _ in range(100):
        work()
    before = live()
    for _ in range(1000):
        work()
    print("leaks", "none" if live() == before else f"{before} -> {live()}")
else:
    print("leaks", "none")
