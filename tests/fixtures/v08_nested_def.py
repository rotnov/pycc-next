import functools
import operator


def traced(func):
    calls = []

    @functools.wraps(func)
    def wrapper(*args, **kwargs):
        calls.append(args)
        return func(*args, **kwargs)

    wrapper.calls = calls
    return wrapper


def counter():
    count = 0

    def bump(step=1):
        nonlocal count
        count += step
        return count

    def peek():
        return count

    return bump, peek


def late_binding():
    fs = []
    for i in range(3):
        def get():
            return i
        fs.append(get)
    return [f() for f in fs]


def defaults_now():
    x = 1

    def show(v=x):
        return v

    x = 2
    return show(), show.__defaults__


def recursive(n):
    def fact(k):
        return 1 if k <= 1 else k * fact(k - 1)

    return fact(n)


def rebinding():
    def make():
        return value

    value = "first"
    a = make()
    value = "second"
    return a, make()


def raising():
    def boom(x):
        raise ValueError(f"bad {x}")

    try:
        boom(3)
    except ValueError as e:
        return str(e)


def shared_loop():
    out = []
    for i, name in enumerate(["a", "b"]):
        def tag(prefix):
            return prefix + name + str(i)
        out.append(tag)
    return [f("-") for f in out]


def logged(func):
    # A plain wrapper over `*args` only, the decorator pattern without
    # `functools.wraps`.
    def call(*args):
        return func(*args) * 2

    return call


@logged
def add(a, b):
    return a + b


def main():
    t = traced(operator.mul)
    print(t(3, 2), t(4, 5), t.calls, t.__name__, t.__doc__, t.__wrapped__ is operator.mul)
    print(t.__qualname__, traced(len).__qualname__, traced(len).calls)
    bump, peek = counter()
    print(bump(), bump(5), peek(), bump.__name__, bump.__qualname__, peek.__defaults__)
    print(late_binding(), defaults_now(), recursive(5), rebinding(), raising())
    print(shared_loop())
    print(add(2, 3), add("x", "y"), add.__name__, add.__qualname__)
    print(repr(counter()[0]).split(" at ")[0])
    bump(None)


main()
