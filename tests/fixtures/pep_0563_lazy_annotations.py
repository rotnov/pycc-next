# PEP 563: `from __future__ import annotations` (superseded by PEP 649/749 on
# 3.14). pycc accepts the directive as a compile-time no-op (#919, D-229).
#
# What this fixture proves: the directive is accepted and the file runs
# byte-identically to CPython, alongside a string annotation naming the
# enclosing class (`-> "Node"`, Part 1 of #889). What it does not exercise --
# deliberately, because pycc rejects it today -- is PEP 563's distinguishing
# case, a forward reference to a name defined *later* in the module, quoted
# or not; that is a recorded `core` gap for the row's flip.
from __future__ import annotations


class Node:
    def __init__(self, value: int) -> None:
        self.value = value

    def update(self, other: Node) -> None:
        self.value = other.value

    def clone(self) -> "Node":
        return Node(self.value)


def total(a: Node, b: Node) -> int:
    return a.value + b.value


a = Node(1)
b = Node(2)
a.update(b)
print(a.value)
print(total(a, b))
print(a.clone().value)
