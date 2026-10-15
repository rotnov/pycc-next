"""Calls the specialized functions of `untyped_specialize` (imported as `m`)
many times, on their native and their object paths, and checks that neither
pycc's runtime objects nor CPython's allocated blocks grow."""
import ctypes
import gc
import sys

import m

lib = ctypes.CDLL(m.__file__)


def counter(name):
    f = getattr(lib, name)
    f.restype = ctypes.c_int64
    f.argtypes = []
    return f


counters = [counter("pycc_rt_%s_live_objects" % k) for k in ("str", "list", "instance")]


def live():
    gc.collect()
    return [c() for c in counters] + [sys.getallocatedblocks()]


t = m.Text()
texts = ["héllo wörld " * 3, "abc" * 20, "日本語"]


def round_trip():
    for s in texts:
        t.common_prefix(s, s[:-1] + "!")
        t.common_suffix(s, "x" + s[1:])
        t.scan(s, 2)
        m.windows(s, 3)
        m.stats(s, "l")
        m.split_words(s, " ")
        m.total(len(s))
        m.char_at(s, 1)
        # The object path.
        m.windows(list(s), 2)
        t.common_prefix(list(s), list(s))


# CPython's allocator and caches settle first.
for _ in range(100):
    round_trip()
before = live()
for _ in range(300):
    round_trip()
after = live()
grown = [b - a for a, b in zip(before, after)]
# A leak of one object per call would add thousands of blocks.
assert grown[:3] == [0, 0, 0] and grown[3] < 300, (before, after)
print("ok")
