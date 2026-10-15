"""Imports `plugins`, which calls back into this module while it runs."""


def early_size() -> int:
    return size() + 1


try:
    print(early_size())
except NameError as e:
    print("registry:", e)

import plugins  # noqa: E402


def lookup(n: int) -> int:
    return n * 3


def size() -> int:
    return lookup(2)


print("registry:", early_size(), plugins.run(1))
