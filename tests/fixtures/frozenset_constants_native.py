# A module's frozenset constants stay native frozensets in --ext --non-strict
# mode, so its calls of an `Iterable[str]` function take the native
# specialization (tomli's `skip_chars(src, pos, TOML_WS)` pattern), even
# when another function of the module fails the first check and the
# non-strict passes widen what they may.
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
