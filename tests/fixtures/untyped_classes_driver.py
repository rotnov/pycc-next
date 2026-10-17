"""Calls `untyped_classes` (imported as `m`) with the arguments its code
expects and with others, printing every result and exception."""
import struct
import sys
import warnings

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

    def __iter__(self):
        raise TypeError("no iterating Weird")

    def __ceil__(self):
        return 99

    def __sub__(self, other):
        return self

    def __truediv__(self, other):
        return self


class Mapping:
    def keys(self):
        return ["a", "b"]

    def __getitem__(self, key):
        return key.upper()


m.main()
for t in (b"IEND", b"IDAT", b"", bytearray(b"IEND"), "IEND", None, 4, Weird()):
    show("tag", m.tag, t)
show("literals", m.literals)
for s, n in (([1, 2, 3, 4], 2), ("abcdef", 3), (b"abcd", 2), (range(5), 2), ([], 1), (5, 1), ([1], 0)):
    show("group", m.group, s, n)
for blocks in ([b"ab", b"cd"], [], [[1, 2], (3,)], (b"x" for _ in range(2)), [b"a", 5], 5, Weird()):
    show("chain", m.chain, blocks)
for fmt, values, last in (("!4B", (1, 2), 3), ("!3B", [1], 2), ("!B", (), 1), ("!2B", "a", 1), ("!3B", None, 1), ("!4B", (1, 2), 300), ("!3B", b"\x01", 2)):
    show("pack", m.pack, fmt, values, last)
show("forward", m.forward, m.target, 1, 2, x=3)
show("forward", m.forward, print, "printed", sep="-", end="!\n")
show("forward", m.forward, max, [3, 1, 2], key=lambda v: -v)
show("forward", m.forward, 5)
show("forward", m.forward, m.target, **{"g": 1})
for a, rest, d in ((0, (1, 2), {"z": 3}), (0, [], {}), (0, 5, {}), (0, (), 5), (0, (), {"key": 2}), (0, (), {1: 2}), (0, iter([4, 5]), Mapping()), (0, Weird(), {})):
    show("call_mixed", m.call_mixed, m.target, a, rest, d)
show("call_mixed", m.call_mixed, len, 0, (), {})
for data in (b"abcd", b"ab", bytearray(b"\x00\x00\x00\x07"), memoryview(b"wxyz"), "abcd", None, 5, b""):
    show("unpack_word", m.unpack_word, data)
    show("classify", m.classify, data)
    show("unmatched", m.unmatched, data)
for x in (0, 1, 2.5, None, "s"):
    show("not_a_class", m.not_a_class, x)
for k in (1, 2, 3, 4, 5, 0, 4.0, 2.0, True, False, "a", None, Weird(), [1], 2 ** 70):
    show("filter_kind", m.filter_kind, k)
for w, s in ((10, 3), (7.5, 2), (2 ** 70, 1), (10, 0), ("a", 1), (Weird(), 1), (10, "b"), (float("nan"), 1), (float("inf"), 1)):
    show("row_count", m.row_count, w, s)
# A warning names the frame that issued it, which compiled code does not
# have (docs/DEVIATIONS.md): raised as an error, it compares.
warnings.simplefilter("error")
for x in (0, 1, "", "x", None):
    show("warn_if", m.warn_if, x)
for blocks in ([], [b""], [b"abc"], [5], None):
    show("decompress", lambda b: list(m.decompress(b)), blocks)
sys.stdout.flush()
for k, x, y in ((1, 2, 3), (2, 2.5, 1), (1, "a", "b"), (0, 1, 2), (3, 1, 2), ("1", 1, 2)):
    show("pick", m.pick, k, x, y)
for f in (m.sub, m.add, max, lambda a, b: (a, b), 5):
    show("apply", m.apply, f, 3)
for data in (b"\x00\x02\x00\x03abcdef", bytearray(b"\x00\x01\x00\x01xyz"), b"\x00\x00\x00\x01", b"ab", "abcd", None):
    d = m.Decoder(data)
    show("parse", d.parse)
    show("parse lenient", d.parse, strict=False)
    show("chunk", d.chunk, size=1)
    show("chunks", lambda: list(d.chunks()))
    show("scaled", d.scaled, 3)
    show("rows", d.rows, [1, 2, 3, 4])
    show("rows flat", d.rows, (5, 6), True)
    show("wide_rows", d.wide_rows, [[1, 2], [3]])
    show("wide_rows bad", d.wide_rows, [[300]])
show("wide", m.Decoder(b"\x00\x01\x00\x01", wide=True).wide_rows, [[1, 2]])
show("no data", lambda: m.Decoder().size())
sys.stdout.flush()
