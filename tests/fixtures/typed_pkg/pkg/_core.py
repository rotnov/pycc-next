# A small `key = value` parser written the way typed packages are: aliases
# imported only for the type checker, `Final` constants, frozenset tables,
# keyword-only parameters with computed defaults, `str | None` locals and
# exceptions of a class from another module.
from __future__ import annotations

from ._errors import ParseError

TYPE_CHECKING = False
if TYPE_CHECKING:
    from collections.abc import Iterable
    from typing import Any, Final

    from ._types import Key, ParseFloat, Pos

MAX_PARTS: Final = 4
WS: Final = frozenset(" \t")
KEY_CHARS: Final = frozenset(
    "abcdefghijklmnopqrstuvwxyz" "ABCDEFGHIJKLMNOPQRSTUVWXYZ" "0123456789" "-_"
)
CTRL: Final = frozenset(chr(i) for i in range(32)) | frozenset(chr(127))
ILLEGAL_STR_CHARS: Final = CTRL - frozenset("\t")
NUMBER_CHARS: Final = frozenset("0123456789") | frozenset(".-+e")


class Table:
    def __init__(self) -> None:
        self.data: dict[str, Any] = {}
        self.count = 0

    def put(self, key: Key, value: Any) -> None:
        cont = self.data
        for k in key[:-1]:
            if k not in cont:
                cont[k] = {}
            cont = cont[k]
        cont[key[-1]] = value
        self.count += 1


def skip_chars(src: str, pos: Pos, chars: Iterable[str]) -> Pos:
    try:
        while src[pos] in chars:
            pos += 1
    except IndexError:
        pass
    return pos


def skip_until(
    src: str, pos: Pos, expect: str, *, error_on: frozenset[str], error_on_eof: bool
) -> Pos:
    try:
        new_pos = src.index(expect, pos)
    except ValueError:
        new_pos = len(src)
        if error_on_eof:
            raise ParseError(f"Expected {expect!r}", src, new_pos) from None
    if not error_on.isdisjoint(src[pos:new_pos]):
        while src[pos] not in error_on:
            pos += 1
        raise ParseError(f"Found invalid character {src[pos]!r}", src, pos)
    return new_pos


def parse_key(src: str, pos: Pos) -> tuple[Pos, Key]:
    start = pos
    pos = skip_chars(src, pos, KEY_CHARS)
    if pos == start:
        raise ParseError("Invalid key", src, pos)
    key: Key = (src[start:pos],)
    while src.startswith(".", pos):
        start = pos + 1
        pos = skip_chars(src, start, KEY_CHARS)
        key += (src[start:pos],)
        if len(key) > MAX_PARTS:
            raise ParseError("Too many key parts", src, pos)
    return pos, key


def parse_string(src: str, pos: Pos) -> tuple[Pos, str]:
    pos += 1
    end = skip_until(src, pos, '"', error_on=ILLEGAL_STR_CHARS, error_on_eof=True)
    return end + 1, src[pos:end]


def parse_value(src: str, pos: Pos, parse_float: ParseFloat) -> tuple[Pos, Any]:
    try:
        char: str | None = src[pos]
    except IndexError:
        char = None
    if char == '"':
        return parse_string(src, pos)
    if src.startswith("true", pos):
        return pos + 4, True
    if src.startswith("false", pos):
        return pos + 5, False
    end = skip_chars(src, pos, NUMBER_CHARS)
    text = src[pos:end]
    try:
        if "." in text or "e" in text:
            return end, parse_float(text)
        return end, int(text)
    except ValueError as e:
        raise ParseError("Invalid number", src, pos) from e


def loads(s: str, *, parse_float: ParseFloat = float) -> dict[str, Any]:
    src = s.replace("\r\n", "\n")
    out = Table()
    pos = 0
    while True:
        pos = skip_chars(src, pos, WS)
        try:
            char = src[pos]
        except IndexError:
            break
        if char == "\n":
            pos += 1
            continue
        if char in {"#", ";"}:
            pos = skip_until(src, pos, "\n", error_on=CTRL - WS, error_on_eof=False)
            continue
        pos, key = parse_key(src, pos)
        pos = skip_chars(src, pos, WS)
        if not src.startswith("=", pos):
            raise ParseError("Expected '='", src, pos)
        pos = skip_chars(src, pos + 1, WS)
        pos, value = parse_value(src, pos, parse_float)
        out.put(key, value)
    return out.data


def load_pairs(lines: list[str], *, sep: str = "=") -> list[tuple[str, str]]:
    pairs = []
    for line in lines:
        k, _, v = line.partition(sep)
        pairs.append((k.strip(), v.strip()))
    return pairs


def describe(value: str) -> str:
    return type(value).__qualname__
