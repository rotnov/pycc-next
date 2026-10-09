# #1212 (Part 4 of #1018): chained comparisons `a < b < c`
# (docs/TYPE_SYSTEM.md, "Chained comparisons"). Every operand is evaluated
# at most once, left to right, and the chain stops at the first false link.
# Every compared `int` stays below 2**53: comparing a heap bigint raises
# pycc's own OverflowError, and an int-versus-float link converts the int.
from dataclasses import dataclass


@dataclass
class Point:
    x: int
    y: int


def say(label: str, n: int) -> int:
    print("eval", label)
    return n


def say_float(label: str, x: float) -> float:
    print("eval", label)
    return x


def maybe(n: int) -> int | None:
    r: int | None = None
    if n >= 0:
        r = n
    return r


def in_range(lo: int, x: int, hi: int) -> bool:
    return lo <= x < hi


def check_len_three() -> None:
    print(say("a", 1) < say("b", 2) < say("c", 3))
    print(say("a", 1) < say("b", 0) < say("c", 3))
    print(say("a", 3) > say("b", 2) < say("c", 1))


def check_len_four() -> None:
    print(say("a", 1) < say("b", 2) > say("c", 0) < say("d", 9))
    print(say("a", 1) < say("b", 2) > say("c", 3) < say("d", 9))
    print(say("a", 1) == say("b", 1) != say("c", 2) <= say("d", 2))


def check_mixed_types(n: int, x: float, s: str) -> None:
    print(1 < 2.5 < 3)
    print(True < 2 < 3)
    print(1 == 1.0 != 2)
    print(0 <= n < x)
    print(False < True == 1)
    print("a" < s <= "z")
    print("b" < s < "c")
    print(say_float("f", 0.5) < say("g", 1) < 2.0)


def check_value_and_condition(n: int) -> None:
    b: bool = 0 < n < 3
    print(b)
    print(in_range(0, n, 10), in_range(0, 10, 10))
    if 0 < n < 3:
        print("in range")
    else:
        print("out of range")
    if not 5 < n < 10:
        print("not in (5, 10)")
    i = 0
    while 0 <= i < 3:
        i = i + 1
    print(i)
    evens = [k for k in range(10) if 2 <= k < 7 and k % 2 == 0]
    for k in evens:
        print("even", k)


def check_boolop_operands(n: int) -> None:
    print(0 < n < 3 or n == 7)
    print(0 < n < 3 and n > 1)
    print(n == 7 or 0 < n < 3)
    print((n or 5) < 6 < 7)
    print((0 or n) < 6 < 7)


def check_dataclass() -> None:
    p = Point(1, 2)
    q = Point(1, 2)
    r = Point(3, 4)
    print(p == q == Point(1, 2))
    print(p == q == r)
    print(p == q != r)
    print(p != q == r)
    print(p == r == q)


def check_is_none(o: int | None) -> None:
    print(o is None is None)
    print(o is not None is not None)
    print(None is o is None)


def check_walrus(a: int, b: int) -> None:
    n = 0
    if 0 < (n := a) < b:
        print("bound in range", n)
    else:
        print("bound out of range", n)


check_len_three()
check_len_four()
check_mixed_types(2, 2.5, "bee")
check_value_and_condition(2)
check_value_and_condition(9)
check_boolop_operands(2)
check_boolop_operands(0)
check_dataclass()
check_is_none(maybe(-1))
check_is_none(maybe(4))
check_walrus(3, 5)
check_walrus(7, 5)
