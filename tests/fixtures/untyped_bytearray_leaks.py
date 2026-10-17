"""Calls the specialized functions of `untyped_bytearray` (imported as `m`)
many times, on their native and object paths and through their errors, and
checks that neither pycc's runtime objects nor CPython's allocated blocks
grow."""
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


base = bytearray((i * 7 + 3) & 0xFF for i in range(300))


def attempt(f, *args):
    try:
        f(*args)
    except (IndexError, ValueError, TypeError):
        pass


def round_trip():
    line = bytearray(base)
    m.undo_sub(3, line, line)
    m.paeth(3, bytearray(base), base, line)
    m.checksum(line)
    m.peek(line, 5)
    m.poke(line, 5, 9)
    m.grow(bytearray(base[:20]), 20)
    m.shrink(bytearray(base[:10]))
    attempt(m.poke, line, 400, 1)
    attempt(m.poke, line, 1, 2 ** 70)
    attempt(m.poke, line, 2 ** 70, 1)
    attempt(m.peek, line, -400)
    m.unpack(bytearray(base[:40]), 2, 0)
    attempt(m.unpack, bytearray(base[:40]), -2, 3)
    attempt(m.unpack, bytearray(base[:40]), 4, 250)
    # The object path.
    m.checksum(bytes(base))
    attempt(m.undo_sub, 1, bytes(base), bytes(base))


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
