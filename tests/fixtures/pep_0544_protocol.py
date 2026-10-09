# PEP 544 (#380): Protocols and structural typing.
#
# Exercises: a @runtime_checkable protocol with a method requirement,
# two conforming concrete classes, a non-conforming class, protocol-typed
# variable assignments with method calls, isinstance against the
# runtime_checkable protocol (True for conforming, False for
# non-conforming), and a protocol-typed function parameter.
#
# Both pycc and CPython 3.14.7 produce identical stdout: the protocol is
# a compile-time-only interface in pycc and a deferred-annotation type
# hint in CPython, so neither emits anything for the protocol definition
# itself — only the concrete class method calls and isinstance results
# produce observable output, and those match byte-for-byte.
from typing import Protocol, runtime_checkable


@runtime_checkable
class Drawable(Protocol):
    def draw(self) -> str:
        ...


class Circle:
    def __init__(self) -> None:
        self.x = 0

    def draw(self) -> str:
        return "circle"


class Square:
    def __init__(self) -> None:
        self.x = 0

    def draw(self) -> str:
        return "square"


class NotDrawable:
    def __init__(self) -> None:
        self.x = 0


c: Drawable = Circle()
print(c.draw())

s: Drawable = Square()
print(s.draw())

print(isinstance(c, Drawable))
print(isinstance(s, Drawable))

nd = NotDrawable()
print(isinstance(nd, Drawable))


def draw_it(d: Drawable) -> None:
    print(d.draw())


draw_it(Circle())
draw_it(Square())
