# Deviations from CPython

Behavior that differs from CPython 3.13 on purpose or because it is not implemented yet.
Each entry: what differs, why, and whether it is planned to go away.

| Behavior | pycc-next | CPython | Status |
|---|---|---|---|
| Arithmetic on ints beyond 63 bits (e.g. `x * 2` after `x` exceeds 2**62) | raises `OverflowError: ... bigint-valued int is not supported yet` | arbitrary precision | not implemented yet; to be fixed |
| Integer literals outside the 63-bit range in some positions | compile error `T0051` | accepted | follows from the row above |
| Unsupported but valid Python (e.g. `round`, `lambda`, generators) | compile error `C0001` naming the construct | runs | each is a gap, never silent miscompilation |
