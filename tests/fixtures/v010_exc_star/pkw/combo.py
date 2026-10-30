"""`Coded` bound by a wildcard import, then a base beside `OSError`."""
from .common import *  # noqa: F403


class Combined(Coded, OSError):  # noqa: F405
    pass


def combine(n: int) -> str:
    try:
        raise Combined(n)
    except OSError as e:
        return type(e).__name__ + " " + str(e.args)
