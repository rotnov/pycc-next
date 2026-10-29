"""Compiled classes whose instances Python code pickles (v0.10)."""


class Point:
    def __init__(self, x: int, y: float, label: str, on: bool) -> None:
        self.x = x
        self.y = y
        self.label = label
        self.on = on

    def show(self) -> str:
        return self.label + ":" + str(self.x) + "," + str(self.y) + "," + str(self.on)


class Path:
    def __init__(self, start: Point, step: int) -> None:
        self.start = start
        self.steps = [step, step + 1]
        self.tags = {"a": step}

    def total(self) -> int:
        t = self.start.x
        for s in self.steps:
            t += s
        return t + len(self.tags)


class Ring:
    def __init__(self, start: Point, step: int) -> None:
        self.start = start
        self.step = step

    def total(self) -> int:
        return self.start.x + self.step


class Lazy:
    def __init__(self, n: int) -> None:
        self.n = n
        if n > 0:
            self.half = n // 2

    def get(self) -> int:
        return self.half


def make(n: int) -> Path:
    return Path(Point(n, n / 2, "p" + str(n), n % 2 == 0), n)


def total(p: Path) -> int:
    return p.total()


def total_ring(r: Ring) -> int:
    return r.total()
