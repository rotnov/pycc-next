"""A region of selected functions (tests/region_words.rs) whose positions are
native `int`s, read by the `str` word paths (`crates/pycc_codegen/src/str_words.rs`
and `tagged_locals.rs`): `s[a:b]`, `s.startswith(t, a)` and `c = s[pos]`
with its comparisons. Each meets the operands the word paths answer --
small positions, exact `str`s, compact ASCII text -- and the ones they hand
to CPython: a big `int` position, a `str` subclass, non-ASCII text, an
index past the end.
"""

KEY = frozenset("abcdefghijklmnopqrstuvwxyz_")


class Text(str):
    def __getitem__(self, key):
        return "<" + str.__getitem__(self, key) + ">"

    def startswith(self, prefix, *args):
        return not str.startswith(self, prefix, *args)


def skip(s: str, pos: int) -> int:
    try:
        while s[pos] == " ":
            pos += 1
    except IndexError:
        pass
    return pos


def kind(s: str, pos: int) -> str:
    try:
        char = s[pos]
    except IndexError:
        return "end"
    if char == "#":
        return "comment"
    if char != "x":
        if char in KEY:
            return "key"
        return "other " + char
    return "x"


def cuts(s: str, pos: int) -> list:
    t = Text(s)
    far = pos + 2**70
    back = pos - 3
    return [
        s[pos : pos + 2],
        s[pos:],
        s[:pos],
        s[back:pos],
        s[pos - 1 : pos + 1],
        s[-pos:],
        s[pos:far],
        s[far:],
        t[pos : pos + 2],
        s[pos + 100 :],
    ]


def starts(s: str, pos: int) -> list:
    t = Text(s)
    far = pos + 2**70
    out = []
    if s.startswith("ab", pos):
        out.append("ab")
    if not s.startswith("b", pos):
        out.append("not b")
    if s.startswith("ab", pos - 100):
        out.append("ab@-100")
    if s.startswith("a", pos + 100):
        out.append("a@+100")
    if s.startswith("a", far):
        out.append("a@far")
    if t.startswith("ab", pos):
        out.append("Text")
    n = 0
    while s.startswith("ab", pos + n):
        n += 2
    out.append(n)
    return out


def beyond(s: str, pos: int) -> str:
    far = pos + 2**70
    try:
        char = s[far]
    except IndexError as error:
        return "IndexError " + str(error)
    return char


def step(s: str, pos: int) -> int:
    return skip(s, pos + 1)


def run(s: str) -> list:
    out = []
    pos = 0
    while pos <= len(s) + 1:
        out.append((pos, kind(s, pos), cuts(s, pos), starts(s, pos), beyond(s, pos)))
        pos = step(s, pos)
    return out


def main():
    for s in ["", "ab ab#x", "  abab_x?", "aé#ab", "xx"]:
        for row in run(s):
            print(row)
