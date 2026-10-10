# Deviations from CPython

Behavior that differs from CPython 3.13 on purpose or because it is not implemented yet.
Each entry: what differs, why, and whether it is planned to go away.

| Behavior | pycc-next | CPython | Status |
|---|---|---|---|
| Arithmetic on ints beyond 63 bits (e.g. `x * 2` after `x` exceeds 2**62) | raises `OverflowError: ... bigint-valued int is not supported yet` | arbitrary precision | not implemented yet; to be fixed |
| Integer literals outside the 63-bit range in some positions | compile error `T0051` | accepted | follows from the row above |
| Unsupported but valid Python (e.g. `sorted`, `lambda`, generators) | compile error `C0001` naming the construct | runs | each is a gap, never silent miscompilation |
| `xs += ys` / `xs *= n` on a list, and the equivalent `xs = xs + ys` | compile error `C0001` | extends/repeats the list in place | the in-place form is not implemented yet; both spellings are refused because they lower to one shape |
| `print`/`str()`/f-string of a list whose elements are class instances | compile error `C0001` | calls each element's `__repr__` | not implemented yet |
| A list element type other than `int`/`float`/`bool`/`str`/class instance/list of those (e.g. `list[set[int]]`) | compile error `T0034` | runs | not implemented yet |
| A bigint-valued `int` stored into a list | aborts (`int boundary does not support bigint-valued values`) | stored | follows from the bigint row above |
