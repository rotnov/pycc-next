"""A class name bound by two class statements (v0.10 accept 5,
`tests/v010_exceptions.rs`): the first `B` derives from `A` and `OSError`,
then a second `B` rebinds the name. Every class statement counts, so `A`
gets no native layout, and the module behaves as its source does."""


class A(Exception):
    def __init__(self, code: int) -> None:
        self.code = code

    def show(self) -> str:
        return "a " + str(self.code)


class B(A, OSError):
    pass


class B(Exception):  # noqa: F811
    pass


def make(n: int) -> str:
    try:
        raise A(n)
    except A as e:
        return e.show()
