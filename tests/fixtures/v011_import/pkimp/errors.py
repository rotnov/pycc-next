"""Compiled functions whose frames a traceback and `sys._getframe` see."""
import sys


def inner(n: int) -> int:
    if n < 0:
        raise ValueError("negative: " + str(n))
    return n * 2


def outer(n: int) -> int:
    return inner(n) + 1


def caller_name() -> str:
    return sys._getframe(0).f_code.co_name + "/" + sys._getframe(1).f_code.co_name
