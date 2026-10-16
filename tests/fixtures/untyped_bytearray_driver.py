"""Calls `untyped_bytearray` (imported as `m`) with the `bytearray`
arguments its guards expect and with others, printing every result,
exception and buffer afterwards."""
import array
import sys

import m


class B(bytearray):
    def __getitem__(self, i):
        return 7

    def __len__(self):
        return 2


def show(label, f, *args):
    try:
        r = f(*args)
        print(label, repr(r.tolist() if isinstance(r, memoryview) else r),
              [a for a in args if isinstance(a, bytearray)])
    except Exception as e:
        print(label, f"{type(e).__name__}: {e}", [a for a in args if isinstance(a, bytearray)])


def line(n, k):
    return bytearray((i * k + 3) & 0xFF for i in range(n))


for n in (0, 1, 3, 4, 9, 64):
    for fu in (1, 3):
        show("undo_sub", m.undo_sub, fu, line(n, 5), line(n, 5))
        show("paeth", m.paeth, fu, line(n, 7), line(n, 11), line(n, 7))
        # The result aliases the scanline, as pypng calls it.
        same = line(n, 13)
        show("paeth alias", m.paeth, fu, same, line(n, 3), same)
    # A previous line shorter than the result.
    show("paeth short", m.paeth, 1, line(n, 7), line(n // 2, 11), line(n, 7))
    show("checksum", m.checksum, line(n, 29))
# The object paths: other buffers, other sequences, a subclass.
for other in (bytes(line(5, 3)), [1, 2, 3, 4, 5], array.array("B", line(5, 3)), memoryview(line(5, 3)), B(b"abcd"), "abcde", None):
    show("checksum", m.checksum, other)
    show("undo_sub", m.undo_sub, 1, other, other)
    show("paeth", m.paeth, 1, other, line(5, 2), other)
show("undo_sub", m.undo_sub, True, line(4, 3), line(4, 3))
show("undo_sub", m.undo_sub, 1.5, line(4, 3), line(4, 3))
for i in (0, 3, -1, -4, 4, -5, 2 ** 40, 2 ** 70, -(2 ** 70)):
    for v in (0, 7, 254, 255, -1, -2, 2 ** 70, -(2 ** 70)):
        show("poke", m.poke, line(4, 9), i, v)
    show("peek", m.peek, line(4, 9), i)
show("poke", m.poke, line(4, 9), True, 3)
show("poke", m.poke, line(4, 9), 1, True)
show("poke", m.poke, line(4, 9), 1, 2.5)
for n in (0, 1, 5, 40, 41):
    show("grow", m.grow, line(40, 3), n)
show("grow", m.grow, line(0, 3), 1)
show("shrink", m.shrink, line(6, 5))
show("shrink", m.shrink, bytearray())
sys.stdout.flush()
