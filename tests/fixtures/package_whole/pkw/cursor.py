"""A compiled class whose instances other modules of the package read natively."""


class Cursor:
    def __init__(self, src: str) -> None:
        self.src = src
        self.pos = 0
        self.depth = 0
        self.marks: list[int] = []
        self.parent: "Cursor | None" = None

    def peek(self, n: int) -> str:
        return self.src[self.pos : self.pos + n]

    def mark(self) -> None:
        self.marks.append(self.pos)

    def child(self) -> "Cursor":
        c = Cursor(self.src)
        c.depth = self.depth + 1
        c.parent = self
        return c

    def at_end(self) -> bool:
        return self.pos >= len(self.src)


def make(src: str) -> Cursor:
    return Cursor(src)
