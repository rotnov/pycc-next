# `int ** <negative int literal>` is a float, as in CPython.
EPS = 10**-10
print(EPS, 2 ** -1, True ** -3, 7 ** -2)


def half(n: int) -> float:
    return n * 2 ** -1


print(half(5))
x = 3
print(x ** -2, (x - 3 + 10 ** 400 // 10 ** 399) ** -1)
try:
    print((x - 3) ** -1)
except ZeroDivisionError as e:
    print("ZeroDivisionError", e)
try:
    print((10 ** 400) ** -1)
except OverflowError as e:
    print("OverflowError", e)
