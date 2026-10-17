# Non-strict per-function fallback (#12): compiled and fallback code call
# each other, share module globals, and raise exceptions across the
# boundary. Oracle: CPython runs this file unchanged.


def scale(x: int, k: int) -> int:
    # Compiled; called from fallback code.
    return x * k


def describe(values):
    # Compiled: CPython creates its lambda, whose body is compiled.
    key = lambda v: -v
    return ",".join(str(v) for v in sorted(values, key=key))


def fails(n: int) -> int:
    # Compiled; its exception is caught in fallback code.
    if n > 2:
        raise ValueError("too big: " + str(n))
    return n


def guarded(n):
    # Compiled (its `def` may run in CPython for its `int` or `str` result);
    # calls compiled `fails` and catches its exception.
    check = lambda m: fails(m)
    try:
        return check(n)
    except ValueError as e:
        return "caught " + str(e)


def boom(n):
    # Falls back (it calls a lambda expression); its exception is caught
    # in compiled code.
    return (lambda: 10 // n)()


counter = 0


def bump():
    # Falls back; rebinds a module global compiled code reads.
    global counter
    counter = (lambda c: c + 1)(counter)


total = sum(scale(i, 3) for i in range(4))
print(total)
print(describe((3, 1, 2)))
print(guarded(1))
print(guarded(5))
bump()
bump()
print(counter)
counter = counter + 10
bump()
print(counter)
try:
    boom(0)
except ZeroDivisionError as e:
    print("compiled caught", e)
print(boom(2))
print(scale(7, 6))
boom(0)
