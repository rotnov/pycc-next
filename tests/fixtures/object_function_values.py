"""Untyped code (`--non-strict`) using module functions as values and
testing objects against constant tuples, as pypng's `undo_filter` does: the
callable a function crosses as is one object, called with positional
arguments only; a constant display a membership test reads is built once."""


def add(a, b):
    return a + b


def sub(a, b):
    return a - b


def neg(a):
    return -a


class Loud:
    def __init__(self, v):
        self.v = v

    def __eq__(self, other):
        print("eq", self.v, other)
        return self.v == other


def pick(k, a, b):
    fn = (None, add, sub)[k]
    return fn(a, b)


def same(k):
    return (None, add, sub)[k] is (add, add, sub)[k]


def kind(x):
    if x not in (1, 2, 3, 4):
        return "other"
    if x in ["a", 2.5, None, True]:
        return "listed"
    return "small"


def call_bad(f, *args, **kw):
    try:
        return f(*args, **kw)
    except TypeError as e:
        return type(e).__name__


def main():
    for k in (1, 2, 1):
        print(pick(k, 7, 3), pick(k, "x", "y") if k == 1 else pick(k, 2.5, 1))
    print(same(1), same(2))
    for x in (1, 4, 5, 0, "a", 2.5, None, True, False, 3.0, Loud(3), Loud(9)):
        print(repr(x) if not isinstance(x, Loud) else x.v, kind(x))
    print(call_bad(pick, 1, add))
    fns = [add, sub, neg]
    print([f is g for f, g in zip(fns, [add, sub, neg])])
    print(call_bad(fns[2], 1, 2), call_bad(fns[2], 5))
    try:
        pick(3, 1, 2)
    except IndexError as e:
        print("IndexError", e)
    try:
        pick(0, 1, 2)
    except TypeError as e:
        print("TypeError", e)


main()
