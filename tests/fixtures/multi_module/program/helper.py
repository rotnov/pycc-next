"""A helper module; it defines `main` too, in its own namespace."""

print("helper: body")
import shared

COUNT: int = 0
LIMITS: list[int] = [1, 2, 3]
NAME = __name__


class HelperError(Exception):
    pass


def double(x: int) -> int:
    return x * 2


def bump() -> None:
    global COUNT
    COUNT += 1


def greet(who: str) -> str:
    return "hello " + who + shared.suffix()


def fail(n: int) -> None:
    if n < 0:
        raise ValueError("negative")
    raise HelperError("bad " + str(n))


def shared_calls() -> int:
    return shared.calls()


def main() -> int:
    return 7
