"""Character-class loops (tests/char_class_loops.rs): `while s[pos] in chars`
and `not in`, whose test `crates/pycc_codegen/src/char_class.rs` lowers
without a character object and, for a `str` and a set that are parameters
the loop only reads, prepares once before the loop. Each meets the operands
the fast path answers -- an exact compact ASCII `str`, an exact `frozenset`
of exact `str`s, a small position in range -- and the ones it hands to
CPython: non-ASCII text, a `str` subclass, a position past the end, below
zero or too big for an index, a `bool` position, a `set`, a `frozenset`
subclass, a set of more than 256 members or with a member that is not a
`str`, a set rebound or changed in the loop, and a mask evicted while the
loop runs.
"""

WS = frozenset(" \t")
KEY = frozenset("abcdefghijklmnopqrstuvwxyz_")
CTRL = frozenset(chr(i) for i in range(32)) | frozenset("\x7f")
WIDE = frozenset(chr(i) for i in range(300))
MIXED = frozenset(["a", "b", 1, (" ",)])
NONE = frozenset()
ACCENTS = frozenset("aé ")
CHURN = [frozenset(chr(97 + i % 26) + chr(65 + i % 26) + str(i)) for i in range(400)]


class Text(str):
    def __getitem__(self, key):
        return str.__getitem__(self, key).upper()


class Chars(frozenset):
    def __contains__(self, item):
        return item != "b"


# A member that is a `str` subclass: the set's mask is not usable.
SUBCLASSED = frozenset([Text(" "), "a"])


def skip(s: str, pos: int, chars) -> int:
    try:
        while s[pos] in chars:
            pos += 1
    except IndexError:
        pass
    return pos


def until(s: str, pos: int, error_on) -> int:
    while s[pos] not in error_on:
        pos += 1
    return pos


def churn(s: str, pos: int, chars) -> int:
    # Every step tests 400 other sets, evicting `chars`'s cached mask.
    seen = 0
    try:
        while s[pos] in chars:
            for other in CHURN:
                if s[pos] in other:
                    seen += 1
            pos += 1
    except IndexError:
        pass
    return pos * 1000 + seen


def rebound(s: str, pos: int, chars) -> int:
    # The set is rebound in the loop: tested again at each step.
    try:
        while s[pos] in chars:
            pos += 1
            chars = KEY if chars is WS else WS
    except IndexError:
        pass
    return pos


def grown(s: str, pos: int, chars) -> int:
    # A `set` changed by the loop: each test sees its members then.
    try:
        while s[pos] in chars:
            chars.add(s[pos + 1])
            pos += 1
    except IndexError:
        pass
    return pos


def local(s: str, pos: int) -> int:
    chars = frozenset("ab")
    try:
        while s[pos] in chars:
            pos += 1
    except IndexError:
        pass
    return pos


def spans(s, pos, chars):
    # A generator: the loop that yields tests each character as it comes;
    # the one that does not runs between two suspensions.
    try:
        while s[pos] in chars:
            yield pos
            pos += 1
    except IndexError:
        pass
    start = pos
    try:
        while s[pos] not in chars:
            pos += 1
    except IndexError:
        pass
    yield start, pos


def describe(error):
    return type(error).__name__ + " " + str(error)


def sets():
    # New `set`s each time: `grown` adds to its own.
    classes = (WS, KEY, CTRL, WIDE, MIXED, ACCENTS, NONE, SUBCLASSED, Chars("a "), set(" a"))
    return classes, (WS, CTRL, frozenset("#"), set("#")), {" "}


def scan(s: str, pos: int) -> list:
    out = []
    classes, stops, growing = sets()
    for chars in classes:
        try:
            out.append(skip(s, pos, chars))
        except IndexError:
            out.append("IndexError")
    for error_on in stops:
        try:
            out.append(until(s, pos, error_on))
        except IndexError:
            out.append("IndexError")
    out.append(churn(s, pos, KEY))
    out.append(rebound(s, pos, WS))
    out.append(grown(s, pos, growing))
    out.append(local(s, pos))
    return out


def run(s: str) -> list:
    out = []
    pos = 0
    while pos <= len(s) + 1:
        out.append((pos, scan(s, pos)))
        pos += 1
    return out


def main():
    texts = ["", "  ab_c d#", "ab\tc  é#", "é  a", "\x01\x02a", "aaaa", "b a b#"]
    for s in texts:
        for row in run(s):
            print(row)
    for s in texts:
        # A `str` subclass, positions below zero, past the end and too big
        # for an index, and a `bool` position.
        for pos in (-3, -1, 2**70, True):
            print(s, pos, scan(s, pos))
        print(s, scan(Text(s), 0))
        print(list(spans(s, 0, WS)), list(spans(s, 1, KEY)), list(spans(Text(s), 0, WS)))
        # The exceptions the loops let through, with CPython's messages.
        for pos in (len(s), 2**70, -len(s) - 1):
            for test, chars in ((skip, WS), (until, WS), (until, set("#"))):
                try:
                    print(test(s, pos, chars))
                except Exception as error:
                    print(describe(error))
