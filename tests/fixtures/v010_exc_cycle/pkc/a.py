"""`C` (here) derives from `b.B(A)` and `OSError`."""


class A(Exception):
    def __init__(self, code: int) -> None:
        self.code = code

    def show(self) -> str:
        return "a " + str(self.code)


from .b import B  # noqa: E402


class C(B, OSError):
    pass


def make(n: int) -> str:
    try:
        raise C(n)
    except A as e:
        return e.show() + " " + type(e).__name__
