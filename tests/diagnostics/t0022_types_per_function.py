def f() -> int:
    return "a"


def g(y: int) -> int:
    return y.nope


def h(x: int) -> int:
    return -"s"


def main() -> int:
    return f() + g(1) + h(2)
