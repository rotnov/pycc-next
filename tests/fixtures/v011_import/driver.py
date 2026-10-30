"""Imports `pkimp` from the tree in argv[1] and prints what Python code can
observe of it; the compiled tree must print what the sources print."""
import os
import sys
import traceback

sys.path.insert(0, sys.argv[1])
sys_before = set(dir(sys))
modules_before = set(sys.modules)

import pkimp
from pkimp import classes, errors

print("new sys attributes:", sorted(set(dir(sys)) - sys_before))
# The import machinery the import loads: the compiled package's finder takes
# `ModuleSpec` from the frozen import system rather than importing
# `importlib.machinery`.
print("new importlib modules:",
      sorted(m for m in set(sys.modules) - modules_before if m.startswith("importlib")))

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

# `from m import a, b, c` calls `__import__` once per statement, as
# `IMPORT_NAME` does: counted for `json` in compiled functions, including a
# statement whose second name is missing. (The module-level statement also
# runs in CPython, for the module namespace, so its count is not CPython's.)
import builtins

json_imports = []
real_import = builtins.__import__


def counting_import(name, *args, **kwargs):
    if name == "json":
        json_imports.append(tuple(args[2]) if len(args) > 2 and args[2] else None)
    return real_import(name, *args, **kwargs)


sys.path.insert(0, os.path.join(sys.argv[1], "vendor"))
from pkimp import imports

builtins.__import__ = counting_import
try:
    print("again", imports.again(), json_imports)
    del json_imports[:]
    print("failing", imports.failing(), json_imports)
    del json_imports[:]
    print("encoder", imports.encoder())
    # `a`'s lookup runs `again()`'s statement between the outer one's names.
    del json_imports[:]
    print("nested", imports.nested(), json_imports)
finally:
    builtins.__import__ = real_import
