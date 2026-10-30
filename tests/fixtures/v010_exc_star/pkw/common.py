"""An exception class that would have a native layout, but `combo` imports
it with a wildcard."""


class Coded(Exception):
    def __init__(self, code: int) -> None:
        self.code = code

    def show(self) -> str:
        return "coded " + str(self.code)


def coded(n: int) -> str:
    try:
        raise Coded(n)
    except Coded as e:
        return e.show()
