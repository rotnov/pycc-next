import math
from math import log, log2, sqrt, ceil, floor, gcd

print(math.sqrt(4), sqrt(2.0), math.sqrt(True))
print(log(1), log(100, 10), log(8, 2), log(2.5, 1.5), math.log(10**400))
print(log2(1024), log2(2**2000), log2(3.0), math.log10(1000), math.log10(0.001))
print(math.exp(1), math.exp(-1000.0), math.sin(0), math.cos(0.0), math.tan(1))
print(math.asin(1), math.acos(0.5), math.atan(1), math.atan2(1, -1))
print(math.fabs(-3), math.hypot(3, 4), math.copysign(2, -0.0))
print(math.pi, math.e, math.tau, math.inf, -math.inf, math.nan)
print(math.factorial(0), math.factorial(25), math.comb(50, 25), math.comb(3, 5))
print(math.perm(5, 2), math.perm(3, 5), math.comb(10, 0))
print(ceil(log2(1000)), floor(sqrt(99)), gcd(12, 18))
print(int(log(10**18, 10) + 1e-9))


def attempt(label: str, which: int) -> None:
    try:
        if which == 0:
            print(math.sqrt(-1))
        elif which == 1:
            print(log(0))
        elif which == 2:
            print(math.log(-2.5))
        elif which == 3:
            print(math.exp(1000))
        elif which == 4:
            print(math.log(8, 1))
        elif which == 5:
            print(math.acos(2))
        elif which == 6:
            print(math.factorial(-1))
        elif which == 7:
            print(math.comb(-1, 2))
        elif which == 8:
            print(math.perm(5, -2))
        elif which == 9:
            print(math.sqrt(10**400))
        else:
            print(math.log2(0.0))
    except ValueError as e:
        print(label, "ValueError", e)
    except OverflowError as e:
        print(label, "OverflowError", e)
    except ZeroDivisionError as e:
        print(label, "ZeroDivisionError", e)


for i in range(11):
    attempt("case" + str(i), i)
print(math.log(0))
