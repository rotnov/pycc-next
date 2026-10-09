from enum import Enum


class Color(Enum):
    RED = 1
    GREEN = 2


def main() -> None:
    c = Color()
    print(c.value)


main()
