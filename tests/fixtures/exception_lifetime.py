# Exception objects are reference counted: every holder below keeps its
# exception (and the exception's message, cause and members) alive exactly
# as long as CPython does, and the last release frees it. Run under
# valgrind by `tests/memory_leaks_exceptions.rs`: no read of a freed
# exception.
from typing import Generator, Iterator


class AppError(Exception):
    pass


class DbError(AppError):
    pass


class Keeper:
    def __init__(self, err: ValueError) -> None:
        self.err = err


def fail(n: int) -> int:
    if n > 0:
        raise ValueError("bad " + str(n))
    return n


def caught_in_callee(n: int) -> str:
    try:
        fail(n)
    except ValueError as e:
        return "callee " + f"{e}"
    return "none"


def keep_one(n: int) -> ValueError:
    try:
        fail(n)
    except ValueError as e:
        return e
    raise RuntimeError("unreachable")


def reraise_from_handler(n: int) -> None:
    try:
        fail(n)
    except ValueError as e:
        print("handling", e)
        raise


def chained(n: int) -> None:
    try:
        fail(n)
    except ValueError as e:
        raise AppError("wrapped") from e


def finally_replaces(n: int) -> int:
    try:
        fail(n)
    finally:
        raise KeyError("replaced " + str(n))


def finally_reads_binding(n: int) -> str:
    try:
        fail(n)
    except ValueError as e:
        try:
            return "early"
        finally:
            print("finally sees", e)
    return "late"


def break_out(n: int) -> int:
    count = 0
    for i in range(n):
        try:
            fail(i)
        except ValueError as e:
            print("break at", e)
            break
        count += 1
    return count


def continue_on(n: int) -> int:
    count = 0
    for i in range(n):
        try:
            fail(i % 2)
        except ValueError:
            continue
        count += 1
    return count


def gen(n: int) -> Generator[int, None, str]:
    yield n
    return "done " + str(n)


def stop_value(n: int) -> str:
    g = gen(n)
    next(g)
    try:
        next(g)
    except StopIteration as e:
        return e.value
    return "no stop"


def counting() -> Iterator[int]:
    try:
        yield 1
        yield 2
    except ValueError as e:
        print("generator caught", e)
        yield 3


def nested(n: int) -> str:
    try:
        try:
            fail(n)
        except ValueError as inner:
            print("inner", inner)
            raise KeyError("k" + str(n))
    except KeyError as outer:
        return f"outer {outer}"
    return "none"


def groups(n: int) -> None:
    try:
        raise ValueError("v" + str(n))
    except ValueError as v:
        try:
            raise KeyError("k")
        except KeyError as k:
            try:
                raise ExceptionGroup("many", [v, k])
            except* ValueError as vs:
                print("values caught")
            except* KeyError as ks:
                print("keys caught")


def main() -> None:
    for i in range(3):
        print(caught_in_callee(i))
    kept: list[ValueError] = []
    for i in range(1, 4):
        kept.append(keep_one(i))
    holder = Keeper(kept[1])
    kept = []
    print("held", holder.err)
    holder.err = keep_one(9)
    print("replaced", holder.err)
    try:
        reraise_from_handler(2)
    except ValueError as e:
        print("outer got", e)
    try:
        chained(3)
    except AppError:
        print("chained")
    try:
        finally_replaces(4)
    except KeyError as e:
        print("finally raised", e)
    print(finally_reads_binding(5))
    print(break_out(5), continue_on(6))
    print(stop_value(7))
    g = counting()
    print(next(g))
    print(g.throw(ValueError("thrown")))
    print(nested(8))
    groups(9)
    try:
        raise DbError("db")
    except Exception as e:
        first = e
        try:
            raise ValueError("second")
        except ValueError as e2:
            print("second", e2)
        print("first again", first)
    d = {"a": 1}
    for k in ["a", "b", "c"]:
        try:
            print(d[k])
        except KeyError as e:
            print("missing", e)
    print("end")


main()
