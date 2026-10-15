"""An object compared with a string literal, and tested against a
frozenset of `str`, in a condition (non-strict mode): compiled code answers
an exact `str` from its UTF-8 bytes and everything else through CPython.
Every case prints what CPython prints."""

import collections
from typing import Any

LETTERS = frozenset("ab")
WORDS = frozenset(["ab", "", "été", "a\x00b", "\U0001f600"])
NOTHING: frozenset[str] = frozenset()


def _equalities(o: Any) -> list[str]:
    seen = []
    if o == "a":
        seen.append("=a")
    if o != "a":
        seen.append("!a")
    if "a" == o:
        seen.append("a=")
    if "a" != o:
        seen.append("a!")
    if o == "":
        seen.append("=empty")
    if o == "été":
        seen.append("=ete")
    if o == "a\x00b":
        seen.append("=nul")
    if o != "\U0001f600":
        seen.append("!emoji")
    return seen


def _memberships(o: Any) -> list[str]:
    seen = []
    if o in LETTERS:
        seen.append("in letters")
    if o not in LETTERS:
        seen.append("not in letters")
    if o in WORDS:
        seen.append("in words")
    if o in NOTHING:
        seen.append("in nothing")
    return seen


def _scan(src: str, pos: int) -> int:
    while True:
        try:
            char: Any = src[pos]
        except IndexError:
            char = None
        if char == "a" or char in LETTERS:
            pos += 1
        else:
            return pos


def main():
    values = [
        "a", "b", "", "ab", "été", "a\x00b", "\U0001f600", chr(0xD800),
        None, 1, 1.0, collections.UserString("a"), collections.UserString("ab"),
        ("a",),
    ]
    for o in values:
        print(ascii(o), _equalities(o), _memberships(o))
    print("scan", _scan("abbac", 0), _scan("", 0), _scan("aaa", 1))


main()
