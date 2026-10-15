"""Reads `app.settings` back while it is partially initialized."""

from app.settings import LIMIT

OK = LIMIT > 2
from app.settings import MODE  # noqa: E402

print("checks: unreachable")


def mode() -> str:
    return MODE
