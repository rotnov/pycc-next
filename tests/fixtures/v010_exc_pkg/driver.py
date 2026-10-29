"""Exercises package `pke` as CPython sees it, against its sources and against
the package built whole (`tests/v010_exceptions.rs`); the outputs must match.
argv[1] is the directory holding `pke`."""
import gc
import pickle
import sys

sys.path.insert(0, sys.argv[1])

import pke  # noqa: E402
from pke import common, errors  # noqa: E402

compiled = pke.__file__.endswith((".so", ".pyd"))


def mro(cls):
    """The MRO without the layout templates (`docs/DEVIATIONS.md`)."""
    return [c.__name__ for c in cls.__mro__ if c.__module__ != "pycc"]


print(pke.lookup({"a": 1}, "a"))
try:
    pke.lookup({}, "b")
except pke.PkgError as e:
    print(type(e).__name__, str(e), e.key, e.details, e.args, isinstance(e, Exception))
print(mro(pke.KeyMissing), mro(pke.Sub))
print(pke.catch(4))
try:
    pke.raise_sub(5)
except common.Owned as e:
    print(type(e).__name__, e.describe(), e.code, e.where, e.extra, e.args, str(e))
    print(common.code_of(e))
s = pke.Sub(6, "py", "y")
print(s.describe(), s.code, s.extra, isinstance(s, common.Owned), common.code_of(s))
k = pke.KeyMissing("q", "here")
k.key = "r"
print(str(k), k.args, repr(k))
try:
    pickle.loads(pickle.dumps(k))
except TypeError as err:
    print("TypeError", err)


class PySub(errors.KeyMissing):
    def __init__(self, key):
        super().__init__(key, "py")


one, two = common.Clash(1), errors.Clash("b", "c")
print(one.a, two.b, two.c, type(one) is not type(two), hasattr(one, "b"), hasattr(two, "a"))
p = PySub("z")
print(str(p), mro(PySub))
try:
    raise p
except pke.PkgError as caught:
    print("caught", caught is p, caught.key)

if compiled:
    import ctypes

    lib = ctypes.CDLL(pke.__file__)

    def live():
        gc.collect()
        return (lib.pycc_rt_str_live_objects(), lib.pycc_rt_instance_live_objects())

    def work():
        try:
            pke.lookup({}, "x")
        except pke.KeyMissing as e:
            str(e)
        pke.catch(3)
        pke.Sub(1, "w", "e").describe()

    for _ in range(50):
        work()
    before = live()
    for _ in range(500):
        work()
    print("leaks", "none" if live() == before else f"{before} -> {live()}")
else:
    print("leaks", "none")
