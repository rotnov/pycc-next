"""Typed numpy arrays whose object changes under compiled code.

Built as an `--ext` module by `tests/numpy_typed_arrays.rs`
(`an_array_changed_by_cpython_code_is_rechecked`): every case below once
read through a stale or never-checked header.
"""
import numpy as np
from numpy.typing import NDArray


def after_hook(a: NDArray[np.float64], hook: object) -> float:
    """Reads `a` before and after CPython code that may change it in place."""
    first = a[0]
    hook(a)
    return first + a[3]


def element_as_array(a: np.ndarray[tuple[int], np.dtype[np.float64]]) -> float:
    """Binds an element to a name typed as an array."""
    x: NDArray[np.float64] = np.zeros(2)
    for x in a:
        pass
    return x[1]


def row_as_matrix(g: np.ndarray[tuple[int, int], np.dtype[np.float64]]) -> float:
    """Binds a row to a name typed as a two-dimensional array."""
    r: NDArray[np.float64] = np.zeros((2, 2))
    t = r[1, 1]
    for r in g:
        t += r[1, 1]
    return t


def index_after_hook(a: NDArray[np.float64], hook: object, i: int) -> int:
    """Runs CPython code that may change `a`, then answers `i`."""
    hook(a)
    return i


def read_with_hooked_index(a: NDArray[np.float64], hook: object) -> float:
    """Reads `a` at an index whose evaluation changes `a`."""
    first = a[0]
    return first + a[index_after_hook(a, hook, 3)]


def store_with_hooked_index(a: NDArray[np.float64], hook: object) -> float:
    """Stores into `a` at an index whose evaluation changes `a`."""
    first = a[0]
    a[index_after_hook(a, hook, 3)] = 7.0
    return first + a[3]


def value_after_hook(a: NDArray[np.float64], hook: object, v: float) -> float:
    """Runs CPython code that may change `a`, then answers `v`."""
    hook(a)
    return v


def store_hooked_value(a: NDArray[np.float64], hook: object) -> float:
    """Stores into `a` a value whose evaluation changes `a`."""
    first = a[0]
    a[3] = value_after_hook(a, hook, 7.0)
    return first + a[3]
