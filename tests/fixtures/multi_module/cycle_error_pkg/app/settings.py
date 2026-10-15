"""Imports `app.checks` before binding `MODE`."""

LIMIT = 5
from app import checks  # noqa: E402

MODE = "fast" if checks.OK else "slow"
