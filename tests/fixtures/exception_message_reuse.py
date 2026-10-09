# Issue #1298: a caught exception's message must survive any number of
# renderings -- `print(e)` and f-string interpolation each borrow the
# message the exception object owns, and a raise whose message is a
# variable or attribute gives the exception its own reference.


class Box:
    def __init__(self, s: str) -> None:
        self.s = s


class AppError(Exception):
    pass


class NarrowError(ValueError):
    pass


def fails(n: int) -> int:
    if n > 0:
        raise ValueError(f"n={n}")
    return n


def local_message() -> None:
    msg: str = "local " + "message"
    raise ValueError(msg)


def handle(n: int) -> str:
    try:
        fails(n)
        return "ok"
    except ValueError as e:
        print(e)
        return f"handled {e}"


def render(e: ValueError) -> str:
    s: str = f"{e}"
    return s


def raise_param(m: str) -> None:
    raise ValueError(m)


def reraise() -> None:
    try:
        raise ValueError("inner boom")
    except ValueError as e:
        print(e)
        print(e)
        raise


# Literal message, repeated use, including a multi-argument print.
try:
    raise ValueError("boom")
except ValueError as e:
    print(e)
    print(e)
    print(e)
    print(e, e)
    print(e, "x", e)

# Non-literal messages: f-string and concatenation.
count: int = 3
try:
    raise ValueError(f"bad {count}")
except ValueError as e:
    print(e)
    print(e)
prefix: str = "x"
try:
    raise ValueError(prefix + "y")
except ValueError as e:
    print(e)
    print(e)

# A variable message: the variable stays usable after the handler.
msg: str = "from var"
try:
    raise ValueError(msg)
except ValueError as e:
    print(e)
    print(e)
print(msg)

# An attribute message: the attribute stays usable after the handler.
b: Box = Box("attr msg")
try:
    raise ValueError(b.s)
except ValueError as e:
    print(e)
    print(e)
print(b.s)

# The message's owner is overwritten before the first use: a reassigned
# variable, an overwritten attribute, and a caller's temporary argument.
owner: str = "first " + "owner"
try:
    raise ValueError(owner)
except ValueError as e:
    owner = "second " + "owner"
    print(e)
    print(owner)
    print(e)
ob: Box = Box("attr " + "one")
try:
    raise ValueError(ob.s)
except ValueError as e:
    ob.s = "attr " + "two"
    print(e)
    print(ob.s)
    print(e)
try:
    raise_param("pa" + "ram")
except ValueError as e:
    print(e)
    print(e)

# A function-local message that escapes through the exception edge.
try:
    local_message()
except ValueError as e:
    print(e)
    print(e)

# f-string interpolation, repeated, then a direct print.
try:
    raise ValueError("fmt")
except ValueError as e:
    s: str = f"got {e}"
    print(s)
    print(f"got {e}!")
    print(e)

# A single-interpolation f-string: the rendered message is the f-string's
# own result, released by whoever consumes it.
try:
    raise ValueError("solo")
except ValueError as e:
    r: str = render(e)
    print(r)
    u: str = f"{e}"
    print(u)
    print(f"{e}")
    print(e)

# OSError family.
try:
    raise FileNotFoundError("nope")
except FileNotFoundError as e:
    print(e)
    print(e)
try:
    raise OSError("os boom")
except OSError as e:
    print(e)
    print(e)

# User-defined exception classes, caught by a base.
try:
    raise AppError("mine")
except Exception as e:
    print(e)
    print(e)
try:
    raise NarrowError("narrow")
except ValueError as e:
    print(e)
    print(e)

# A runtime-raised builtin exception.
xs: list[int] = [1]
try:
    y: int = xs[3]
except IndexError as e:
    print(e)
    print(e)

# Raised from a called function; handled inside a function, repeatedly.
try:
    fails(5)
except ValueError as e:
    print(e)
    print(e)
print(handle(1))
print(handle(1))
print(handle(0))

# Bare `raise` after using the binding, caught again outside.
try:
    reraise()
except ValueError as e:
    print(e)
    print(e)

# The same name bound by consecutive handlers, and inside a loop.
try:
    raise ValueError("first")
except ValueError as e:
    print(e)
try:
    raise ValueError("second")
except ValueError as e:
    print(e)
    print(e)
for i in range(3):
    try:
        raise ValueError(f"it {i}")
    except ValueError as e:
        print(e)
        print(e)

# Rebinding the handler name to another caught exception.
try:
    raise ValueError("outer")
except ValueError as e:
    print(e)
    try:
        raise ValueError("inner")
    except ValueError as e2:
        e = e2
    print(e)
    print(e)
print("done")
