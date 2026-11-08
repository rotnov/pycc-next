"""Scanner-shaped generic bodies whose `int` and one-character `str` locals
are tagged words (`crates/pycc_codegen/src/tagged_locals.rs`) and whose
`str` slices, `startswith` tests and comparisons with `int` literals
take machine-integer operands
(`crates/pycc_codegen/src/str_words.rs`). Each meets the operands the word
paths answer -- small `int`s, exact `str`s, compact ASCII text -- and the
ones they must hand to CPython: big `int`s, `bool`s, non-`int` bounds,
`str` subclasses, non-ASCII text, lists, tuples and objects whose
`__getitem__` sees the `slice` CPython builds, values that leave the
function, and exceptions raised mid-scan.
"""

import sys

class Probe:
    def __getitem__(self, key):
        return ("getitem", type(key).__name__, key.start, key.stop, key.step)


class Loud(str):
    def startswith(self, *args):
        return ("Loud.startswith",) + args

    def __getitem__(self, key):
        return ("Loud.getitem", key)


def skip(src, pos, chars):
    try:
        while src[pos] in chars:
            pos += 1
    except IndexError:
        pass
    return pos


def word(src, pos):
    start = pos
    pos = skip(src, pos, frozenset("abcdefghijklmnopqrstuvwxyz"))
    return pos, src[start:pos]


def quoted(src, pos):
    pos += 1
    start = pos
    out = ""
    while True:
        try:
            char = src[pos]
        except IndexError:
            raise ValueError(f"unterminated at {pos}") from None
        if char == '"':
            if src.startswith('"""', pos):
                return pos + 3, out + src[start:pos] + "<3>"
            return pos + 1, out + src[start:pos]
        if char == "\\":
            out += src[start:pos]
            out += src[pos + 1 : pos + 2].upper()
            pos += 2
            start = pos
            continue
        pos += 1


def tokens(src):
    pos = 0
    out = []
    while pos < len(src):
        char = src[pos]
        if char == " ":
            pos += 1
        elif char == '"':
            pos, text = quoted(src, pos)
            out.append(("str", text))
        elif not src.startswith("#", pos):
            pos, text = word(src, pos)
            if not text:
                out.append(("other", src[pos : pos + 1]))
                pos += 1
            else:
                out.append(("word", text))
        else:
            out.append(("comment", src[pos + 1 :]))
            break
    return out


def slices(s, a, b):
    return [s[a:b], s[a:], s[:b], s[a : b + 1], s[a - b : b - a], s[-a:-1]]


def starts(s, t, a):
    out = []
    if s.startswith("ab"):
        out.append("ab")
    if s.startswith("ab", a):
        out.append("ab@a")
    if not s.startswith("b", a + 1):
        out.append("not b@a+1")
    if s.startswith("bc", a - 1):
        out.append("bc@a-1")
    while s.startswith("a", a):
        a += 1
    out.append(a)
    return out


def char_leaves(src):
    i = 0
    c = src[i]
    seen = [c]
    while c != "z":
        i += 1
        c = src[i]
    seen.append(c)
    seen.append(c is src[i])
    return i, seen


def counter(n):
    i = n
    while i < n + 5:
        i += 1
    j = i - 2**61
    j -= 2**61
    j -= 2**61
    return i, j, type(j).__name__


def compares(start, step):
    out = []
    i = start
    for _ in range(3):
        if i < 0:
            out.append("lt0")
        if 1 >= i:
            out.append("le1")
        if i == 2:
            out.append("eq2")
        if 3 != i:
            out.append("ne3")
        if i > -2 and 4 <= i:
            out.append("ge4")
        i += step
    return i, out


def pinned(text, pos):
    # `text[pos : pos + 2]` evaluates `pos + 2` after `text`, so `text`'s
    # read is pinned (`operand_pin`): it takes a reference the word path
    # must give back.
    before = sys.getrefcount(text)
    out = []
    for _ in range(4):
        out.append(text[pos : pos + 2])
        if text.startswith("ob", pos):
            out.append("ob")
        pos += 1
    out.append(text[2**70 :])
    out.append(text[pos + 0.5 :] if pos > 9 else "")
    out.append(sys.getrefcount(text) == before)
    return out


def leaked(n):
    pos = n
    pos += 1
    kept = [pos, pos]
    pos += 1
    return kept[0] is kept[1], pos, kept


def main():
    print(tokens('alpha "be\\ta" # tail'))
    print(tokens('x "a""""b" y'))
    print(tokens('k=1 "q\\"" z'))
    for args in [("abcdef", 1, 4), ("abcdef", 0, 0), ("abcdef", 4, 2), ("abcdef", 7, 9),
                 ("abcdef", -3, -1), ("", 0, 1), ("héllo wörld", 2, 8), ("abcdef", 2**62, 3),
                 ("abcdef", 2**70, 3), ("abcdef", True, 3), ([1, 2, 3, 4, 5], 1, 3),
                 ((1, 2, 3, 4, 5), 1, 3), (Loud("abcdef"), 1, 3)]:
        print(slices(*args))
    print(Probe()[1:3], Probe()[2:], Probe()[:5])
    for s, a in [("abcabc", 0), ("abcabc", 3), ("abcabc", -3), ("abcabc", -9), ("abcabc", 6),
                 ("abcabc", 99), ("aaab", 0), ("ábc", 0), ("abc", 2**63), ("abc", False)]:
        print(starts(s, "ab", a))
    print(Loud("abc").startswith("ab", 0))
    try:
        starts("abc", "ab", 1.5)
    except TypeError as error:
        print("TypeError", error)
    try:
        slices("abc", None, "x")
    except TypeError as error:
        print("TypeError", error)
    try:
        quoted('"abc', 0)
    except ValueError as error:
        print("ValueError", error)
    print(skip("aaab", 0, "a"), skip("aaa", 0, "a"), skip("aaa", 5, "a"), skip("aaa", 2**64, "a"))
    print(char_leaves("abcz"))
    print(counter(10), counter(2**62 - 3))
    print(leaked(300))
    print(pinned("".join(["glo", "bal"]), 0), pinned("".join(["glo", "bal"]), -3))
    for args in [(-1, 1), (2**62 - 2, 1), (0.5, 1), (True, 1), (5, -2), (2**63, -2**62)]:
        print(compares(*args))


main()
