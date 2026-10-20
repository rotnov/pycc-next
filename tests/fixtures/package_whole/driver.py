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

    for _ in range(100):
        base.label(3)
        circle.safe(-1)
        base.Point(1, 2).shifted(1).norm1()
    before = live()
    for _ in range(1000):
        base.label(3)
        circle.safe(-1)
        base.Point(1, 2).shifted(1).norm1()
    print("leaks", "none" if live() == before else f"{before} -> {live()}")
else:
    print("leaks", "none")
