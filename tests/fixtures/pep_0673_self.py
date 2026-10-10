# The CPython 3.13 oracle evaluates annotations eagerly; this future import gives it
# the deferred (PEP 649) semantics pycc follows.
from __future__ import annotations
class Box:
    def __init__(self, value: int) -> None:
        self.value = value

    def with_value(self, v: int) -> Self:
        self.value = v
        return self

    def clone(self) -> Self:
        return self


b = Box(1)
print(b.with_value(2).value)
print(b.clone().value)
