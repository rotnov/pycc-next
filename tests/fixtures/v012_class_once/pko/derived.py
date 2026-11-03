from .base import Child, Plain


class Far(Plain):
    def value(self) -> int:
        return -self.n

    def tag(self) -> str:
        self.tagged = True
        return "far"


class Farther(Child):
    def value(self) -> int:
        return 100 + self.n


def both(p: Plain) -> int:
    return p.value() + p.n
