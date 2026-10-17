"""Calls the functions of `int_ranges_api` many times, with smallints,
bigints and failing arguments, and checks that neither pycc's runtime
objects nor CPython's allocated blocks grow."""
import ctypes
import gc
import sys

import int_ranges_api as m

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


def round_trip():
    m.mix(17, 200)
    m.mix(2**70, 3)
    m.window(-70, 70)
    m.window(2**65, 2**65 + 3)
    m.scale(2**64 + 7)
    for bad in ((1.5, 3), ("a", 3)):
        try:
            m.mix(*bad)
        except TypeError:
            pass


for _ in range(100):
    round_trip()
before = live()
for _ in range(300):
    round_trip()
after = live()
grown = [b - a for a, b in zip(before, after)]
assert grown[:3] == [0, 0, 0] and grown[3] < 300, (before, after)
print("ok")
