"""A circular import inside a package: CPython names the partially
initialized module and its file."""

print("main: start")
from app import settings  # noqa: E402

print("main: unreachable", settings.MODE)
