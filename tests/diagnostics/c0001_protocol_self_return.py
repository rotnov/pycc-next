from typing import Protocol

class P(Protocol):
    def clone(self) -> P: ...

class C:
    def __init__(self) -> None:
        self.x = 0

    def clone(self) -> C:
        return C()

def main() -> None:
    c: P = C()
    print(c.x)
