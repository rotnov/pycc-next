# #604 (Part 3 of #573): `not x` and `~x`.
#
# Neither operator has a literal-folding arm the way `-`/`+` do for numeric
# literals (#602): `not True` and `~5` are not part of Python's literal
# grammar, so both always lower through the generic `HirExpr::UnaryOp` path
# regardless of the operand's own shape.
#
# `not x` is defined by truthiness (CPython's `PyObject_IsTrue`), so this
# fixture spans every operand type this compiler computes a truth value for:
# `bool`, `int` (including the zero/nonzero/negative/bigint cases), `float`
# (including the signed-zero case that makes `bool(-0.0)` false), `str`
# (including the empty string), and `Optional` (both present and absent).
#
# `~x` is `int -> int` only (`bool` included, as `int`'s subtype): `~x == -x -
# 1`, decomposed at the MIR level into two chained `int_sub`s, so it inherits
# arbitrary-precision promotion "for free" the same way `-x` does; this
# fixture spans the smallint/bigint boundary the same way
# `unary_general_operand.py` does for plain negation.

# `not` over `bool`.
print(not True)
print(not False)

# `not` over `int`.
x = 0
print(not x)
y = 5
print(not y)
z = -3
print(not z)

# `not` over a bigint.
big = 9000000000000000000
print(not big)

# `not` over `float`, including the signed-zero and infinity edge cases.
print(not 0.0)
print(not -0.0)
print(not 2.5)
inf = 1e400
print(not inf)
print(not -inf)

# `not` over `str`.
print(not "")
print(not "ab")

# `not` over `Optional`.
def maybe_int(present: bool) -> int | None:
    if present:
        return 5
    return None


a = maybe_int(True)
print(not a)
b = maybe_int(False)
print(not b)

zero_present: int | None = 0
print(not zero_present)

# Composition: double negation and use inside `if`.
print(not not y)
if not x:
    print("empty")
else:
    print("full")


# `~` over `int`.
print(~0)
print(~5)
print(~(-5))
print(~(~7))

# `~` over `bool`.
t = True
f = False
print(~t)
print(~f)

# `~` at the tagged-smallint boundary and past it into bigint territory --
# `~x == -x - 1` must inherit `int_sub`'s arbitrary-precision promotion.
smallint_boundary = 4611686018427387903
print(~smallint_boundary)
print(~big)


def inverted(n: int) -> int:
    return ~n


print(inverted(3))
print(inverted(-3))
