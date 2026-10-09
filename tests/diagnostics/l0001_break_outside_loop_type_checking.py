from typing import TYPE_CHECKING


def f() -> int:
    if TYPE_CHECKING:
        break
    return 0


def main() -> None:
    print(f())
