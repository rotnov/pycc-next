# A small parser whose selected functions form one region
# (tests/select_regions.rs): `value` and `items` close a cycle, `escapes`
# dispatches through a local bound to `esc_a` or `esc_b` (B2), `word`
# takes its position from a match of the pattern global `WORD` (B3), and
# `word` and `esc_b` are not selected: they join as position-flow callees.
# `tails` reads `pos` from a lambda, so its `pos` stays an object; `size`
# calls a parameter named `len`.
import re

WORD = re.compile(r"[a-z]+")


def skip(s: str, pos: int, chars=" ") -> int:
    try:
        while s[pos] in chars:
            pos += 1
    except IndexError:
        pass
    return pos


def other_skip(s, pos, chars=" "):
    return pos + 100


def word(s: str, pos: int) -> tuple[int, str]:
    m = WORD.match(s, pos)
    if m is None:
        raise ValueError("no word at " + str(pos))
    return m.end(), m.group()


def number(s: str, pos: int) -> tuple[int, int]:
    start = pos
    while pos < len(s) and s[pos].isdigit():
        pos += 1
    return pos, int(s[start:pos])


def esc_a(s: str, pos: int) -> tuple[int, str]:
    return pos + 1, "A"


def esc_b(s: str, pos: int) -> tuple[int, str]:
    return pos + 2, "B"


def escapes(s: str, pos: int, loud: bool) -> tuple[int, str]:
    if loud:
        f = esc_a
    else:
        f = esc_b
    out = ""
    while pos < len(s) and s[pos] == "\\":
        pos, t = f(s, pos)
        out += t
    return pos, out


def value(s: str, pos: int, depth: int) -> tuple[int, object]:
    if s[pos] == "[":
        return items(s, pos + 1, depth + 1)
    if s[pos] == "\\":
        return escapes(s, pos, depth % 2 == 0)
    if s[pos].isdigit():
        return number(s, pos)
    return word(s, pos)


def items(s: str, pos: int, depth: int) -> tuple[int, object]:
    out = []
    while True:
        pos = skip(s, pos)
        if s[pos] == "]":
            return pos + 1, out
        pos, v = value(s, pos, depth)
        out.append(v)
        pos = skip(s, pos, " ,")


def parse(s: str) -> list:
    pos = 0
    out = []
    while pos < len(s):
        pos = skip(s, pos, " \n")
        if pos >= len(s):
            break
        pos, v = value(s, pos, 0)
        out.append(v)
    return out


def tails(s: str) -> list:
    n = skip(s, 0)
    pos, w = word(s, n)
    get = lambda: s[pos:]
    pos += 1
    return [n, w, get()]


# `len` is a parameter here, not the builtin: `size` proves nothing.
def size(s: str, len) -> int:
    return len(s)


def one(t):
    return 1


def sizes(s: str) -> int:
    return size(s, len) + size(s, one)
