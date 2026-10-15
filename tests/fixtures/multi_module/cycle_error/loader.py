"""Reads `config` back while it is partially initialized.

Nothing here prints before the failing import: CPython's traceback printer
imports `config` again to suggest names, which would run this body twice.
"""

from config import DEBUG

LEVEL = 2 if DEBUG else 0
from config import READY  # noqa: E402

print("loader: unreachable")


def ready() -> bool:
    return READY
