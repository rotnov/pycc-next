# isinstance on a caught builtin exception value (#1337, WI-6a): the
# value's dynamic class may be any subclass, so a target outside its static
# MRO is decided from the runtime type tag. An except* binding and a plain
# user exception instance keep the compile-time answer.
class E(ValueError):
    pass


class F(E):
    pass


class Mixin:
    pass


class M(Mixin, KeyError):
    pass


class Both(ValueError, KeyError):
    pass


class Plain:
    pass


def caught_key() -> None:
    try:
        raise KeyError("k")
    except Exception as e:
        print(isinstance(e, KeyError), isinstance(e, ValueError), isinstance(e, Exception))


def caught_user() -> None:
    try:
        raise F("f")
    except ValueError as e:
        print(isinstance(e, E), isinstance(e, F), isinstance(e, KeyError))
    try:
        raise ValueError("v")
    except ValueError as e:
        print(isinstance(e, E), isinstance(e, ValueError))


def multiple() -> None:
    try:
        raise Both("b")
    except ValueError as e:
        print(isinstance(e, KeyError), isinstance(e, Both), isinstance(e, IndexError))
    try:
        raise M("m")
    except KeyError as e:
        print(isinstance(e, Mixin), isinstance(e, Plain))


def tuple_handler() -> None:
    for i in range(2):
        try:
            if i == 0:
                raise ValueError("a")
            raise KeyError("b")
        except (ValueError, KeyError) as e:
            print(isinstance(e, KeyError), isinstance(e, (IndexError, KeyError)), isinstance(e, (TypeError, IndexError)))


def runtime_error() -> None:
    try:
        n = 0
        print(10 // n)
    except Exception as e:
        print(isinstance(e, ZeroDivisionError), isinstance(e, ValueError))


def os_family() -> None:
    try:
        raise FileNotFoundError("nf")
    except Exception as e:
        print(isinstance(e, OSError), isinstance(e, FileNotFoundError), isinstance(e, PermissionError))


class Owned(ValueError):
    def __init__(self, n: int) -> None:
        self.n = n


def group() -> None:
    try:
        raise ValueError("v")
    except* ValueError as eg:
        print(isinstance(eg, ExceptionGroup), isinstance(eg, ValueError))


def plain_instance() -> None:
    f = Owned(3)
    print(isinstance(f, KeyError), isinstance(f, ValueError), isinstance(f, Owned))


caught_key()
caught_user()
multiple()
tuple_handler()
runtime_error()
os_family()
group()
plain_instance()
