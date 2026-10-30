"""A base spelled like a builtin but imported (v0.10 accept 5,
`tests/v010_exceptions.rs`): `helper.Exception` (an uncompiled module next to
this one) is not the builtin -- its `code` property takes `self.code = code`."""
from helper import Exception


class Err(Exception):
    def __init__(self, code: int) -> None:
        self.code = code

    def show(self) -> str:
        return "err " + str(self.code)


def make(n: int) -> str:
    try:
        raise Err(n)
    except Err as e:
        return e.show()
