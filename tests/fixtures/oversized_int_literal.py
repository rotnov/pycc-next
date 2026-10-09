"""#148/D-178: `int` literals outside D-061's tagged 63-bit range.

Every value below is written as a decimal literal, never as an expression
(`2**62` would be routed through `int_pow` -- the pre-existing arithmetic
promotion path -- rather than through literal materialization).

Restricted to the operations that a bigint-valued `int` supports today:
printing, truthiness, `+`/`-`, f-strings, assignment, and function
argument/return round-trips. Multiplication, floor division, modulo, power,
true division, and comparison on a bigint operand are still open gaps.
"""

from enum import Enum


class Discriminant(Enum):
    SMALL = 1
    HUGE = 4611686018427387904


def identity(value: int) -> int:
    return value


smallest_promoted: int = 4611686018427387904
largest: int = 9223372036854775807
smallest_negative_promoted: int = -4611686018427387905
most_negative: int = -9223372036854775808
boundary: int = 4611686018427387903

print(smallest_promoted)
print(largest)
print(smallest_negative_promoted)
print(most_negative)
print(boundary)
print(4611686018427387904)
print(-9223372036854775808)

print(smallest_promoted + 1)
print(smallest_promoted - 1)
print(largest + 1)
print(most_negative - 1)
print(boundary + 1)

print(f"promoted={smallest_promoted} negative={most_negative}")
print(identity(4611686018427387904))

if smallest_promoted:
    print("nonzero promoted literal is truthy")
if most_negative:
    print("most negative literal is truthy")

print(Discriminant.SMALL.value)
print(Discriminant.HUGE.value)
print(f"discriminant={Discriminant.HUGE.value}")
