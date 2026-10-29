"""Exception classes with members of their own (v0.10 accept 5): CPython makes
each from its statement, and the attributes their methods store from typed
parameters live in a native layout (`tests/v010_exceptions.rs`)."""


class Owned(Exception):
    """A message for `args`, two native attributes, a method using both."""

    def __init__(self, code: int, where: str) -> None:
        super().__init__(f"code {code}")
        self.code = code
        self.where = where

    def describe(self) -> str:
        return self.where + ":" + str(self.code)

    def bump(self, by: int) -> int:
        self.code += by
        return self.code


class Coded(Exception):
    """`args` holds exactly the constructor's argument: it pickles."""

    def __init__(self, code: int) -> None:
        super().__init__(code)
        self.code = code

    def __str__(self) -> str:
        return "coded " + str(self.code)


class Silent(Exception):
    """No `super().__init__`: `args` is what the constructor got."""

    def __init__(self, level: int, why: str) -> None:
        self.level = level
        self.why = why

    def __repr__(self) -> str:
        return "Silent(" + self.why + ")"


class Deeper(Owned):
    """A layout over another: its instance extends `Owned`'s."""

    def __init__(self, code: int, where: str, depth: int) -> None:
        super().__init__(code, where)
        self.depth = depth

    def describe(self) -> str:
        return super().describe() + "/" + str(self.depth)

    def total(self) -> int:
        return self.code + self.depth


class Tag(Owned):
    pass


def fail(n: int) -> None:
    if n > 10:
        raise Deeper(n, "deep", n // 10)
    if n > 2:
        raise Owned(n, "fail")
    raise Coded(n)


def run(n: int) -> str:
    try:
        fail(n)
    except Deeper as e:
        return "deeper " + e.describe() + " " + str(e.total())
    except Owned as e:
        return "owned " + e.describe() + " " + str(e.code)
    except Coded as e:
        return "coded " + str(e) + " " + str(e.code)
    return "none"


def churn(n: int) -> int:
    total = 0
    for i in range(n):
        try:
            fail(i % 20)
        except Owned as e:
            total += e.bump(1)
        except Coded as e:
            total += e.code
    return total
