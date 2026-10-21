# v0.9: Python code using `v09_generic`'s classes as CPython classes
# (`tests/v09_native_state.rs` imports the fixture as `m`, compiled or not).
import typing

from m import Acc, Base, Box, Derived, Pair, T

print(Box.__parameters__, Box.__orig_bases__, Pair.__parameters__, Acc.__parameters__)
print(Box[int], Box[int](3).get(), typing.get_args(Box[str]), typing.Generic in Box.__mro__)
print(issubclass(Derived, Base), isinstance(Derived(1), Base), Derived(2).twice())
print(Derived.__name__, Derived.__qualname__, Derived.__module__, Box.__module__)
print([c.__name__ for c in Derived.__mro__ if c in (Derived, Base, object)])
print(T in Box.__parameters__, Pair("a", 2).swap().key)
try:
    Box[int, str]
except TypeError as e:
    print("TypeError", e)
