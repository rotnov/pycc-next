"""Every `object` slot owns one reference (compiled as `--ext --non-strict`).

Each function crosses the extension boundary in both directions: the host
passes objects in, and objects come back. `tests/object_ownership.rs` calls
each one many times and checks that reference counts and memory stay flat.
"""


class Holder:
    o: object

    def __init__(self, o: object) -> None:
        self.o = o


def ident(o: object) -> object:
    return o


def keep(o: object) -> int:
    x = o
    y = x
    x = y
    return 1


def add(a: object, b: object) -> object:
    c = a + b
    c = c - b
    return c + a


def via_holder(o: object) -> object:
    h = Holder(o)
    y = h.o
    h = Holder(1)
    return y


def rebind_attr(o: object, p: object) -> object:
    h = Holder(o)
    h.o = p
    h.o = h.o
    return h.o


def boxed(n: int) -> object:
    x: object = n
    x = "s"
    return x


def pick(o: object, p: object, flag: bool) -> object:
    return o if flag else p


def either(o: object, p: object) -> object:
    return o or p


def items(o: object) -> object:
    return [x for x in o]


def passes(o: object) -> object:
    return ident(ident(o))


def nested_call(o: object) -> object:
    return ident(o).real


def raises(o: object) -> object:
    h = Holder(o)
    y = h.o
    return y.missing


def stores(o: object, n: int) -> object:
    h = Holder(o)
    i = 0
    while i < n:
        h.o = i * 2
        h.o = o
        i = i + 1
    return h.o
