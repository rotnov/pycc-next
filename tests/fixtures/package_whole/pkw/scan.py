"""Reads instances of another module's compiled class natively: its slots and its
compiled methods, in compiled functions, in a body whose `def` CPython runs, and in a
slot of a class of its own."""

from .cursor import Cursor


def words(c: Cursor) -> int:
    count = 0
    while not c.at_end():
        if c.peek(1) == " ":
            count += 1
            c.mark()
        c.pos += 1
    return count + 1


def same(c: Cursor) -> Cursor:
    return c


def marks(c: Cursor) -> list[int]:
    return [m * 10 for m in c.marks]


def child_of(c: Cursor) -> Cursor:
    k = c.child()
    k.pos = 2
    return k


def depth_chain(c: Cursor) -> int:
    n = 0
    p = c.parent
    while p is not None:
        n += 1
        p = p.parent
    return n


def keep(f):
    return f


class Walker:
    @keep
    def walk(self, c: Cursor, limit: int) -> str:
        out = ""
        while c.pos < limit and not c.at_end():
            out += c.peek(1).upper()
            c.pos += 1
        return out


class Holder:
    def __init__(self, c: Cursor) -> None:
        self.c = c

    def at(self) -> int:
        return self.c.pos
