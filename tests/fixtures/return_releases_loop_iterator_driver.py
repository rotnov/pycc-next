"""Drives `return_releases_loop_iterator` from the directory in argv[1]."""

import gc
import sys

sys.path.insert(0, sys.argv[1])
import return_releases_loop_iterator as rl  # noqa: E402

CALLS = 100


def list_iterators():
    gc.collect()
    return sum(1 for o in gc.get_objects() if type(o).__name__ == "list_iterator")


def measure(name, *args):
    before = list_iterators()
    for _ in range(CALLS):
        result = getattr(rl, name)(*args)
    print(name, "leaked", list_iterators() - before, "->", result)


measure("early", [{"type": "x"}, {"type": "text"}])
measure("early", [{"type": "x"}])
for name in ("early_void", "try_in_loop", "loop_in_try", "overridden"):
    log = []
    measure(name, [1, 2, 3], log)
    print(" ", log[:4], len(log))
for name in ("nested", "between"):
    log = []
    measure(name, [1, 2], [3, 4], log)
    print(" ", log[:4], len(log))
