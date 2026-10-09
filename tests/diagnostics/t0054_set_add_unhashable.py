class B:
    def __init__(self, x: int) -> None:
        self.x = x

    def __eq__(self, other: B) -> bool:
        return self.x == other.x


class P(B):
    pass


def f(s: set[P]) -> None:
    s.add(P(1))
