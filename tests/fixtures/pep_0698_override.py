# PEP 698 -- `@override` (pycc fixture)
#
# `@override` is a pure marker: at runtime it returns the decorated
# function unchanged, so the program's observable output is decided
# entirely by ordinary method overriding. pycc additionally verifies
# at compile time that the decorated method name exists in a base class.
#
# The import is required by CPython (decorators evaluate eagerly);
# pycc recognizes the bare name `override` without it (#579).
from typing import override


class Base:
    def __init__(self) -> None:
        self.x = 0

    def f(self) -> int:
        return 1

class Derived(Base):
    def __init__(self) -> None:
        return

    @override
    def f(self) -> int:
        return 2

d = Derived()
print(d.f())
