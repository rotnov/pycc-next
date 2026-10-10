"""Arbitrary-precision `int` arithmetic, compared with CPython 3.13.

Covers the 63-bit smallint boundary (+/-2**62) and the i64 boundary
(+/-2**63) in both directions, values far beyond both, and the conversions
and errors CPython defines for them.
"""


def factorial(n: int) -> int:
    result = 1
    for i in range(2, n + 1):
        result *= i
    return result


def fib(n: int) -> int:
    a = 0
    b = 1
    for _ in range(n):
        a, b = b, a + b
    return a


def doubling(steps: int) -> int:
    x = 1
    for _ in range(steps):
        x = x * 2
    return x


def collatz_steps(n: int) -> int:
    steps = 0
    while n != 1:
        if n % 2 == 0:
            n = n // 2
        else:
            n = 3 * n + 1
        steps += 1
    return steps


small_max = 4611686018427387903
small_min = -4611686018427387904
i64_max = 9223372036854775807
i64_min = -9223372036854775808

print("-- boundaries")
print(small_max + 1, small_min - 1, small_max * 2, small_min * 2)
print(i64_max + 1, i64_min - 1, -i64_min, i64_max * i64_max)
print(small_max + 1 - 1, small_min - 1 + 1, (small_max + 1) // 2)
print(-small_min, small_min // -1, small_min % -1, abs(small_min))
print(small_max + 1 == 4611686018427387904, small_min - 1 < small_min)
print(doubling(61), doubling(62), doubling(63), doubling(64), doubling(200))

print("-- large values")
print(factorial(100))
print(fib(300))
print(2**200, -(2**200), 3**150)
print(2**1000 % 1000007, 7**222 // 7**220, 2**(2**10) == 2**1024)
print(1606938044258990275541962092341162602522202993782792835301376)
print(-0xFFFF_FFFF_FFFF_FFFF_FFFF_FFFF, 0o7777777777777777777777777, 0b1 << 100)
print(collatz_steps(2**100 + 1))

print("-- floor division and modulo")
big = 10**30 + 7


def floor_row(a: int) -> None:
    print(a // 7, a % 7, a // -7, a % -7)
    print(a // (big - 7), a % (big - 7), a // -(big - 7), a % -(big - 7))
    print(a // 10**40, a % 10**40)


floor_row(big)
floor_row(-big)
floor_row(17)
floor_row(-17)
q, r = divmod(-(10**25), 3)
print(q, r, divmod(7, -2)[0], divmod(-7, 2)[1])

print("-- shifts and bitwise")
print(1 << 62, 1 << 63, 1 << 64, -1 << 100, (2**100) >> 99, (-(2**100)) >> 99)
print((2**100 + 5) >> 1000, (-(2**100)) >> 1000, 5 >> 70, -5 >> 70)
print((2**80 - 1) & (2**70 + 3), (2**80) | 1, (2**80) ^ (2**80 - 1), ~(2**70))
print(-(2**80) & 0xFFFF, (-(2**80)) | 7, 12345 & -(2**64))

print("-- comparisons")
print(2**64 > 2**63, -(2**64) < -(2**63), 2**64 == 2**64, 2**64 != 2**64 + 1)
print(2**64 > 5, 5 < 2**64, -(2**64) < -5, 2**100 >= 2**100, 2**100 <= 2**99)
print(2**53 + 1 == 2.0**53, 2**53 + 1 > 2.0**53, 10**400 > 1e308, -(10**400) < -1e308)
print(3 == 3.0, 3.5 > 3, 2**63 == 9223372036854775808.0, -(2**63) <= -9.223372036854775808e18)

print("-- int and float")
print(float(2**60) == 2.0**60, float(-(2**62)) == -(2.0**62), float(2**53 + 1) == 2.0**53, float(2**40))
print(10**400 // 10**399, 10**400 / 10**399, 2**64 / 2**20, 1 / 3, 7 / -2)
print(int(1e20), int(-2.7), int(3.99), int(True), int(2**100))
print(2**100 * 0.5 > 6e29, 1.5 + 2**62 == 2.0**62, 2**62 - 0.5 == 2.0**62, 2**100 * 0.0)
print(abs(-(2**100)), abs(-5), abs(True), abs(-2.5), abs(2**63))

print("-- bool as int")
print(True + 2**62, False - 2**62, True * 2**63, (2**64) // True, -True)

print("-- formatting and hashing")
x = 2**100
print(f"{x}|{-x}|{small_max + 1}")
print(f"{x * 0}", hash(x), hash(-x), hash(2**61 - 1), hash(2**61), hash(-(2**63)))

print("-- errors")
try:
    print(x // 0)
except ZeroDivisionError as e:
    print("ZeroDivisionError", e)
try:
    print(x % 0)
except ZeroDivisionError as e:
    print("ZeroDivisionError", e)
try:
    print(divmod(x, 0)[0])
except ZeroDivisionError as e:
    print("ZeroDivisionError", e)
try:
    print(x / 0)
except ZeroDivisionError as e:
    print("ZeroDivisionError", e)
try:
    print(5 // 0)
except ZeroDivisionError as e:
    print("ZeroDivisionError", e)
try:
    print(float(10**400))
except OverflowError as e:
    print("OverflowError", e)
try:
    print(10**400 / 3)
except OverflowError as e:
    print("OverflowError", e)
try:
    print(x << -1)
except ValueError as e:
    print("ValueError", e)
try:
    inf = 1e308 * 10.0
    print(int(inf))
except OverflowError as e:
    print("OverflowError", e)
try:
    nan = 1e308 * 10.0 - 1e308 * 10.0
    print(int(nan))
except ValueError as e:
    print("ValueError", e)
try:
    print(f"{10**5000}")
except ValueError as e:
    print("ValueError", e)
