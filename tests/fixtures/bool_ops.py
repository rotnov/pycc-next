# #1211 (Part 3 of #1018): `and`/`or` as value-returning, short-circuiting
# expressions (docs/TYPE_SYSTEM.md, "`and` and `or`"). A value-context
# node yields the selected operand; a truth-context node (an if/elif/while
# test, a comprehension filter, a `not` operand) only tests the result.
# Optional-typed results are observed only after `or <int>` or inside an
# `is not None` branch: printing an Optional directly is not supported yet.
# A caught ZeroDivisionError's message is not printed: pycc's `//` text
# still differs from CPython 3.14's.


class Box:
    def __init__(self, n: int) -> None:
        self.n = n


def say_int(label: str, n: int) -> int:
    print("eval", label)
    return n


def say_str(label: str, s: str) -> str:
    print("eval", label)
    return s


def maybe(n: int) -> int | None:
    r: int | None = None
    if n >= 0:
        r = n
    return r


def maybe_float(x: float) -> float | None:
    r: float | None = None
    if x > -100.0:
        r = x
    return r


def maybe_bool(flag: bool, present: bool) -> bool | None:
    r: bool | None = None
    if present:
        r = flag
    return r


def divide(a: int, b: int) -> int:
    return a // b


def _pick(a, b):
    return a or b


def short_circuit() -> None:
    print(say_int("a", 0) or say_int("b", 5))
    print(say_int("c", 3) or say_int("d", 5))
    print(say_int("e", 0) and say_int("f", 5))
    print(say_int("g", 3) and say_int("h", 5))
    print(say_int("i", 0) or say_int("j", 0) or say_int("k", 9))
    print(say_int("l", 1) and say_int("m", 2) and say_int("n", 0))
    print(say_str("o", "") or say_str("p", "fallback"))


def joins(n: int, z: int, s: str, e: str, x: float, t: bool, f: bool) -> None:
    print(n or z, z or n, n and z, z and n)
    print(s or e, e or s, s and e, e and s)
    print(x or 2.5, 0.0 or x, x and 0.0)
    print(t or f, f or t, t and f, f and t)
    print(True or 0, 0 or True, False or 7, 7 and False)
    print(t and n, f or z)
    b = Box(1)
    c = Box(2)
    print((b or c).n, (b and c).n)
    print(_pick(0, 4), _pick(6, 4))
    print(1.5 or 2.5, 0.0 or 2.5)
    m = -0.0
    print(m or 1.0, m and 1.0)


def truthiness(big: int) -> None:
    inf = 1e308 * 10.0
    nan = inf - inf
    zero_big = big - big
    values = [0, 1, -3]
    for value in values:
        print("int", value or 100, value and 100)
    print("nan", nan or 1.0)
    print("-0.0", -0.0 or 1.0)
    print("empty", "" or "x", " " or "x")
    print("big", big or 1, zero_big or 1, big and 1, zero_big and 1)
    none_opt = maybe(-1)
    zero_opt = maybe(0)
    five_opt = maybe(5)
    print("opt", none_opt or 11, zero_opt or 12, five_opt or 13)
    print("optf", maybe_float(-200.0) or 1.5, maybe_float(-0.0) or 2.5, maybe_float(4.5) or 3.5)
    print("optb", maybe_bool(True, True) or False, maybe_bool(False, True) or True, maybe_bool(True, False) or False)
    r = zero_opt and 1
    if r is not None:
        print("and kept", r)
    q = none_opt and 1
    if q is None:
        print("and kept None")
    p = 7 and five_opt
    if p is not None:
        print("int and opt", p)
    box = Box(0)
    if box and 1:
        print("an instance is truthy")


def conditions(n: int, s: str, x: float, o: int | None) -> None:
    if n and s:
        print("n and s")
    elif x or o:
        print("x or o")
    else:
        print("neither")
    count = 3
    while count and s:
        count = count - 1
    print("count", count)
    print(not (n or s), not (n and s))
    if (w := n or 4) > 3:
        print("walrus", w)
    if (n or 7) == 7:
        print("compare", n or 7)
    if (k := n) and s:
        print("first-operand walrus", k)


def arguments(n: int, z: int) -> None:
    print(divide(n or 1, z or 2))
    print(say_int("arg", n and 10) or z)


def raising(n: int, z: int) -> None:
    try:
        print(z or divide(n, z))
    except ZeroDivisionError:
        print("ZeroDivisionError")
    try:
        print(n and n // z)
    except ZeroDivisionError:
        print("ZeroDivisionError")
    print(n or n // z)


short_circuit()
joins(3, 0, "s", "", 1.5, True, False)
joins(0, 4, "", "e", 0.0, False, True)
truthiness(4611686018427387904)
conditions(0, "s", 0.0, None)
conditions(5, "", 2.0, 0)
conditions(5, "s", 0.0, 3)
arguments(6, 0)
arguments(0, 3)
raising(8, 0)
limit = 5
evens = {i for i in range(8) if i and i % 2 == 0}
print(len(evens))
small = [i for i in range(8) if i < 2 or i > limit]
print(len(small), small[0], small[3])
