"""Raises an exception class its own module defines."""


class Failure(Exception):
    pass


def check(n: int) -> int:
    if n > 2:
        raise Failure("too big: " + str(n))
    return n
