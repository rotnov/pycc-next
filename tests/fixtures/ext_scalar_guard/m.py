"""Non-strict `--ext` functions with scalar parameters, published through
a signature wrapper: a Python caller passing an argument of another type
gets what CPython gives, the function's own error handling included."""

from __future__ import annotations

from typing import Any


def clean(s: str, /, *, sep: str = "\n") -> str:
    try:
        src = s.replace("\r\n", sep)
    except (AttributeError, TypeError):
        raise TypeError(f"Expected str object, not '{type(s).__qualname__}'") from None
    return src


def scale(x: float, n: int = 2, *, label: str = "x") -> str:
    return f"{label}={x * n}"


def count(*words: str, flag: bool = False) -> int:
    return len(words) + (10 if flag else 0)


def tag(name: str, **attrs: int) -> str:
    return name + "".join(f" {k}={v + 1}" for k, v in sorted(attrs.items()))


class Box:
    def __init__(self, n: int) -> None:
        self.n = n


def kind(value: Any) -> str:
    # `isinstance` of a CPython object against a tuple of classes.
    if isinstance(value, (dict, list)):
        return "container"
    if isinstance(value, (bool, (int, float))):
        return "number"
    if isinstance(value, (Box, str)):
        return "box or str"
    return "other"


def pick(items: Any, i: int = 0) -> object:
    return items[i]
