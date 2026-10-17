"""Calls the functions of `untyped_readers` (imported as `m`) many times,
through their successful and raising paths, and checks that neither
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


counters = [
    counter("pycc_rt_%s_live_objects" % k)
    for k in ("str", "list", "instance", "exception", "closure", "gen", "set")
]


def live():
    gc.collect()
    return [c() for c in counters] + [sys.getallocatedblocks()]


def attempt(f, *args):
    try:
        f(*args)
    except Exception:
        pass


one_bit = m.encode(16, 1, [(0, b"\xa5\x0f"), (1, b"\x01\x01"), (2, b"\x10\x01")])
eight_bit = m.encode(3, 8, [(1, b"\x01\x02\x03"), (2, b"\x05\x05\x05"), (0, b"abc")])
bad_filter = m.encode(2, 8, [(3, b"ab")])


def round_trip():
    m.cut_both(b"abc")
    m.cut_both([1, 2, 3])
    m.planes(3)
    m.letters(b"IHDR")
    m.spread(bytearray(b"\x01\x07\x03"), 5)
    attempt(m.spread, bytearray(b"\x01"), 300)
    m.store_bytes(0)
    attempt(m.store_bytes, None)
    w, rows, info = m.with_rows([1, "a"], 3)
    list(rows)
    m.decode(one_bit)
    m.decode(eight_bit)
    attempt(m.decode, bad_filter)
    attempt(m.decode, eight_bit[:20])
    r = m.Reader(one_bit)
    r.process()
    r._values(bytearray(b"\x12\x34"), 3)
    attempt(r.undo, 5, bytearray(b"ab"), None)


# CPython's allocator and caches settle first.
for _ in range(100):
    round_trip()
before = live()
for _ in range(1000):
    round_trip()
after = live()
grown = [b - a for a, b in zip(before, after)]
# A leak of one object per round would add a thousand blocks, one per call
# tens of thousands; what CPython itself allocates once varies by a few
# hundred between runs.
assert grown[:7] == [0] * 7 and grown[7] < 1000, (before, after)
print("ok")
