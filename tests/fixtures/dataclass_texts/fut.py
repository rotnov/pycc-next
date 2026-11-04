from __future__ import annotations

import dataclasses
import typing as t
from dataclasses import dataclass, field


@dataclass(frozen=True)
class Deferred:
    a: int
    b: "list[int]" = field(default_factory=list)
    c: t.ClassVar[int] = 3
    d: dataclasses.InitVar[str] = "x"

    def __post_init__(self, d: str) -> None:
        object.__setattr__(self, "a", self.a + len(d))


@dataclass(slots=True)
class Later:
    ref: Deferred | None = None
