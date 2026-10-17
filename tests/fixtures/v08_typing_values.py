"""v0.8 accept 4: a non-strict module using modeled `typing` forms as values
(`V = Union[A, B]`, `R = dict[str, Any]`) gets their CPython objects, while
its annotations and `cast` still compile."""

from typing import Any, Dict, Optional, Union, cast

V = Union[int, str]
R = dict[str, Any]
D = Dict[str, int]


def describe(x: Optional[int]) -> str:
    return "none" if x is None else str(cast(int, x) + 1)


print(V, R, D, Optional[int])
print(describe(None), describe(4), Union[int, str] == V)
print(V.__args__[3])
