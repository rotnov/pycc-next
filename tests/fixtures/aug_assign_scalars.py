# #1209 (Part 1 of #1018): augmented assignment on a name bound to an
# immutable value is the plain `x = x op value` (docs/TYPE_SYSTEM.md,
# "Augmented assignment"). Every admitted operator on `int`, `float` and
# `str`, at module and function scope. Only scalars are printed.
#
# `**=` uses non-negative exponents only (#1068 owns negative ones), and the
# bigint growth is a `+=` loop on a name: a bigint `*=`/`//=`/`%=`/`**=` is the
# inherited #1040 deviation, not something this fixture asserts.


def int_ops(a: int, b: int) -> None:
    x = a
    x += b
    print(x)
    x -= b
    print(x)
    x *= b
    print(x)
    x //= b
    print(x)
    x %= b
    print(x)
    x **= 2
    print(x)


def float_ops(a: float, b: float) -> None:
    y = a
    y += b
    print(y)
    y -= b
    print(y)
    y *= b
    print(y)
    y /= b
    print(y)
    y //= b
    print(y)
    y %= b
    print(y)
    y **= 2
    print(y)


int_ops(17, 5)
int_ops(-7, 3)
int_ops(7, -3)
float_ops(7.5, 2.0)
float_ops(-7.5, 2.0)

x = 10
x += 5
x -= 2
x *= 3
print(x)
x //= 4
print(x)
x %= 4
print(x)
x **= 3
print(x)

m = -7
m //= 2
print(m)
m %= 3
print(m)

y = 1.5
y /= 4.0
print(y)
y += 2
print(y)

s = "ab"
s += "cd"
print(s)
s *= 3
print(s)

n = 1
n += True
print(n)

t = 1
for _ in range(70):
    t += t
print(t)
