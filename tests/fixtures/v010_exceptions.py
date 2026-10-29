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


class Checked(Exception):
    """A `__new__` of its own, which validates and sets an attribute: it runs,
    as the MRO names it, before the layout's instance exists."""

    def __new__(cls, code, *rest):
        if code < 0:
            raise ValueError("negative code")
        self = super().__new__(cls, code, *rest)
        self.origin = "new"
        return self

    def __init__(self, code: int) -> None:
        super().__init__(code)
        self.code = code

    def show(self) -> str:
        return self.origin + ":" + str(self.code)


class Validated(Checked):
    """A layout over one whose base defines `__new__`."""

    def __init__(self, code: int, label: str) -> None:
        super().__init__(code)
        self.label = label

    def show(self) -> str:
        return super().show() + "/" + self.label


class NoFile(OSError):
    """`errno` is `OSError`'s descriptor, which `str()` reads: the store stays
    CPython's; `path` is native."""

    def __init__(self, code: int, path: str) -> None:
        super().__init__(code, "no such thing")
        self.errno = code
        self.path = path

    def show(self) -> str:
        return self.path + ":" + str(self.errno)


def probe(n: int) -> str:
    try:
        if n > 0:
            raise NoFile(n, "/x")
        raise Validated(n, "zero")
    except NoFile as e:
        return e.show() + " " + str(e)
    except Checked as e:
        return e.show()


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
