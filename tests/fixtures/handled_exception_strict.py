# Strict `--ext`: an `except` clause publishes the exception it handles.
# Every function is typed, so the whole module is compiled; the test's
# driver, run by CPython, calls each one and prints what it raised, with
# the chain of contexts and causes, as CPython does for the source.
import sys


def handled_name() -> str:
    try:
        raise ValueError("v")
    except ValueError:
        return repr(sys.exception())


def miss_in_handler(x: int) -> int:
    d: dict[int, int] = {1: 2}
    try:
        raise ValueError("v")
    except ValueError:
        return d[x]


def tuple_miss_in_handler(name: str, n: int) -> int:
    d: dict[tuple[str, int], int] = {("a", 1): 2}
    key = (name, n)
    try:
        raise ValueError("v")
    except ValueError:
        return d[key]


def str_miss_in_handler(key: str) -> int:
    d: dict[str, int] = {"a": 1}
    try:
        raise ValueError("v")
    except ValueError:
        return d[key]


def raises_new(key: str) -> int:
    d: dict[str, int] = {"a": 1}
    try:
        return d[key]
    except KeyError:
        raise ValueError("in the handler")


def raises_from_none(key: str) -> int:
    d: dict[str, int] = {"a": 1}
    try:
        return d[key]
    except KeyError:
        raise ValueError("no context shown") from None


def raises_from_cause(key: str) -> int:
    d: dict[str, int] = {"a": 1}
    try:
        return d[key]
    except KeyError:
        raise ValueError("caused") from IndexError("the cause")


def raises_from_handled(key: str) -> int:
    d: dict[str, int] = {"a": 1}
    try:
        return d[key]
    except KeyError as e:
        raise ValueError("caused by the handled one") from e


def raises_again_from_none() -> int:
    try:
        raise ValueError("new") from IndexError("old")
    except ValueError as e:
        raise e from None


def raises_stored(key: str) -> int:
    d: dict[str, int] = {"a": 1}
    first: KeyError | None = None
    try:
        return d[key]
    except KeyError as e:
        first = e
    try:
        raise ValueError("second")
    except ValueError:
        if first is not None:
            raise first
        return 0


def raises_outer_in_inner(key: str) -> int:
    d: dict[str, int] = {"a": 1}
    try:
        return d[key]
    except KeyError as a:
        try:
            raise ValueError("b")
        except ValueError:
            raise a


def default_after_miss(key: str) -> int:
    d: dict[str, int] = {"a": 1}
    try:
        return d[key]
    except KeyError:
        return -1


def handled_links() -> str:
    e = sys.exception()
    return repr(e.__cause__) + " " + repr(e.__suppress_context__)


def inner_sees_from_none() -> str:
    try:
        raise ValueError("new") from IndexError("old")
    except ValueError as e:
        try:
            raise e from None
        except ValueError:
            return handled_links()


def inner_sees_new_cause() -> str:
    try:
        raise ValueError("new") from IndexError("old")
    except ValueError as e:
        try:
            raise e from KeyError("other")
        except ValueError:
            return handled_links()


def self_reraise_from_self() -> None:
    try:
        raise ValueError("v")
    except ValueError as e:
        raise e from e
