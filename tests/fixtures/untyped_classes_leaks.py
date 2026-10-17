"""Calls the functions of `untyped_classes` (imported as `m`) many times,
through their successful and raising paths, and checks that neither
pycc's runtime objects nor CPython's allocated blocks grow."""
import ctypes
import gc
import sys
import warnings
import zlib

import m

lib = ctypes.CDLL(m.__file__)


def counter(name):
    f = getattr(lib, name)
    f.restype = ctypes.c_int64
    f.argtypes = []
    return f


counters = [counter("pycc_rt_%s_live_objects" % k) for k in ("str", "list", "instance", "exception", "closure")]


def live():
    gc.collect()
    return [c() for c in counters] + [sys.getallocatedblocks()]


data = zlib.compress(b"hello world" * 3)
warnings.simplefilter("ignore")


def attempt(f, *args):
    try:
        f(*args)
    except Exception:
        pass


def round_trip():
    m.tag(b"IEND")
    m.tag(b"zTXt")
    m.literals()
    m.group([1, 2, 3, 4], 2)
    m.chain([b"ab", b"cd"])
    m.pack("!4B", (1, 2), 3)
    attempt(m.pack, "!4B", 5, 3)
    attempt(m.pack, "!4B", (1,), 3)
    m.call_mixed(m.target, 0, (1, 2), {"z": 3})
    attempt(m.call_mixed, m.target, 0, (), {"key": 1})
    attempt(m.call_mixed, m.target, 0, (), 5)
    m.unpack_word(b"abcd")
    m.unpack_word(b"ab")
    m.classify(b"ab")
    m.classify(b"abcd")
    attempt(m.unmatched, b"x")
    attempt(m.not_a_class, 0)
    m.filter_kind(3)
    m.filter_kind(9)
    m.row_count(10, 3)
    m.row_count(7.5, 2)
    m.warn_if(1)
    list(m.decompress([data[:5], data[5:]]))
    m.pick(1, 2, 3)
    attempt(m.pick, 0, 2, 3)
    m.apply(m.sub, 5)
    m.apply(max, 5)
    d = m.Decoder(b"\x00\x02\x00\x03abcdef")
    d.parse()
    list(d.chunks())
    d.scaled(2)
    d.rows([1, 2, 3, 4])
    d.rows([5], True)
    d.wide_rows([[1, 2]])
    attempt(m.Decoder(b"\x00\x00\x00\x01").parse)
    attempt(m.Decoder().size)


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
assert grown[:5] == [0, 0, 0, 0, 0] and grown[5] < 1000, (before, after)
print("ok")
