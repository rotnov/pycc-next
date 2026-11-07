"""Operations on objects in generic (unannotated) bodies, compiled with
their inline fast paths (`crates/pycc_codegen/src/fast_ops.rs`): each one
meets the operands the fast path answers -- compact `int`s, compact ASCII
`str`s, exact `set`s and `frozenset`s, exact built-in instances, module
globals -- and the ones it must hand to CPython: big `int`s, `bool`s,
subclasses, non-ASCII text, out-of-range indexes, unhashable items, and
globals rebound between reads. A one-character `str` in a `frozenset` is a
bit test against the set's cached ASCII mask, which must answer as the set
does for members of other types, with an `__eq__` CPython calls, beyond the
mask's size limit, after its cache entries are evicted, and when releasing
an evicted set runs a weakref callback that refills the cache.
"""

import sys
import weakref

LIMIT = 3
CHARS = frozenset("abc")
SEEN = {"x", "y"}
A_HASH = hash("a")


class MyInt(int):
    def __add__(self, other):
        return ("MyInt.add", int(self), other)

    def __lt__(self, other):
        return True


class MyStr(str):
    def __eq__(self, other):
        return "eq-called"

    __hash__ = str.__hash__


class LikeA:
    """Hashes as `"a"` and equals everything: CPython's lookup calls its
    `__eq__` for an item of the same hash."""

    def __hash__(self):
        return A_HASH

    def __eq__(self, other):
        print("LikeA.__eq__", other)
        return True


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


def bits(a, b):
    return (a & b, a | b, a ^ b)


def undo_sub(scanline, result):
    ai = 0
    for i in range(1, len(result)):
        result[i] = (scanline[i] + result[ai]) & 0xFF
        ai += 1
    return result


def store(buf, i, v):
    try:
        buf[i] = v
        return buf
    except (IndexError, TypeError, ValueError):
        return "refused"


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


def masks():
    odd = [frozenset({"a", 1}), frozenset({LikeA(), "b"}), frozenset({MyStr("a"), "b"}),
           frozenset(map(chr, range(300))), frozenset(), frozenset({"é", "ab", "c"}),
           frozenset({"\x00", "\x7f", "?", "@"})]
    for c in ["a", "b", "c", "é", "ab", "", "\x00", "\x7f", "?", "@", "A", MyStr("a"), 7, None]:
        print([member(c, s) for s in odd], member(c, CHARS), member(c, SEEN))
    # More sets than the cache holds, each queried twice, evicting the
    # entries of the earlier ones.
    print(count_members([frozenset(chr(48 + k) for k in range(i % 70)) for i in range(400)]))
    print(evicted_callback())


def evicted_callback():
    """A miss whose eviction releases the last reference to a set with a
    weakref callback, the callback refilling the same cache bucket with a
    set that lacks the item: the miss must still answer for its own set."""
    # Which sets hold "q" must not follow the allocation stride.
    pool = [frozenset({"q" if (i * 7919) % 13 < 6 else "z", "n%d" % i}) for i in range(2000)]
    buckets = {}
    for s in pool:
        buckets.setdefault((id(s) >> 4) & 63, []).append(s)
    for group in buckets.values():
        with_q = [s for s in group if "q" in s]
        without = [s for s in group if "q" not in s]
        if len(with_q) >= 2 and len(without) >= 2:
            break
    else:
        return "no bucket"
    target, victim = with_q[0], with_q[1]
    other, refill = without[0], without[1]
    del pool, buckets, group, with_q, without, s
    seen = []
    ref = weakref.ref(victim, lambda _: seen.append(member("q", refill)))
    member("q", victim)
    member("q", other)
    del victim
    answer = member("q", target)
    return answer, seen, ref() is None


def count_members(sets):
    found = 0
    for _ in range(2):
        i = 0
        for s in sets:
            for c in "0Az~\x7f":
                if c in s:
                    found += i
            i += 1
    return found


def rebind_shared(n):
    # `i += 1` may update `i`'s `int` in place only while nothing else holds
    # it: a list, another local, a dict and a default all keep their value.
    i = n
    held = [i]
    i += 1
    other = i
    i += 1
    table = {"k": i}
    i -= 7
    return i, held, other, table


def rebind_default(x=100000):
    x += 1
    x += 1
    return x


def rebind_ranges(start, step, count):
    # Through CPython's small ints, across zero, and past one 30-bit digit.
    out = []
    v = start
    for _ in range(count):
        v += step
        out.append(v)
    return out


def rebind_bits(v):
    v |= 1 << 20
    v &= 0xFFFFF0
    v ^= 0x5555
    return v


def rebind_kinds(v):
    v += 1000
    return v


def same_object(n):
    # The probe's: with the fast paths on, the loop keeps one `int`.
    i = n + 1000
    first = id(i)
    for _ in range(n):
        i += 1
    return id(i) == first


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
    for a, b in [(12, 10), (-1, 255), (big, 3), (True, False), (MyInt(6), 3), (1, 2.0)]:
        try:
            print(bits(a, b))
        except TypeError:
            print("TypeError")
    print(undo_sub(b"\x01\x02\xff\x80", bytearray(b"\x05\x00\x00\x00")))
    print(undo_sub(bytearray(b"\x10" * 6), bytearray(6)))
    for i in [0, 3, 4, -1, -4, -5, True, big]:
        print(index(b"ab\xffc", i), index(bytearray(b"ab\xffc"), i))
    for buf, i, v in [(bytearray(3), 0, 7), (bytearray(3), -1, 255), (bytearray(3), 3, 1),
                      (bytearray(3), 1, 256), (bytearray(3), 1, -1), (bytearray(3), 1, True),
                      (bytearray(3), 1, b"x"[0]), (bytearray(), 0, 1), ([0, 0], 1, 9), ({}, 1, 2)]:
        print(store(buf, i, v))
    view = bytearray(b"xyz")
    m = memoryview(view)
    print(store(view, 1, 65), bytes(m))
    m.release()
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
    print(rebind_shared(1000), rebind_shared(250), rebind_shared(2**40))
    print(rebind_default(), rebind_default(), rebind_default(5))
    print(rebind_ranges(270, -7, 80))
    print(rebind_ranges(2**30 - 40, 9, 10), rebind_ranges(-(2**30) + 40, -9, 10))
    print(rebind_bits(300), rebind_bits(-300000), rebind_bits(True))
    print(rebind_kinds(True), rebind_kinds(MyInt(5)), rebind_kinds(2.5), rebind_kinds(2**62))
    masks()


if __name__ == "__main__":
    main()
