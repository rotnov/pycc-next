from typing import Protocol

class P(Protocol):
    def foo(self) -> int: ...

class C:
    def __init__(self) -> None:
        self.x = 0

    def foo(self) -> int:
        return self.x

def make() -> P:
    return C()

def main() -> None:
    p: P = make()
    print(p.foo())
