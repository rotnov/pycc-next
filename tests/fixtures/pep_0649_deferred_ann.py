# The CPython 3.13 oracle evaluates annotations eagerly; this future import gives it
# the deferred (PEP 649) semantics pycc follows.
from __future__ import annotations
class Node:
    def __init__(self, value: int) -> None:
        self.value = value

    def update(self, other: Node) -> None:
        self.value = other.value

    def same(self, other: Node) -> bool:
        return self.value == other.value


a = Node(1)
b = Node(2)
a.update(b)
print(a.value)
print(a.same(b))
