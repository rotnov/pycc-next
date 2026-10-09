# #1289: a name bound in a `try` body and in every handler that can fall
# through the statement is definitely bound after it, so a later read runs.
# pycc's stdout must match CPython 3.14.7 byte for byte. The caught
# `ZeroDivisionError` message is never printed: pycc's text differs from
# CPython's today, which is a separate defect.


class Bad(Exception):
    pass


def check(n: int) -> int:
    if n < 0:
        raise ValueError("negative")
    if n == 0:
        raise Bad("zero")
    return n


def function_scope(d: int) -> int:
    try:
        x = 10 // d
    except ZeroDivisionError:
        x = -1
    return x


def handler_returns(d: int) -> int:
    try:
        x = 10 // d
    except ZeroDivisionError:
        return -1
    return x


def handler_reraises(d: int) -> int:
    try:
        x = 10 // d
    except ZeroDivisionError:
        raise
    return x


def outer(d: int) -> int:
    try:
        return handler_reraises(d)
    except ZeroDivisionError:
        return -9


def else_rebinds(d: int) -> int:
    try:
        x = 10 // d
    except ZeroDivisionError:
        x = 0
    else:
        x = x - 7
    return x


def with_finally(d: int) -> int:
    count = 0
    try:
        x = 10 // d
    except ZeroDivisionError:
        x = 0
    finally:
        count = count + 1
    return x + count - 1


def multiple_handlers(n: int) -> int:
    try:
        x = check(n)
    except ValueError:
        x = -1
    except Bad:
        x = 0
    return x


def as_binding(n: int) -> int:
    try:
        x = check(n)
    except ValueError as e:
        print(e)
        x = -1
    return x


def in_a_loop() -> None:
    total = 0
    for d in range(2):
        try:
            try:
                x = 5 // d
            except ZeroDivisionError:
                x = 1
            total = total + x
        except ValueError:
            total = -100
            x = 0
        print(x)
    print(total)


def all_paths_return(d: int) -> int:
    try:
        return 10 // d
    except ZeroDivisionError:
        return 4


def str_binding(d: int) -> str:
    try:
        q = 10 // d
        s = "a"
        if q < 0:
            s = "b"
    except ZeroDivisionError:
        s = "c"
    return s + "!"


def raise_by_argument(kind: int) -> int:
    if kind == 0:
        raise ValueError("v")
    if kind == 1:
        raise TypeError("t")
    return 10


def except_star(kind: int) -> int:
    try:
        x = raise_by_argument(kind)
    except* ValueError:
        x = 1
    except* TypeError:
        x = 2
    return x


def two_as_handlers(kind: int) -> int:
    try:
        x = raise_by_argument(kind)
    except ValueError as e:
        print(e)
        x = 1
    except TypeError as e:
        print(e)
        x = 2
    return x


def int_handler_bool_else(d: int) -> None:
    try:
        y = 10 // d
    except ZeroDivisionError:
        x = 1
    else:
        x = y > 100
    print(x)


def bool_handler_over_int_body(d: int) -> None:
    try:
        x = 10 // d
    except ZeroDivisionError:
        x = False
    print(x)


def bool_handler_over_int_body_star(d: int) -> None:
    try:
        x = 10 // d
    except* ZeroDivisionError:
        x = False
    print(x)


def _solver_helper(d):
    try:
        r = 10 // d
    except ZeroDivisionError:
        r = -1
    return r


def _solver_print(d):
    try:
        x = 10 // d
    except ZeroDivisionError:
        x = False
    print(x)


def _solver_return(d):
    try:
        x = 10 // d
    except ZeroDivisionError:
        x = False
    return x


def _solver_return_star(d):
    try:
        x = 10 // d
    except* ZeroDivisionError:
        x = False
    return x


# Module scope.
d = 0
try:
    x = 10 // d
    ok = True
except ZeroDivisionError:
    x = -1
    ok = False
print(x, ok)

print(function_scope(0), function_scope(5))
print(handler_returns(0), handler_returns(5))
print(outer(2))
print(else_rebinds(0), else_rebinds(1))
print(with_finally(0), with_finally(3))
print(multiple_handlers(6), multiple_handlers(0), multiple_handlers(-1))
print(as_binding(-1))
print(as_binding(3))
in_a_loop()
try:
    y = 1
finally:
    z = 10
print(all_paths_return(0), z)
print(str_binding(0))
print(except_star(0), except_star(1), except_star(2))

# The #1063 pattern: two sequential `try`s whose bodies always raise, each
# binding `e` to a different exception type.
try:
    raise OverflowError("by name")
except OverflowError as e:
    print(e)
try:
    raise OverflowError("by base")
except Exception as e:
    print(e)

print(two_as_handlers(0))
print(two_as_handlers(1))
print(two_as_handlers(2))
try:
    m = raise_by_argument(0)
except ValueError as e:
    m = 1
except TypeError as e:
    m = 2
print("m", m)

int_handler_bool_else(0)
int_handler_bool_else(1)
bool_handler_over_int_body(0)
bool_handler_over_int_body(1)
bool_handler_over_int_body_star(0)
bool_handler_over_int_body_star(1)
print(_solver_helper(0), _solver_helper(5))
_solver_print(0)
_solver_print(1)
print(_solver_return(0), _solver_return(1))
print(_solver_return_star(0), _solver_return_star(1))
