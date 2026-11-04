from __future__ import annotations

import typing
from typing import Literal, NamedTuple, NotRequired, Required, TypedDict


class Config(TypedDict, total=False):
    """Options."""

    title: str | None
    items: list[int]
    mode: Literal["a", "b"]
    nested: dict[str, Config]
    req: Required[int]
    star: tuple[*tuple[int, ...]]


class Point(NamedTuple):
    x: int
    y: float = 0.0
    label: str | None = None


class Twin1(TypedDict):
    a: int
    b: str | None


class Twin2(TypedDict):
    a: int
    b: str | None


class Child(Config):
    extra: NotRequired[bytes]
