# PEP 572 -- Assignment expressions (`:=`, the "walrus operator"), issue #774.
#
# Scope proven here: a walrus in an `if` condition with the bound name read
# afterward, a walrus in a `while` condition, a walrus as a bare expression
# statement, and a walrus nested inside a larger expression (a function call
# argument and an arithmetic operand). Every walrus value below is `int` --
# T0050 (this PR's own deliberate scope-cut, see docs/PYTHON_STANDARDS.md and
# the conformance-breadth manifest) restricts a walrus value to `int`/
# `float`/`bool`/`None` (including `Optional` of those), since extending
# every reference-counted-type codegen classifier for a `NamedExpr`-yielded
# value is out of scope for this PR. A walrus target nested inside a
# comprehension is also out of scope (CPython's own comprehension-scope-skip
# semantics, D-177 `core` gap) and is not exercised here.

# Walrus in an `if` condition, with the bound name read both inside the
# branch and afterward (module scope).
if (n := 7) > 5:
    print(n)
print(n)

# Walrus in a `while` condition -- classic "read until sentinel" shape.
total = 0
i = 5


def decrement(x: int) -> int:
    return x - 1


while (v := decrement(i)) > 0:
    total = total + v
    i = v
print(total)
print(v)

# Walrus as a bare expression statement.
(m := 42)
print(m)

# Walrus nested inside a larger expression: a function call argument and an
# arithmetic operand.
def double(x: int) -> int:
    return x * 2


print(double(k := 9))
print(k)
print((a := 3) + (b := a + 1))
print(a)
print(b)


# Function-scope walrus: the bound name is visible to later statements in
# the same function body, exactly as module scope above.
def use_walrus_in_function() -> int:
    if (local_n := 100) > 50:
        result = local_n + 1
    else:
        result = 0
    return result + local_n


print(use_walrus_in_function())
