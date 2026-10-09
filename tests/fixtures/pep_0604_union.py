# PEP 604 -- Allow writing union types as X | Y (D-197, #763, Part 1 of #747).
#
# Scope proven here: `Optional[int]` (`int | None`, either operand order),
# `is`/`is not` as a presence test against `None`, and both a module-level
# and a function-local `int | None` variable. Flow-sensitive narrowing
# (reading the wrapped payload back as a plain `int` inside a `not None`
# branch) is now in scope too (D-205, #769, Part 2 of #747), restricted to
# a top-level `if name is None:` / `if name is not None:` test -- the
# rungs below exercise both polarities, the early-return-narrows-the-
# continuation shape, kill-on-reassignment inside a narrowed branch, and a
# function-local narrowed read, alongside the original presence-only
# checks this fixture already proved. `Optional[T]`'s inner type `T` is
# further widened from `int`-only to also accept `float` and `bool` (#809,
# Part 3 of #747) -- the section below repeats the same presence, ordering,
# truthiness, function-return, and function-local shapes for `float | None`
# and `bool | None`. `Optional[str]` and general (non-`Optional`) unions
# such as `int | str` remain out of scope.

present: int | None = 5
absent: int | None = None
print(present is None)
print(present is not None)
print(absent is None)
print(absent is not None)

# D-205, #769: `is not None` narrows the body -- `present + 1` reads
# `present` as a plain `int`, not `Optional[int]`.
if present is not None:
    print(present + 1)

# D-205, #769: `is None` narrows the `orelse` -- the mirror polarity.
if present is None:
    pass
else:
    print(present * 2)

# D-205, #769: a reassignment inside the narrowed branch kills the
# narrowing for every read after it, within the same branch -- `present`
# is written back as a bare `int` here, then the branch's own remaining
# reads still work (through the ordinary `Optional`-widening `Assign`
# path, not narrowing).
if present is not None:
    present = present + 100
    print(present is None)
    print(present is not None)

# Reversed operand order: `None | T` must parse identically to `T | None`,
# and narrows exactly the same way.
reversed_order: None | int = 7
print(reversed_order is None)
if reversed_order is not None:
    print(reversed_order + 1)


def maybe_double(x: int, present: bool) -> int | None:
    if present:
        return x * 2
    return None


doubled = maybe_double(21, True)
skipped = maybe_double(21, False)
print(doubled is None)
print(skipped is None)


def describe(x: int | None) -> None:
    # D-205, #769: the early-return continuation shape -- a guard clause
    # whose body definitely terminates (ends in a bare `return`) narrows
    # every read after it in the same statement sequence, with no `else`
    # needed at all.
    if x is None:
        print("absent")
        return
    print(x + 1)


describe(doubled)
describe(skipped)


def local_optional() -> None:
    y: int | None = 9
    print(y is not None)
    z: int | None = None
    print(z is not None)
    # D-205, #769: narrowing works identically inside a function body.
    if y is not None:
        print(y + 1)


local_optional()

# D-199, #769: the narrowed read must also work when the payload is a
# heap-allocated bigint, not just a smallint -- assigning the narrowed
# read into a second binding forces codegen's duplicate-reference retain
# path (`retain_if_int_duplicate`) to run on an `OptionalUnwrap` source,
# not just the plain-int sources it already covered. 4611686018427387904
# is 2**62, the same promoted-to-bigint boundary value used by the
# bigint fixtures (e.g. `tests/fixtures/bigint_range.py`).
big: int | None = 4611686018427387904
if big is not None:
    duplicated = big
    print(big + 1)
    print(duplicated)

# #809 (Part 3 of #747): `Optional[T]` widened from `T == int` only to
# also accept `T == float` and `T == bool` -- the same presence checks,
# truthiness, and narrowed-read shapes proven for `int | None` above,
# now proven for `float | None` and `bool | None` too.

present_float: float | None = 5.5
absent_float: float | None = None
print(present_float is None)
print(present_float is not None)
print(absent_float is None)
print(absent_float is not None)

if present_float is not None:
    print(present_float + 1.0)

if present_float is None:
    pass
else:
    print(present_float * 2.0)

reversed_order_float: None | float = 2.5
print(reversed_order_float is None)
if reversed_order_float is not None:
    print(reversed_order_float + 1.0)

present_bool: bool | None = True
absent_bool: bool | None = None
print(present_bool is None)
print(present_bool is not None)
print(absent_bool is None)
print(absent_bool is not None)

if present_bool is not None:
    print(present_bool)

reversed_order_bool: None | bool = False
print(reversed_order_bool is None)
if reversed_order_bool is not None:
    print(reversed_order_bool)

# Truthiness: `bool(x)` for `x: float | None`/`bool | None` is `False`
# only for `None` or a present falsy payload (`0.0`/`False`).
falsy_float: float | None = 0.0
if falsy_float:
    print("truthy")
else:
    print("falsy")

falsy_bool: bool | None = False
if falsy_bool:
    print("truthy")
else:
    print("falsy")


def maybe_float(x: float, present: bool) -> float | None:
    if present:
        return x * 2.0
    return None


def maybe_bool(x: bool, present: bool) -> bool | None:
    if present:
        return x
    return None


doubled_float = maybe_float(21.0, True)
skipped_float = maybe_float(21.0, False)
print(doubled_float is None)
print(skipped_float is None)

kept_bool = maybe_bool(True, True)
skipped_bool = maybe_bool(True, False)
print(kept_bool is None)
print(skipped_bool is None)


def local_optional_float_bool() -> None:
    y: float | None = 9.5
    print(y is not None)
    z: float | None = None
    print(z is not None)
    if y is not None:
        print(y + 1.0)
    w: bool | None = True
    print(w is not None)
    if w is not None:
        print(w)


local_optional_float_bool()
