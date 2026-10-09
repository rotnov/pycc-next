from enum import Enum


class Color(Enum):
    RED = 1
    GREEN = 2


def main() -> None:
    c = Color(1)
    print(c.value)


main()
