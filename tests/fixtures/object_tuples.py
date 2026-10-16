# Native tuples whose elements may be CPython objects (`tuple[int, Any]`),
# the shape typed parsers return: a position and a value of any type.
from typing import Any


def parse_num(s: str, pos: int) -> tuple[int, str]:
    end = pos
    while end < len(s) and s[end].isdigit():
        end += 1
    return end, s[pos:end]


def parse_value(s: str, pos: int) -> tuple[int, Any]:
    if s[pos] == "t":
        return pos + 4, True
    if s[pos] == "[":
        return pos + 2, []
    if s[pos] == "x":
        return pos + 1, None
    if s[pos] == "f":
        return pos + 3, 1.5
    if s[pos] == "k":
        key: tuple[str, ...] = (s, s[:1])
        return pos + 1, key
    return parse_num(s, pos)


def pair(s: str) -> tuple[int, Any, tuple[str, ...]]:
    pos, value = parse_value(s, 0)
    return pos, value, (s,)


def nested(s: str) -> tuple[int, Any]:
    pos, value = parse_value(s, 0)
    return pos + 100, [value, value]


def items(texts: list[str]) -> list[Any]:
    out: list[Any] = []
    for text in texts:
        pos, value = parse_value(text, 0)
        out.append(value)
    return out


for text in ["true", "[]", "x", "123ab", "f", "kq"]:
    pos, value = parse_value(text, 0)
    print(pos, repr(value), type(value).__name__)
t = parse_value("42", 0)
print(t[0], t[1], len(t))
a, b, c = pair("7")
print(a, b, c, type(b).__name__)
n, v = nested("[")
v[0].append(n)
print(n, v, v[0] is v[1])
print(items(["true", "9", "x"]))
try:
    parse_value("", 0)
except IndexError as e:
    print("IndexError:", e)
