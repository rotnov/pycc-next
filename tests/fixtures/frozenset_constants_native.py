# A module's frozenset constants passed by compiled functions to an
# `Iterable[str]` function (tomli's `skip_chars(src, pos, TOML_WS)` pattern)
# and tested with `in`, next to a function that is not annotated.
from __future__ import annotations

TYPE_CHECKING = False
if TYPE_CHECKING:
    from collections.abc import Iterable

WS = frozenset(" \t")
WORD = frozenset("abcdefghijklmnopqrstuvwxyz")


def skip_chars(src: str, pos: int, chars: Iterable[str]) -> int:
    try:
        while src[pos] in chars:
            pos += 1
    except IndexError:
        pass
    return pos


def words(src: str) -> list[str]:
    out = []
    pos = skip_chars(src, 0, WS)
    while pos < len(src):
        end = skip_chars(src, pos, WORD)
        if end == pos:
            break
        out.append(src[pos:end])
        pos = skip_chars(src, end, WS)
    return out


def first(src, pos):
    return skip_chars(src, pos, WS)


def codes(text):
    total = 0
    for c in text:
        total += ord(c)
    return total
