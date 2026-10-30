"""The module's `A` is `a.A`; the function's own import binds a local `A`."""
from .a import A


class B(A, OSError):
    pass


def other():
    from .other import OtherError as A

    return A
