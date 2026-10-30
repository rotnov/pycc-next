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


class Meta(type):
    """A metaclass with a `__new__` of its own."""

    def __new__(mcls, name, bases, ns):
        ns["tagged"] = name.lower()
        return super().__new__(mcls, name, bases, ns)


class Metabase(Exception, metaclass=Meta):
    pass


class Metered(Metabase):
    """Over a base of another metaclass: no layout, CPython's class."""

    def __init__(self, code: int) -> None:
        super().__init__(code)
        self.code = code

    def show(self) -> str:
        return self.tagged + ":" + str(self.code)


class Plated(Exception):
    """Would have a layout, but `Shown` shadows `code`: CPython's class."""

    def __init__(self, code: int) -> None:
        super().__init__(code)
        self.code = code

    def show(self) -> str:
        return "plated " + str(self.code)


class Shown(Plated):
    """`code` is a property here: `Plated.__init__`'s store reaches it."""

    @property
    def code(self) -> int:
        return 42

    @code.setter
    def code(self, value: int) -> None:
        self.stored = value


class Left(Exception):
    """Would have a layout, but `Both` derives from it and `Right`."""

    def __init__(self, a: int) -> None:
        self.a = a


class Right(Exception):
    def __init__(self, b: str) -> None:
        self.b = b


class Both(Left, Right):
    """Two bases that would carry a layout each: CPython's classes."""

    def __init__(self, a: int, b: str) -> None:
        Left.__init__(self, a)
        Right.__init__(self, b)

    def show(self) -> str:
        return str(self.a) + self.b


class KeyOS(KeyError, OSError):
    """Several bases of different solid bases: no layout of its own."""

    def __init__(self, n: int) -> None:
        self.n = n

    def show(self) -> str:
        return "keyos " + str(self.n)


class Lazy(Exception):
    """`__getattr__` is off the allowlist: CPython's class."""

    def __init__(self, code: int) -> None:
        self.code = code

    def __getattr__(self, name):
        return "lazy " + name


class Reducer(Exception):
    """A `__reduce__` of its own, which a subclass inherits."""

    def __reduce__(self):
        return (type(self), (self.code + 100,))


class Reduced(Reducer):
    """Over a base reducing itself: no layout."""

    def __init__(self, code: int) -> None:
        self.code = code


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
