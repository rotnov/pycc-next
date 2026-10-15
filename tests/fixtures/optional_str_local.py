"""`str | None` locals bound only to `str` values and `None` (non-strict
mode): compiled code keeps them native, with null for `None`, and boxes
them back wherever an object is needed. Every case prints what CPython
prints. The annotations are postponed (PEP 563), as in a package that
supports older Pythons, which is what makes a `str | None` local an
object in non-strict mode."""

from __future__ import annotations

from typing import Any

LETTERS = frozenset("abé")


def _show(value: Any) -> str:
    return "<" + ascii(value) + ">"


def _kind(src: str, pos: int, extra: frozenset[str]) -> str:
    try:
        char: str | None = src[pos]
    except IndexError:
        char = None
    seen = []
    if char == "a":
        seen.append("=a")
    if char != "b":
        seen.append("!b")
    if char in LETTERS:
        seen.append("in")
    if char not in extra:
        seen.append("not in extra")
    if char is None:
        seen.append("None")
    if char is not None:
        seen.append("str")
    if char == "a" or char in extra:
        seen.append("a or extra")
    if not char:
        seen.append("falsy")
    seen.append("yes" if char else "no")
    seen.append(_show(char))
    seen.append(f"{char}|" + ascii(char))
    copy = char
    seen.append(str(copy is char))
    seen.append(str(char == src))
    return " ".join(seen)


def _last_word(words: list[str]) -> Any:
    last: str | None = None
    for word in words:
        if word == "":
            last = None
        else:
            last = word
    return last


def _unbound(flag: bool) -> str:
    if flag:
        char: str | None = "set"
    try:
        return f"{char}"
    except NameError as e:
        return f"NameError {e}"


def _either(text: str) -> Any:
    found: str | None = None
    if text:
        found = text[0]
    return found or "default"


def main():
    for src, pos in [("abc", 0), ("abc", 1), ("abc", 2), ("abc", 3), ("été", 0), ("", 0)]:
        print(ascii(src), pos, ascii(_kind(src, pos, frozenset("c"))))
    print(_last_word(["x", "", "yz"]), _last_word(["x", ""]), _last_word([]))
    print(_unbound(True), _unbound(False))
    print(_either("hello"), _either(""))


main()
