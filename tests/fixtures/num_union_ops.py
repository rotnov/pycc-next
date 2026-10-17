"""`int | float` (v0.6 accept 3): every operation on a value that is an
`int` at one point and a `float` at another, with CPython's semantics for
whichever it holds."""

import math


def show(x: int | float) -> None:
    print(x, str(x), repr(x), f"{x}", f"{x!r}", f"[{x:>8}]", abs(x), -x, +x)
    # `hash(nan)` is identity-based in CPython 3.13, so it is not printed.
    print(float(x), round(x, 1), round(x, -1), bool(x), hash(x) if x == x else -1)
    try:
        print(int(x), round(x), math.floor(x), math.ceil(x))
    except ValueError as e:
        print("ValueError:", e)
    except OverflowError as e:
        print("OverflowError:", e)
    print(x == 2, x != 2, x < 2.5, x >= 1, 2 < x, x == 2.0, 2.0 <= x)
    print(x + 1, x - 1.5, x * 3, x / 4, x // 3, x % 3, 2 ** (x % 5))
    try:
        print(x ** 2)
    except OverflowError as e:
        print("OverflowError:", e)
    print(math.sqrt(abs(x)), min(x, 1), max(x, 1.5), min(1, x), max(x, x))
    print(f"{x:.3f}", f"{x:e}", f"{x:g}", format(x, "10.2f"), f"{x:+}")
    print(isinstance(x, int), isinstance(x, float), isinstance(x, bool), isinstance(x, (int, float)))
    if x:
        print("truthy", not x)
    else:
        print("falsy", not x)


for value in [2, 2.5, -7, -0.0, 0, 3.0, float("inf"), float("nan"), 1e-5]:
    show(value)
show(1e300)
show(1e-300)
show(True)
show(10 ** 20)
show(-(2 ** 70))


def mixed(a: int | float, b: int | float) -> None:
    print(a + b, a - b, a * b, a == b, a < b, a > b, min(a, b), max(a, b))
    try:
        print(a / b)
    except ZeroDivisionError as e:
        print("ZeroDivisionError:", e)
    try:
        print(a // b, a % b)
    except ZeroDivisionError as e:
        print("ZeroDivisionError:", e)
    try:
        # A negative base to a fractional power is complex: not printed.
        if a < 0 and b % 1 != 0:
            print("complex")
        else:
            print(a ** b if b != 2 ** 64 else a ** (b - 2 ** 64 + 3))
    except ZeroDivisionError as e:
        print("ZeroDivisionError:", e)
    except OverflowError as e:
        print("OverflowError:", e)


for a in [3, 3.0, -2, 0.5, 0]:
    for b in [2, 2.0, 0, 0.0, -1, 0.5]:
        mixed(a, b)
    mixed(a, 2 ** 64)

# Exact comparisons between big ints and floats.
big: int | float = 2 ** 53 + 1
print(big == 2.0 ** 53, big > 2.0 ** 53, big == 2 ** 53 + 1, big < float("inf"))
huge: int | float = 10 ** 400
print(huge > 1e308, huge < float("inf"), huge == huge)
try:
    print(huge + 0.5)
except OverflowError as e:
    print("OverflowError:", e)
try:
    print(float(huge))
except OverflowError as e:
    print("OverflowError:", e)
nan: int | float = float("nan")
print(nan == nan, nan != nan, nan < 1, nan > 1)
try:
    print(int(nan))
except ValueError as e:
    print("ValueError:", e)
try:
    print(round(float("inf")))
except OverflowError as e:
    print("OverflowError:", e)
print(round(1234, -2), round(1250, -2), round(1350, -2), round(-1250, -2), round(5, -9))
print(hash(1), hash(1.0), hash(1.5), hash(-1), hash(-1.0), hash(2 ** 61), hash(float(2 ** 61)))
z: int | float = 1.5
z = 10 ** 30
print(z, z * 2, z // 7)
z = 0.1
print(z * 3, sum([0.1, 0.2, 0.3, 1]), sum([1, 2, 3.5]), sum([1, 2 ** 60, 0.5]))
