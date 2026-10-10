# `math`'s integer functions: gcd/lcm (also variadic), isqrt, floor/ceil of
# floats and ints, through both import spellings, including bigints and the
# error cases.
import math
from math import gcd, isqrt

print(math.gcd(12, 18), gcd(-4, 6), math.gcd(0, 0), math.gcd(7, 0))
print(math.gcd(12, 18, 27), math.lcm(4, 6), math.lcm(0, 5), math.lcm(-3, 4, 10))
print(math.gcd(2**80, 6**40), math.lcm(2**70, 3**5))
print(isqrt(0), isqrt(99), isqrt(100), math.isqrt(10**40 + 7))
print(math.floor(2.5), math.floor(-2.5), math.ceil(2.5), math.ceil(-2.5))
print(math.floor(7), math.ceil(-7), math.floor(1e20), math.ceil(-3.0))


def total(n: int) -> int:
    s = 0
    for i in range(1, n + 1):
        s += math.gcd(i, n) + isqrt(i)
    return s


print(total(30))
try:
    print(math.isqrt(-1))
except ValueError as e:
    print("ValueError:", e)
try:
    print(math.floor(float("inf")))
except OverflowError as e:
    print("OverflowError:", e)
print(math.ceil(float("nan")))
