"""An exception class reached through an alias (v0.10 accept 5,
`tests/v010_exceptions.rs`): `class Through(Alias, OSError)` works as it
does on CPython."""


class Aliased(Exception):
    def __init__(self, code: int) -> None:
        self.code = code

    def show(self) -> str:
        return "aliased " + str(self.code)


class Plain(Exception):
    """An exception class beside the aliased one."""

    def __init__(self, n: int) -> None:
        self.n = n


Alias = Aliased


class Through(Alias, OSError):
    def __init__(self, code: int) -> None:
        super().__init__(code)
        self.code = code


def run(n: int) -> str:
    try:
        raise Through(n)
    except Aliased as e:
        return e.show() + " " + type(e).__name__


def plain(n: int) -> int:
    try:
        raise Plain(n)
    except Plain as e:
        return e.n
