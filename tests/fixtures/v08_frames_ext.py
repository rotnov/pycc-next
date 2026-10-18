"""v0.8 accept 3: compiled functions run in real frames (an `--ext` module).

`tests/v08_frames.rs` builds this module with `--ext --non-strict` and runs
`v08_frames_driver.py` against it and against this source; both runs must
print the same text.
"""
import abc
import inspect
import sys
import traceback
import warnings
from copy import deepcopy

VERSION = "2.13.5"
LIMIT = 3


def depth_name(n):
    f = sys._getframe(n)
    return f.f_code.co_name


def caller_info(n):
    f = sys._getframe(n)
    return (f.f_code.co_name, f.f_code.co_qualname, f.f_lineno, f.f_back.f_code.co_name,
            f.f_globals["__name__"], f.f_locals is f.f_globals)


def outer(n):
    return depth_name(n)


def through_python(callback, n):
    return callback(n)


def boom(x):
    return 10 // x


def call_boom(x):
    y = x + 1
    return boom(x) + y


def reraises(xs, i):
    try:
        return xs[i]
    except IndexError:
        raise ValueError("no item " + str(i) + " of " + str(len(xs)))


def handled_traceback(x):
    try:
        boom(x)
    except ZeroDivisionError as e:
        stack = traceback.TracebackException.from_exception(e).stack
        return [(frame.name, frame.lineno) for frame in stack]
    return []


def calls_back(callback):
    total = 0
    for i in range(3):
        total += callback(i)
    return total


def warns(n):
    warnings.warn("careful " + str(n), stacklevel=2)
    return n


def stack_names():
    return [frame.function for frame in inspect.stack()[:3]]


def version_short():
    return ".".join(VERSION.split(".")[:2])


def limit_plus(n):
    return LIMIT + n


def copies(x):
    return deepcopy(x)


def _private_twice(n: int) -> int:
    return n * 2


def _caller_name(depth: int = 2) -> str:
    return sys._getframe(depth).f_code.co_name


def defaulted(n: int = 1, *, scale: int = 1) -> int:
    if n < 0:
        raise ValueError("negative " + str(n))
    return len(_caller_name()) * scale + n


class Box:
    def __init__(self, v):
        self.v = v

    def where(self):
        return sys._getframe(0).f_code.co_name, sys._getframe(1).f_code.co_name

    def fail(self):
        return self.v // 0


class Lifted(abc.ABC):
    """CPython creates this class (an `abc.ABC` base); its methods compile."""

    def __init__(self, v=0):
        super().__init__()
        self.v = v

    def where(self, n=1):
        return sys._getframe(n).f_code.co_name

    def fails(self, x):
        return self.v // x

    def bind(self, a, b=2, *rest, c=3, **kw):
        return (a, b, rest, c, sorted(kw.items()))

    @property
    def prop(self):
        return sys._getframe(1).f_code.co_name

    @classmethod
    def make(cls, v):
        return cls(v=v).v


def frame_kind(n):
    f = sys._getframe(n)
    return f.f_code.co_name, f.f_globals["__name__"], f.f_locals is f.f_globals


AT_IMPORT = frame_kind(1)
