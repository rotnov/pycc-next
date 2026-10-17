"""Calls `untyped_readers` (imported as `m`) with the arguments its code
expects and with others, printing every result and exception."""
import struct
import sys
import zlib

import m


def show(label, f, *args, **kwargs):
    try:
        print(label, repr(f(*args, **kwargs)))
    except Exception as e:
        print(label, f"{type(e).__name__}: {e}")


class Weird:
    def __eq__(self, other):
        return other == 4

    def __hash__(self):
        return 4

    def __index__(self):
        return 1

    def __and__(self, other):
        return 3


m.main()
for x in (b"abc", [1, 2, 3], "abcd", (1,), bytearray(b"xyz"), None, 5):
    show("cut_both", m.cut_both, x)
for ct in (0, 2, 3, 6, True, 2.5, Weird(), None, "a"):
    show("planes", m.planes, ct)
for kind in (b"IHDR", b"\x01\x02ab", bytearray(b"HI"), b"", "IH", [73, 72], None):
    show("letters", m.letters, kind)
for bs, n in ((bytearray(b"\x01\x07"), 5), (b"\x07\x02", 300), ([7, 1], 2), (bytearray(), 1.5), ("ab", 1), (bytearray(b"\x07"), Weird()), (None, 1)):
    show("spread", m.spread, bs, n)
for n in (300, -1, 0, 255, True, 2.5, "x", None, 2 ** 70):
    show("store_bytes", m.store_bytes, n)
for o, w in (([1, "a"], 3), ("xy", None), (5, 1)):
    show("with_rows", lambda o, w: (lambda r: (r[0], list(r[1]), r[2]))(m.with_rows(o, w)), o, w)
for width, depth, rows in ((16, 1, [(0, b"\xa5\x0f"), (2, b"\x01\x01")]), (2, 8, [(1, b"\x01\x02"), (0, b"zz")]), (4, 2, [(0, b"\x1b")]), (3, 4, [(1, b"\x12\x34")])):
    data = m.encode(width, depth, rows)
    show("decode", m.decode, data)
    show("decode bytes", m.decode, bytes(data))
    show("decode bytearray", m.decode, bytearray(data))
for bad in (m.encode(2, 8, [(3, b"ab")]), m.encode(2, 8, [(0, b"ab")])[:20], b"", b"\x00\x00\x00\x02NOPEab", struct.pack("!I4s", 1, b"HEAD") + b"\x01", None, 5):
    show("decode bad", m.decode, bad)
r = m.Reader(m.encode(8, 8, [(0, b"abcdefgh")]))
show("process", r.process)
show("handler", lambda: callable(getattr(r, "_process_HEAD")))
show("width", lambda: (r.width, r.depth, r.row_bytes))
show("raw unset", lambda: r.raw)
show("process again", r.process)
show("raw", lambda: zlib.decompress(zlib.compress(r.raw)))
r.depth = 2.5
show("values float depth", r._values, bytearray(b"\x01"))
r.depth = "8"
show("values str depth", r._values, bytearray(b"\x01"))
r.depth = 4
show("values", r._values, bytearray(b"\x12\x34"), 3)
show("values width", r._values, bytearray(b"\x12"), None)
show("values list", r._values, [1, 2])
del r.depth
show("values unset depth", r._values, bytearray(b"\x01"))
show("undo bad", r.undo, 5, bytearray(b"ab"), None)
show("undo sub", r.undo, 1, bytearray(b"\x01\x02"), None)
show("undo up", r.undo, 2, bytearray(b"\x01\x02"), bytearray(b"\x10\x10"))
show("undo up short", r.undo, 2, bytearray(b"\x01\x02"), bytearray(b"\x10"))
sys.stdout.flush()
