# A `--non-strict` statement running in CPython (it calls `math.trunc` of a
# float the strict checker types differently) whose f-string uses the `=`
# debug specifier on a module global the fallback reads from its namespace
# dictionary still prints the field as written (fuzz seed 202, case 324).
import math

b0_v0: int = math.trunc((-835.536))
b0_v1: float = (0.1 * 3)
b0_v2: str = '{} and {}'.format(b0_v0, 'abc')
print(f'{b0_v0=} {b0_v1=:.2f} {b0_v2!r:>10}')
print(f'{ b0_v0 = } {b0_v0=!s} {b0_v0 + 1=} {b0_v2=:>14} {b0_v0=}{{}}')
