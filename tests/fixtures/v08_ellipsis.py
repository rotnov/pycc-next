from typing import overload


@overload
def pick(x: int) -> int: ...
@overload
def pick(x: str) -> str: ...
def pick(x):
    return x


def stub(x):
    ...


def todo():
    """A docstring, then the placeholder."""
    ...
    return "after"


def marker():
    value = ...
    return value


def is_marker(x):
    return x is ...


def not_marker(x):
    return x is not Ellipsis


def shapes():
    return (1, ...), [..., ...], {...: "e"}


def main():
    print(stub(1), todo(), pick(3), pick("a"))
    m = marker()
    print(m, repr(m), str(m), m is Ellipsis, type(m).__name__)
    print(is_marker(m), is_marker(None), is_marker(marker()), not_marker(m), not_marker(0))
    print(*shapes())
    print(m == ..., m != ..., hash(m) == hash(Ellipsis), bool(m))
    xs = [0, 1, 2]
    print(xs[...])


main()
