# A call of a module-level function before its `def` statement has run
# raises CPython's `NameError: name '<f>' is not defined`, catchable like
# any other exception (issue #22's call-before-def path).


def side(tag: str) -> int:
    print("evaluated", tag)
    return 1


def early() -> int:
    return late(2) + 1


def early_arg() -> int:
    # CPython looks the callee up before it evaluates an argument, so
    # `side` never runs while `late` is unbound.
    return late(side("argument"))


def early_void() -> None:
    shout("hi")


def early_twice() -> int:
    return redefined() * 100


def nested() -> int:
    try:
        return early()
    except NameError as e:
        print("nested caught:", e)
        return -1


def deep(n: int) -> int:
    if n == 0:
        return early()
    return deep(n - 1) + 1


try:
    print(early())
except NameError as e:
    print("caught:", e)

try:
    print(early_arg())
except NameError as e:
    print("caught:", e)

try:
    early_void()
except NameError as e:
    print("caught:", e)

try:
    print(early_twice())
except NameError as e:
    print("caught:", e)

print(nested())

try:
    print(deep(3))
except ValueError:
    print("not a ValueError")
except Exception as e:
    print("deep:", e)

try:
    early()
except NameError as e:
    print("again:", e)
finally:
    print("finally runs")


def late(x: int) -> int:
    return x * 10


def shout(s: str) -> None:
    print(s.upper())


def redefined() -> int:
    return 1


print(early())
print(early_arg())
early_void()
print(early_twice())
print(nested())
print(deep(3))


def redefined() -> int:
    return 2


print(early_twice())
