"""`A` would have a native layout, but `b` binds the name twice."""


class A(Exception):
    def __init__(self, code: int) -> None:
        self.code = code

    def show(self) -> str:
        return "a " + str(self.code)


def make(n: int) -> str:
    try:
        raise A(n)
    except A as e:
        return e.show()
