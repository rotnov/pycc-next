# PEP 557 -- Dataclasses (pycc fixture)
#
# This fixture exercises pycc's dataclass implementation:
#   - `@dataclass` with required annotated fields
#   - Auto-generated `__init__` (constructor)
#   - Auto-generated `__eq__` (equality) and `__ne__` (inequality)
#   - Auto-generated `__repr__` (string representation)
#   - Dataclass inheritance (field merging: parent fields first)
#   - `ClassVar` members, which PEP 557 excludes from the synthesized
#     `__init__`, `__eq__` and `__repr__` (#913)
#
# Field defaults (`field(default=...)`, `field(default_factory=...)`) are
# not supported in this version -- only required fields are supported.
#
# The import is required by CPython, which evaluates decorators eagerly
# (unlike annotations, which PEP 649/749 defers) and would otherwise
# raise `NameError` on the oracle run. pycc itself recognizes the bare
# name `dataclass` without the import, matching its
# `Final`/`Annotated`/`Enum`/`abstractmethod` precedent, so the import
# only has to resolve -- see `pycc_std`'s `dataclasses` module (#579).
#
# `ClassVar` is imported for the same reason: CPython's `@dataclass` has to
# resolve the annotation to classify the name as a non-field.
from dataclasses import dataclass
from typing import ClassVar


@dataclass
class Point:
    x: int
    y: int

# Auto-generated constructor: Point(x, y)
p1 = Point(1, 2)
p2 = Point(1, 2)
p3 = Point(3, 4)

# Auto-generated __eq__ and __ne__
print(p1 == p2)
print(p1 != p3)
print(p1 == p3)

# Auto-generated __repr__
print(p1)
print(p3)

# Dataclass inheritance: parent fields come first
@dataclass
class NamedPoint(Point):
    name: int

np = NamedPoint(10, 20, 30)
print(np.x)
print(np.y)
print(np.name)
print(np)

# Equality with inherited fields
np2 = NamedPoint(10, 20, 30)
np3 = NamedPoint(10, 20, 99)
print(np == np2)
print(np != np3)

# PEP 557 (#913): a `ClassVar` member is *not* a field. It is excluded from
# the synthesized `__init__` (so `Bounded(5)` still takes exactly one
# argument), from `__repr__`, and from `__eq__` -- while still being readable
# through the class name and through an instance.
@dataclass
class Bounded(Point):
    LIMIT: ClassVar[int] = 100
    KIND: ClassVar[str] = "bounded"


b1 = Bounded(1, 2)
b2 = Bounded(1, 2)
print(b1.LIMIT)
print(Bounded.LIMIT)
print(b1.KIND)
print(b1)
print(b1 == b2)
print(b1 != Bounded(3, 4))
