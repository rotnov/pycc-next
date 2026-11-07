"""Operations on objects in generic (unannotated) bodies, compiled with
their inline fast paths (`crates/pycc_codegen/src/fast_ops.rs`): each one
meets the operands the fast path answers -- compact `int`s, compact ASCII
`str`s, exact `set`s and `frozenset`s, exact built-in instances, module
globals -- and the ones it must hand to CPython: big `int`s, `bool`s,
subclasses, non-ASCII text, out-of-range indexes, unhashable items, and
globals rebound between reads.
"""

import sys

LIMIT = 3
CHARS = frozenset("abc")
SEEN = {"x", "y"}


class MyInt(int):
    def __add__(self, other):
        return ("MyInt.add", int(self), other)

    def __lt__(self, other):
        return True


class MyStr(str):
    def __eq__(self, other):
        return "eq-called"

    __hash__ = str.__hash__


def add(a, b):
    return a + b


def sub(a, b):
    return a - b


def bump(n):
    n += 1
    n -= 3
    return n


def less(a, b):
    if a < b:
        return "lt"
    if a >= b:
        return "ge"
    return "neither"


def equal(a, b):
    if a == b:
        return "eq"
    if a != b:
        return "ne"
    return "neither"


def index(s, i):
    try:
        return s[i]
    except IndexError:
        return "IndexError"
    except TypeError:
        return "TypeError"


def first_chars(s):
    out = []
    i = 0
    while i < len(s):
        out.append(s[i])
        i += 1
    return out


def is_quote(c):
    if c == '"':
        return "quote"
    if c != "ab":
        return "not ab"
    return "ab"


def member(c, s):
    try:
        if c in s:
            return "in"
        return "out"
    except TypeError:
        return "TypeError"


def kinds(v):
    if isinstance(v, dict):
        return "dict"
    if isinstance(v, list):
        return "list"
    if isinstance(v, int):
        return "int"
    if isinstance(v, str):
        return "str"
    return "other"


def read_limit():
    return LIMIT + 1


def read_print():
    return len


def count_in(text):
    n = 0
    for c in text:
        if c in CHARS:
            n += 1
    return n


def main():
    big = 2**40
    for a, b in [(1, 2), (-5, 3), (2**30 - 1, 1), (-(2**30), -1), (big, 1), (1, big),
                 (True, 1), (1, False), (1.5, 2), ("a", "b"), (MyInt(3), 4), (sys.maxsize, 1)]:
        print(add(a, b), sub(a, b) if not isinstance(a, str) else None)
    print(bump(5), bump(2**62), bump(True), bump(-2**30))
    for a, b in [(1, 2), (2, 1), (3, 3), (-1, 0), (big, 1), (1, big), (True, 2), (MyInt(9), 1),
                 (1.5, 1), ("a", "b"), ("b", "a")]:
        print(less(a, b))
    nan = float("nan")
    s1 = "hello"
    s2 = "".join(["hel", "lo"])
    for a, b in [(1, 1), (1, 2), (big, big), (s1, s1), (s1, s2), ("ab", "abc"), ("é", "é"),
                 ("é", "e"), (MyStr("x"), "x"), ("x", MyStr("x")), (nan, nan), (None, "x"),
                 (1, True), ("", "")]:
        print(equal(a, b))
    text = "ab\"cd"
    for i in [0, 1, 4, 5, -1, -5, -6, True, big, 2**62]:
        print(index(text, i))
    print(index("héllo", 1), index("héllo", -1), index(MyStr("xyz"), 1), index([1, 2], 1),
          index((3, 4), -1), index({0: "zero"}, 0))
    print(first_chars("tomli"), first_chars(""), first_chars("ñu"))
    for c in ['"', "ab", "a", "", "é", MyStr('"'), 1, None]:
        print(is_quote(c))
    for c, s in [("a", CHARS), ("z", CHARS), ("x", SEEN), (1, SEEN), (MyStr("a"), CHARS),
                 ([1], SEEN), ({1}, {frozenset({1})}), ("a", "cab"), ("a", ["a"]), ("a", {"a": 1})]:
        print(member(c, s))
    for v in [{}, [], 1, True, "s", MyInt(1), MyStr("s"), (), 1.0]:
        print(kinds(v))
    global LIMIT
    print(read_limit())
    LIMIT = 10
    print(read_limit())
    del LIMIT
    try:
        read_limit()
    except NameError as e:
        print("NameError", e)
    LIMIT = 3
    print(read_limit(), read_print() is len)
    print(count_in("abcabcxyz" * 30))


if __name__ == "__main__":
    main()
