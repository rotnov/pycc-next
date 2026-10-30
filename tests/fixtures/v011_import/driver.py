"""Imports `pkimp` from the tree in argv[1] and prints what Python code can
observe of it; the compiled tree must print what the sources print."""
import os
import sys
import traceback

sys.path.insert(0, sys.argv[1])
sys_before = set(dir(sys))

import pkimp
from pkimp import classes, errors

print("new sys attributes:", sorted(set(dir(sys)) - sys_before))

# A module's namespace names its builtins as `exec` stores them: the dict.
for mod in (pkimp, classes, errors):
    print(mod.__name__, "__builtins__", type(vars(mod).get("__builtins__")).__name__)

# Published classes made from their statements. The template every
# published class derives from (module `pycc`) is a documented deviation.
for cls in (classes.Plain, classes.Shape, classes.Square, classes.Boxed, classes.Loader,
            classes.Abstract):
    print(cls.__name__, cls.__qualname__, cls.__module__, type(cls).__name__)
    print("  orig_bases:", getattr(cls, "__orig_bases__", None))
    print("  bases:", [b.__name__ for b in cls.__mro__
                       if b.__module__ not in (cls.__module__, "pycc")])
print("Plain", classes.Plain(3).get())
print("Square", classes.Square(3).area(), classes.Square(3).perimeter(),
      isinstance(classes.Square(1), classes.Shape))
print("Boxed", classes.Boxed(7).peek(), classes.Boxed[int].__origin__ is classes.Boxed)
loader = classes.Loader("main")
print("Loader", classes.Loader.template, classes.Loader.template_name, loader.describe())
loader.template = "t"
print("Loader set", loader.template, loader.describe())
del loader.template
print("Loader del", loader.template)
print("Abstract", classes.Abstract(5).twice())


class Sub(classes.Loader):
    template = "sub"


print("Sub", Sub("s").describe(), Sub.template)

# Compiled frames: their names and order (the compiled tree has no source
# lines to show).
print("outer", errors.outer(4))
try:
    errors.outer(-2)
except ValueError as exc:
    tb = traceback.extract_tb(exc.__traceback__)
    print("traceback", exc, [f.name for f in tb], tb[-1].filename == os.path.join(os.path.dirname(errors.__file__), "errors.py"))
print("caller", errors.caller_name())
