# #1210 (Part 2 of #1018): the shift and bitwise operators `<< >> & | ^` and
# their augmented forms on `int` and `bool` (docs/TYPE_SYSTEM.md, "Bitwise and
# shift operators"). Bigints are built only with `<<` and `+=` and are only
# printed: no bigint dict values (#1089), comparisons, `*` or `**` (#1040).
# Every raise is caught, so the fixture exits 0 under CPython.


class Flags:
    def __init__(self, n: int) -> None:
        self.n = n


def _g(a, b) -> int:
    return a & b


def int_ops(a: int, b: int) -> None:
    print(a << 3, a >> 1, a & b, a | b, a ^ b)
    x = a
    x <<= 2
    print(x)
    x >>= 1
    print(x)
    x &= b
    print(x)
    x |= 12
    print(x)
    x ^= b
    print(x)


def shift_error(a: int, n: int) -> None:
    try:
        print(a << n)
    except ValueError as e:
        print("ValueError:", e)
    try:
        print(a >> n)
    except ValueError as e:
        print("ValueError:", e)


int_ops(22, 7)
int_ops(-22, 7)
int_ops(22, -7)
int_ops(-22, -7)
int_ops(0, 0)

# Shifting past the inline range promotes to a bigint, and back down.
big = 1 << 70
print(big)
print(big >> 70, big >> 3, big >> 200)
neg = -big
print(neg >> 3, neg >> 70, neg >> 200)
neg -= 1
print(neg >> 3, neg >> 70)

# Bigint operands of `& | ^`, in two's-complement semantics.
p = 1 << 65
p += 12345
print(neg & p, -big & p)
q = -big
q += 7
r = 1 << 66
print(q | r)
u = -big
u -= 3
v = -(1 << 68)
v -= 99
print(u ^ v)
print(big & 0, big | 0, big ^ big)

# A bigint shift count.
huge = 1 << 70
print(0 << huge, 5 >> huge, -5 >> huge)

# A negative count raises `ValueError`.
shift_error(3, -1)
shift_error(-3, -2)

# `bool` results.
print(True & False, True | False, True ^ True, False ^ True)
print(True & 3, True | 4, False ^ 5, True << 1, True >> 1)
x: int = True
print(x & True, x & x, x | 0, x << 1)
flag = True
flag &= False
print(flag)
flag |= True
print(flag)
flag ^= True
print(flag)
print(_g(True, False), _g(6, 3))

# Attribute and `dict[str, int]` subscript targets.
f = Flags(5)
f.n <<= 4
print(f.n)
f.n >>= 2
print(f.n)
f.n &= 6
print(f.n)
f.n |= 9
print(f.n)
f.n ^= 3
print(f.n)
d: dict[str, int] = {"a": 5}
d["a"] <<= 3
d["a"] >>= 1
d["a"] &= 14
d["a"] |= 1
d["a"] ^= 8
print(d["a"])

# A plain value `|` at module scope (not a PEP 604 annotation).
READ = 4
WRITE = 2
FLAGS = READ | WRITE
print(FLAGS, FLAGS & ~WRITE)
