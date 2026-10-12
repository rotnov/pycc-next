SEP = "-"


def add(a: int, b: int = 10) -> int:
    return a + b


def scale(x: float, *, factor: float = 2.0) -> float:
    return x * factor


def total(*nums: int) -> int:
    return sum(nums)


def tags(name: str, sep: str = SEP, **extra: str) -> str:
    out = name
    for k in extra:
        out += sep + k + "=" + extra[k]
    return out


def mix(a: int, /, b: int, *rest: int, c: int, d: int = 4, **kw: int) -> str:
    return f"{a} {b} {rest} {c} {d} {sorted(kw.items())}"


def pos(a: int, /, b: int = 1) -> int:
    return a - b


def shout(*words: str) -> None:
    print(" ".join(words).upper())


class Box:
    def __init__(self, w: int = 1, h: int = 2) -> None:
        self.w = w
        self.h = h

    def area(self, *, scale: int = 1) -> int:
        return self.w * self.h * scale

    def grow(self, *amounts: int) -> int:
        return self.w + sum(amounts)


def use_inside() -> str:
    return mix(1, 2, 3, c=4, z=5) + " " + str(add(1)) + " " + tags("t", k="v")
