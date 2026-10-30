"""An exception class subclassed by code `exec` runs (v0.10 accept 5,
`tests/v010_exceptions.rs`): what the planner cannot read withdraws every
native layout of the module."""


class Base(Exception):
    def __init__(self, code: int) -> None:
        self.code = code


def made():
    exec("class Made(Base, OSError):\n    pass\n", globals())
    return globals()["Made"]
