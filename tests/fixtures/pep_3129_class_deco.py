# PEP 3129 -- Class Decorators (pycc fixture)
#
# This fixture exercises pycc's narrow class-decorator support:
#   - `@dataclass` (PEP 557) is recognized and triggers auto-generated
#     `__init__`, `__eq__`, and `__repr__`.
#   - Any other class decorator is rejected with C0001.
#   - `@dataclass(frozen=True)` and other option-bearing forms are
#     rejected with C0001.
#
# Only the `@dataclass` form is executable in this fixture; the rejected
# forms are documented below as comments.
#
# The import is required by CPython, which evaluates decorators eagerly;
# pycc recognizes the bare name `dataclass` without it (#579).
#
# `@dataclass_transform()` (PEP 681) is deliberately *not* exercised here.
# pycc treats it as equivalent to `@dataclass` (docs/ROADMAP.md), while
# CPython's own `typing.dataclass_transform` is a pure marker that
# synthesizes nothing -- so a class built that way diverges from the
# oracle by construction and cannot appear in a byte-for-byte conformance
# fixture at all. That divergence is recorded in D-196 (see also #749);
# PEP 681 keeps its own matrix row, marked "no fixture" for that reason.
from dataclasses import dataclass


@dataclass
class Point:
    x: int
    y: int

p = Point(3, 4)
print(p.x)
print(p.y)
print(p)

# Rejected forms (not executed -- documented for conformance reference):
#
#   @dataclass(frozen=True)
#   class Frozen:
#       x: int
#
#   @some_other_decorator
#   class Other:
#       x: int
#
# Both produce C0001 ("not supported yet") in pycc.
