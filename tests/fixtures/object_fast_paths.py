"""Object operations with a fast path in compiled code (non-strict mode):
`o[i]` and `o[a:b]` with native `int` keys, tuple displays, method calls
with inert arguments, comparisons in a condition, and `s[i] in chars`.
Every case prints what CPython prints, including the exceptions."""

import collections
import decimal
from typing import Any


def item(label: str, o: Any, i: int):
    try:
        print(label, o[i])
    except IndexError as e:
        print(label, "IndexError", e)
    except KeyError as e:
        print(label, "KeyError", e)
    except TypeError as e:
        print(label, "TypeError", e)


def item_true(label: str, o: Any):
    try:
        print(label, o[True])
    except IndexError as e:
        print(label, "IndexError", e)


def item_big(label: str, o: Any):
    try:
        print(label, o[10**30])
    except IndexError as e:
        print(label, "IndexError", e)


def subscripts(t: Any, items: Any, d: Any, s: Any, n: int):
    item("t[0]", t, 0)
    item("t[-1]", t, -1)
    item("t[n]", t, n)
    item("t[-n-1]", t, -n - 1)
    item_true("t[True]", t)
    item_big("t[big]", t)
    item("list[1]", items, 1)
    item("list[-4]", items, -4)
    item("dict[2]", d, 2)
    item("dict[3]", d, 3)
    item("str[1]", s, 1)
    item("range[5]", range(10), 5)
    item("range[-11]", range(10), -11)
    item("None[0]", None, 0)
    for i in range(-4, 4):
        try:
            print("loop", i, t[i])
        except IndexError as e:
            print("loop", i, "IndexError", e)
    a, b, c = t
    print("unpack", a, b, c)
    try:
        a, b = t
    except ValueError as e:
        print("unpack ValueError", e)


def slices(t: Any, items: Any, s: Any, n: int, none: Any):
    print("t[1:]", t[1:])
    print("t[:-1]", t[:-1])
    print("t[-10:10]", t[-10:10])
    print("t[2:1]", t[2:1])
    print("t[0:n] is t", t[0:n] is t)
    print("t[True:]", t[True:])
    print("t[:big]", t[: 10**30])
    print("t[-big:]", t[-(10**30) :])
    print("list[1:2]", items[1:2])
    print("list[:n]", items[:n])
    print("list[n:] fresh", items[0:n] is items)
    print("str[1:3]", s[1:3])
    print("range[1:3]", range(10)[1:3])
    try:
        print(none[1:])
    except TypeError as e:
        print("None[1:] TypeError", e)


def displays(a: Any, b: Any):
    pair = (a, b)
    print("pair", pair, type(pair).__name__, len(pair))
    x, y = pair
    print("unpacked", x, y)
    nested = ((a,), (b, a))
    print("nested", nested)


def counted() -> int:
    print("argument evaluated")
    return 1


def calls(text: Any, value: Any, d: Any):
    print("upper", text.upper())
    print("split", text.split(","))
    print("ljust", text.ljust(9, "."))
    print("count", text.count(value))
    print("get", d.get(2), d.get(3, "default"))
    try:
        print(d.missing(1))
    except AttributeError as e:
        print("missing AttributeError", e)
    try:
        print(d.missing(counted()))
    except AttributeError as e:
        print("missing with a call AttributeError", e)


def check(left: Any, right: Any):
    if left == right:
        print("eq", ascii(left), ascii(right))
    else:
        print("not eq", ascii(left), ascii(right))
    if left != right:
        print("ne", ascii(left))
    if left < right:
        print("lt", ascii(left))
    if left >= right:
        print("ge", ascii(left))


def compares(user: Any, other: Any):
    check("a", "a")
    check("a", "b")
    check("b", "a")
    check("ab", "a")
    check("é", "e")
    check(1, 1.0)
    check(float("nan"), float("nan"))
    if user == "x":
        print("UserString == x")
    if None == other:
        print("unreachable")
    else:
        print("None != other")
    if None != other:
        print("None != other again")
    try:
        if decimal.Decimal(1) < other:
            print("unreachable")
    except TypeError as e:
        print("Decimal < str TypeError", e)
    n = 0
    while other != "z":
        other = chr(ord(other) + 1)
        n += 1
    print("while", n, other)


def _skip(src: str, pos: int, chars: frozenset[str]) -> int:
    try:
        while src[pos] in chars:
            pos += 1
    except IndexError:
        pass
    return pos


def _not_in(src: str, pos: int, chars: set[str]) -> int:
    while src[pos] not in chars:
        pos += 1
    return pos


def membership():
    ws = frozenset(" \t")
    print("skip", _skip("  \tx", 0, ws), _skip("   ", 0, ws), _skip("ab", 5, ws))
    print("skip -", _skip("a  ", -2, ws))
    print("unicode", _skip("ééx", 0, frozenset("é")))
    print("not in", _not_in("abc;d", 0, {";", ","}))
    try:
        print(_not_in("abc", 0, {";"}))
    except IndexError as e:
        print("not in IndexError", e)


def kinds(values: Any, classes: Any, empty: Any):
    for v in values:
        print(
            type(v).__name__,
            isinstance(v, list),
            isinstance(v, dict),
            isinstance(v, (dict, list)),
            isinstance(v, int),
            isinstance(v, (str, float)),
            isinstance(v, classes),
            isinstance(v, empty),
        )


def main():
    subscripts((1, "two", 3.0), [4, 5, 6], {2: "two"}, "xyz", 3)
    slices((1, "two", 3.0), [4, 5, 6], "wxyz", 3, None)
    displays("a", 2)
    calls("a,b,c", "b", {2: "two"})
    compares(collections.UserString("x"), "k")
    membership()
    kinds(
        [1, True, "s", [1], {}, (1,), None, 1.5, frozenset(), set(),
         collections.UserList([1]), collections.OrderedDict()],
        (type({}), type([])),
        (),
    )


main()
