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


def edited_then_reraised(edit: object) -> int:
    try:
        raise ValueError("v") from IndexError("old")
    except ValueError:
        edit()
        raise


def edited_inner_sees(edit: object) -> str:
    try:
        raise ValueError("v") from IndexError("old")
    except ValueError as e:
        edit()
        try:
            raise e
        except ValueError:
            return handled_links()


def _fails(message: str) -> None:
    raise ValueError(message)


def _caught() -> ValueError | None:
    # Raised in a callee and handed back from a handler that runs nothing
    # that could see it: it has traceback records and no CPython object.
    try:
        _fails("saved")
    except ValueError as e:
        return e
    return None


def raises_from_unpublished() -> None:
    # Given as a cause while another exception is handled: that one is not
    # its context.
    saved = _caught()
    if saved is None:
        return
    try:
        raise IndexError("handled")
    except IndexError:
        raise RuntimeError("new") from saved
