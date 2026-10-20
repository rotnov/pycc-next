"""Functions other modules of the package call natively."""

print("init util", __name__)
SCALE = 3


def scaled(n: int) -> int:
    return n * SCALE


def shout(s: str) -> str:
    return s.upper() + "!"


def ratio(a: float, b: float) -> float:
    return a / b


def is_short(s: str) -> bool:
    return len(s) < 4


def note(s: str) -> None:
    print("note", s)


def ident(x):
    return x


def fails(s: str) -> str:
    raise KeyError(s)
