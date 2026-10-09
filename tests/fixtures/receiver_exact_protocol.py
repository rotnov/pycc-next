# #1337 (D-254): an inherited method runs the body its receiver's class
# resolves to. Oracle fixture: pycc must match CPython 3.14.7 byte for byte.
# Kept apart from the `super()` fixture: a module with a Protocol-typed
# parameter and a `super()` call is refused today (pre-existing).
from typing import Protocol

# c11_proto
class P_11(Protocol):
    def g(self) -> int: ...
class A_11:
    def m(self) -> int:
        return 1
    def g(self) -> int:
        return self.m()
class B_11(A_11):
    def m(self) -> int:
        return 2
def call_g(x: P_11) -> int:
    return x.g()
print(call_g(B_11()), call_g(A_11()))

# c11b_protoparam
class P_11b(Protocol):
    def v(self) -> int: ...
class X_11b:
    def v(self) -> int:
        return 100
class A_11b:
    def m(self) -> int:
        return 1
    def use(self, p: P_11b) -> int:
        return p.v() + self.m()
class B_11b(A_11b):
    def m(self) -> int:
        return 2
print(B_11b().use(X_11b()), A_11b().use(X_11b()))
