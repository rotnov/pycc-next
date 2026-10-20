print("cyc_a start")
from . import cyc_b

A_VALUE = 1


def a_func() -> int:
    return cyc_b.b_func() + A_VALUE


print("cyc_a end")
