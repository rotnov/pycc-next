"""Drives `object_ownership.py`, compiled or not: `python3 driver.py MODULE N`.

Prints each function's result and the change in the arguments' reference
counts after N calls (0 under CPython), then whether traced memory grew.
"""

import importlib
import sys
import tracemalloc

m = importlib.import_module(sys.argv[1])
n = int(sys.argv[2])


class Thing:
    pass


big = 10**30
obj = Thing()
other = [1, 2]
seq = (big, obj, "s")


def probe(name, *args):
    f = getattr(m, name)
    before = [sys.getrefcount(a) for a in args]
    result = None
    for _ in range(n):
        result = f(*args)
    shown = type(result).__name__ if isinstance(result, (Thing, list)) else result
    del result
    after = [sys.getrefcount(a) for a in args]
    print(name, shown, [a - b for a, b in zip(after, before)])


def failing(name, *args):
    f = getattr(m, name)
    before = [sys.getrefcount(a) for a in args]
    for _ in range(n):
        try:
            f(*args)
        except AttributeError as e:
            message = str(e)
    after = [sys.getrefcount(a) for a in args]
    print(name, message, [a - b for a, b in zip(after, before)])


def run():
    probe("ident", obj)
    probe("keep", obj)
    probe("add", big, big)
    probe("via_holder", obj)
    probe("rebind_attr", obj, other)
    probe("boxed", 7)
    probe("pick", obj, other, True)
    probe("pick", obj, other, False)
    probe("either", obj, other)
    probe("either", 0, other)
    probe("items", seq)
    probe("passes", obj)
    probe("nested_call", big)
    failing("raises", obj)
    probe("stores", obj, 3)


run()
tracemalloc.start()
run_snapshot = tracemalloc.get_traced_memory()[0]
for _ in range(3):
    run()
grown = tracemalloc.get_traced_memory()[0] - run_snapshot
tracemalloc.stop()
print("traced memory flat", grown < 64 * 1024)

# A million calls through the boundary: the resident set stays flat.
import resource

before = resource.getrusage(resource.RUSAGE_SELF).ru_maxrss
counts = [sys.getrefcount(a) for a in (big, obj, other)]
for _ in range(1_000_000):
    m.add(big, big)
    m.via_holder(other)
    m.passes(obj)
after = resource.getrusage(resource.RUSAGE_SELF).ru_maxrss
print("rss flat", after - before < 4096)
print("counts flat", counts == [sys.getrefcount(a) for a in (big, obj, other)])
