from typing import TYPE_CHECKING


def f() -> int:
    try:
        pass
    finally:
        if TYPE_CHECKING:
            return 1
    return 0


def main() -> None:
    print(f())
