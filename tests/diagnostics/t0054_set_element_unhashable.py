class P:
    def __init__(self, x: int) -> None:
        self.x = x

    def __eq__(self, other: P) -> bool:
        return self.x == other.x


def f() -> int:
    s = {P(1), P(2)}
    return len(s)
