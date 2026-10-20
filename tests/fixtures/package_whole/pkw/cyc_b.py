print("cyc_b start")
from . import cyc_a

B_VALUE = 10


def b_func() -> int:
    return B_VALUE


def uses_a() -> int:
    return cyc_a.A_VALUE + B_VALUE


print("cyc_b end", hasattr(cyc_a, "A_VALUE"))
