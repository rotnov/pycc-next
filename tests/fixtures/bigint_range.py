"""#147/D-179: `range()` over values outside D-061's tagged 63-bit range.

Every bound is written as a decimal literal rather than an expression
(`2**62` would route through `int_pow`, a still-open bigint gap) and every
loop is deliberately short: the point is that the *representation* crosses
the smallint boundary, not that the trip count is large.

Restricted to the shapes pycc and CPython agree on byte for byte. A bigint
*zero* step diverges (CPython raises `ValueError`, pycc takes D-072's exit
`101` boundary) and is pinned by `tests/issue_147_bigint_range.rs` instead.
"""


def total(start: int, stop: int) -> int:
    acc: int = 0
    for value in range(start, stop):
        acc = acc + 1
    return acc


# A promoted start paired with a bounded promoted stop.
for i in range(4611686018427387904, 4611686018427387907):
    print(i)

# A promoted start with a smallint-representable ascending step.
for i in range(4611686018427387904, 4611686018427387910, 3):
    print(i)

# A negative step walking back across the boundary.
for i in range(4611686018427387906, 4611686018427387903, -1):
    print(i)

# A negative bigint step.
for i in range(0, -9223372036854775807, -4611686018427387904):
    print(i)

# The induction variable promotes mid-loop: 4611686018427387904 is the first
# value that no longer fits the tagged encoding, and it is past `stop`.
for i in range(4611686018427387900, 4611686018427387903, 2):
    print(i)

# An empty bigint range runs its body zero times.
for i in range(4611686018427387904, 4611686018427387904):
    print("unreachable")
print("empty range completed")

# Negative promoted bounds.
for i in range(-4611686018427387906, -4611686018427387903):
    print(i)

# The loop target keeps the last value the loop bound.
last: int = 0
for last in range(4611686018427387904, 4611686018427387906):
    print(last)
print(last)

# Reassigning the target inside the body does not corrupt the next iteration.
for i in range(4611686018427387904, 4611686018427387907):
    print(i)
    i = 5

print(total(4611686018427387904, 4611686018427387908))

# D-141's bool-identity contract is unchanged: `range` consumes the numeric
# value and produces ordinary integer objects.
for i in range(True):
    print(i)
for i in range(False, True):
    print(i)

# Comprehensions use the same three emitters and the same normalizer.
listcomp = [0 for i in range(4611686018427387904, 4611686018427387907)]
setcomp = {0 for i in range(4611686018427387904, 4611686018427387907)}
dictcomp = {"k": 0 for i in range(4611686018427387904, 4611686018427387907)}
print(len(listcomp))
print(len(setcomp))
print(len(dictcomp))
