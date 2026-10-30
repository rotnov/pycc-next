"""Exception classes reached by class statements the layout planner does not
examine (v0.10 accept 5, `tests/v010_exceptions.rs`): a statement nested in
a function, under `if`, under `try`, or decorated. Each base used so gets no
native layout; `Kept`, used only by examined statements, keeps its own."""


class Kept(Exception):
    def __init__(self, code: int) -> None:
        self.code = code


class Factory(Exception):
    def __init__(self, code: int) -> None:
        self.code = code


class Conditional(Exception):
    def __init__(self, code: int) -> None:
        self.code = code


class Guarded(Exception):
    def __init__(self, code: int) -> None:
        self.code = code


class Decorated(Exception):
    def __init__(self, code: int) -> None:
        self.code = code


def factory():
    class Combined(Factory, OSError):
        pass

    return Combined


if True:

    class Cond(Conditional, OSError):
        pass


try:

    class Guard(Guarded, OSError):
        pass

except TypeError:
    pass


REGISTRY = []


def register(cls):
    """Receives the class: it could make a subclass of it at any time."""
    REGISTRY.append(cls)
    return cls


@register
class Dec(Decorated):
    pass


def kept(n: int) -> int:
    try:
        raise Kept(n)
    except Kept as e:
        return e.code
