# #603 (Part 2 of #573): a source-level `+`/`-` applied to an operand that is
# *not* a numeric literal. #602's fold handles `-1`; everything here is a case
# folding cannot reach -- a name, a call result, a parenthesized expression, an
# attribute, a subscript, and a nested unary.
#
# The two operand representations are exercised separately on purpose, because
# `pycc_mir` rewrites them into different binary shapes: `int`/`bool` become
# `0 - x` / `0 + x` (so bigint promotion is inherited from the runtime's own
# `int_sub`/`int_add`), while `float` becomes `x * -1.0` / `x * 1.0` (so `-0.0`
# and the infinities negate exactly, which `0.0 - x` would not do).

x = 5
print(-x)
print(+x)
print(-(-x))
print(-(+x))

y = 3
print(-(x + y))
print(-x + y)
print(-x * y)
print(-(x * y))
print(-x - -y)

# `bool` is not `int`-preserving under unary: `+True` is the integer `1`, and
# `-True` is `-1`.
t = True
f = False
print(-t)
print(+t)
print(-f)
print(+f)
print(-t + 1)

# `float`, including the signed zero and the infinities the multiplication
# rewrite exists to get right.
p = 2.5
print(-p)
print(+p)
print(-(-p))
print(-p + p)

zero = 0.0
print(-zero)
print(+zero)
print(-(-zero))

inf = 1e400
print(-inf)
print(+inf)
print(-(-inf))


def twice(n: int) -> int:
    return n * 2


print(-twice(4))
print(+twice(4))


def negated(n: int) -> int:
    return -n


def negated_float(v: float) -> float:
    return -v


print(negated(7))
print(negated(-7))
print(negated_float(1.25))
print(negated_float(-1.25))


# The operand crossing into bigint territory: negation must inherit arbitrary
# precision from the runtime's own `int_sub`, which `x * -1` would not give
# (`int_mul` rejects an already-promoted bigint operand).
base = 2000000000000000000
big = base * 4
print(big)
print(-big)
print(-(big + 1))
print(-big + big)
print(-(-big))


xs = [1, 2, 3]
print(-xs[0])
print(-xs[2])


class Point:
    def __init__(self, dx: int, dy: float) -> None:
        self.dx = dx
        self.dy = dy

    def flipped_x(self) -> int:
        return -self.dx


pt = Point(9, 4.5)
print(-pt.dx)
print(-pt.dy)
print(pt.flipped_x())
print(+pt.dx)


# `rename_name_in_expr` (D-117) rewrites a comprehension's loop variable to a
# synthesized internal name, and is exhaustive over `HirExpr` on purpose, so
# the unary node has to be renamed through it too.
ys = [-v for v in xs]
print(ys[0])
print(ys[2])
zs = [v for v in xs if -v < -1]
print(zs[0])
ss = {-v for v in xs}
print(len(ss))


# The generic-call and protocol-call specialization passes are exercised by
# `tests/issue_603_unary_general_operand.rs` instead of here: PEP 695's `def
# f[T]` syntax and `typing.Protocol` add an unrelated feature dependency to a
# fixture whose subject is the unary operators themselves.
