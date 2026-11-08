"""Unpacked calls between compiled bodies in generic (unannotated) code
(`crates/pycc_mir/src/multi_return_object.rs`): `a, b = f(...)` of a body
whose `return a, b` builds no tuple, and every result that is not such a
tuple -- a list, a string, a tuple of another length, a non-iterable, an
iterator, a rebound or re-coded callee, a forward -- unpacked at the call
site as CPython unpacks it.
"""

import sys
import traceback


def pair(s, pos):
    if pos >= len(s):
        return pos, None
    return pos + 1, s[pos]


def triple(s, pos, *, flag):
    p, c = pair(s, pos)
    return p, c, flag


def fwd(s, pos):
    return pair(s, pos)


def fwd_kw(s, pos):
    return triple(s, pos, flag=pos % 2 == 0)


def fwd_fwd(s, pos):
    return fwd(s, pos)


class Two:
    def __iter__(self):
        frame = sys._getframe(1)
        yield frame.f_code.co_name
        yield "two"


def gen2():
    yield "g1"
    yield "g2"


def whole(x):
    if x == 0:
        return [10, 20]
    if x == 1:
        return "xy"
    if x == 2:
        t = (1, 2, 3)
        return t
    if x == 3:
        return 5
    if x == 4:
        return gen2()
    if x == 5:
        return Two()
    if x == 6:
        return
    if x == 7:
        return (x,)
    return x, -x


def fwd_whole(x):
    return whole(x)


def boom(n):
    return n, 1 // n


def deep_boom(n):
    a, b = boom(n)
    return a, b


def depth(n):
    if n == 0:
        return 0, "end"
    a, b = depth(n - 1)
    return a + 1, b


def me(x):
    return sys._getframe(0).f_code.co_name, x


def where():
    frame = sys._getframe(1)
    return frame.f_code.co_name, frame.f_lineno


def who(x):
    name, line = where()
    return name, x


def keep(x):
    return x, x


def other(s, pos):
    return pos + 10, "z"


def coded(s, pos):
    return pos, "coded"


def scan(s):
    out = []
    pos = 0
    while pos < len(s):
        pos, c = pair(s, pos)
        out.append(c)
    return out


def scan_kw(s):
    out = []
    pos = 0
    while pos < len(s):
        pos, c, flag = fwd_kw(s, pos)
        out.append((c, flag))
    return out


def unpack_one(x):
    a, b = whole(x)
    return a, b


def unpack_fwd(x):
    a, b = fwd_whole(x)
    return "fwd", a, b


def call_deep(n):
    a, b = deep_boom(n)
    return a, b


def call_whole_int():
    a, b = whole(3)
    return a, b


def call_depth(n):
    a, b = depth(n)
    return a, b


def frames():
    a, b = me(1)
    c, d = who(2)
    return a, b, c, d


def keep_loop(x):
    a = b = None
    for _ in range(1000):
        a, b = keep(x)
    return a is x and b is x


# The drivers below catch what the compiled calls above raise.


def unpack_whole():
    out = []
    for x in range(9):
        for call in (unpack_one, unpack_fwd):
            try:
                out.append(call(x))
            except (TypeError, ValueError) as exc:
                out.append((call.__name__, type(exc).__name__, str(exc)))
    return out


def tracebacks():
    out = []
    for call, arg in ((call_deep, 2), (call_deep, 0), (lambda _: call_whole_int(), 0)):
        try:
            out.append(call(arg))
        except (ZeroDivisionError, TypeError) as exc:
            frames = [(f.name, f.lineno) for f in traceback.extract_tb(exc.__traceback__)]
            out.append((type(exc).__name__, str(exc), frames))
    return out


def recursion():
    a, b = call_depth(50)
    try:
        call_depth(10**6)
    except RecursionError:
        return a, b, "RecursionError"


def refcounts():
    x = object()
    before = sys.getrefcount(x)
    same = keep_loop(x)
    return same, sys.getrefcount(x) - before


def call_pair():
    a, b = pair("ab", 0)
    c, d = fwd("ab", 1)
    e, f = fwd_fwd("ab", 2)
    return a, b, c, d, e, f


def main():
    print(scan("hello"))
    print(scan_kw("abcd"))
    print(fwd_fwd("q", 0), fwd_kw("q", 1), triple("q", 0, flag=1))
    for row in unpack_whole():
        print(row)
    for row in tracebacks():
        print(row)
    print(recursion())
    print(frames())
    print(refcounts())
    print(call_pair())
    global pair
    saved = pair
    pair = other
    print(call_pair(), scan_kw("ab"))
    pair = saved
    print(call_pair())
    pair.__code__ = coded.__code__
    print(call_pair())


if __name__ == "__main__":
    main()
