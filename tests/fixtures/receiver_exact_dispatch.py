# #1337 (D-254): an inherited method runs the body its receiver's class
# resolves to. Oracle fixture: pycc must match CPython 3.14.7 byte for byte.
from abc import ABC, abstractmethod
from dataclasses import dataclass

# c02_chain
class A_02:
    def m(self) -> int:
        return 1
    def g(self) -> int:
        return self.m()
class B_02(A_02):
    pass
class C_02(B_02):
    def m(self) -> int:
        return 2
print(C_02().g(), B_02().g())

# c04_prop
class A_04:
    def __init__(self) -> None:
        self.v = 0
    def m(self) -> int:
        return 1
    @property
    def p(self) -> int:
        return self.m()
    @p.setter
    def p(self, x: int) -> None:
        self.v = x + self.m()
class B_04(A_04):
    def m(self) -> int:
        return 2
b04 = B_04()
b04.p = 10
print(b04.p, b04.v)
class P_04:
    Y: int = 10
    @property
    def q(self) -> int:
        return self.Y
class Q_04(P_04):
    Y: int = 20
print(Q_04().q)

# c05_init
class A_05:
    def __init__(self) -> None:
        self.x = 0
        self.x = self.hook()
    def hook(self) -> int:
        return 1
class B_05(A_05):
    def hook(self) -> int:
        return 2
class C_05(A_05):
    def __init__(self) -> None:
        super().__init__()
    def hook(self) -> int:
        return 3
print(B_05().x, C_05().x, A_05().x)

# c06_isinst
class A_06:
    def isb(self) -> bool:
        return isinstance(self, B_06)
class B_06(A_06):
    pass
print(B_06().isb(), A_06().isb())

# c07_cm
class A_07:
    @classmethod
    def k(cls) -> int:
        return 5
    def g(self) -> int:
        return self.k()
    @classmethod
    def base(cls) -> int:
        return 10
    @classmethod
    def make(cls) -> int:
        return cls.base()
class B_07(A_07):
    @classmethod
    def k(cls) -> int:
        return 6
    @classmethod
    def base(cls) -> int:
        return 20
print(B_07().g(), B_07.make(), A_07.make())

# c08_super
class A_08:
    def m(self) -> int:
        return 1
    def g(self) -> int:
        return self.m()
class B_08(A_08):
    def m(self) -> int:
        return 2
    def h(self) -> int:
        return super().m() + self.m()
    def g(self) -> int:
        return super().g()
print(B_08().h(), B_08().g())

# c09_t9
class A_09:
    def m(self) -> int:
        return 1
    def g(self) -> int:
        return self.m()
class B_09(A_09):
    def m(self) -> int:
        return 2
    def g(self) -> int:
        return 10 + super().g()
class C_09(B_09):
    def m(self) -> int:
        return 3
print(C_09().g(), B_09().g(), A_09().g())

# c10_dc
@dataclass
class A_10:
    x: int
    def m(self) -> int:
        return 1
    def g(self) -> int:
        return self.x + self.m()
@dataclass
class B_10(A_10):
    y: int
    def m(self) -> int:
        return 2
print(B_10(5, 0).g())

# c12_abs
class A_12(ABC):
    @abstractmethod
    def m(self) -> int: ...
    def g(self) -> int:
        return self.m()
class B_12(A_12):
    def m(self) -> int:
        return 42
print(B_12().g())

# c13_diamond
class Base_13:
    def f(self) -> int:
        return 1
class L_13(Base_13):
    def f(self) -> int:
        return 10 + super().f()
class R_13(Base_13):
    def f(self) -> int:
        return 100 + super().f()
class C_13(L_13, R_13):
    pass
class D_13(L_13, R_13):
    def f(self) -> int:
        return 1000 + super().f()
print(C_13().f(), D_13().f(), L_13().f())

# c13b_diamond_ty
class Base_13b:
    def f(self) -> int:
        return 1
class L_13b(Base_13b):
    def f(self) -> str:
        return f"{super().f()}!"
class R_13b(Base_13b):
    def f(self) -> str:
        return "r"
class C_13b(L_13b, R_13b):
    pass
print(C_13b().f(), L_13b().f())

# c14_exc
class E_14(Exception):
    def __init__(self, v: int) -> None:
        self.v = v
    def m(self) -> int:
        return 1
    def g(self) -> int:
        return self.m()
class F_14(E_14):
    def m(self) -> int:
        return 2
e14 = F_14(0)
print(e14.g())

# c16a_helper
def _h16(k: int) -> int:
    return k + 1
class A_16a:
    K: int = 1
    def g(self) -> int:
        return _h16(self.K)
class B_16a(A_16a):
    K: int = 2
print(B_16a().g(), A_16a().g())

# c17_e1
class A_17:
    def m(self) -> int:
        return 1
    def me(self) -> A_17:
        return self
class B_17(A_17):
    def extra(self) -> int:
        return 5
print(B_17().me().m())

# c18_e2
class A_18:
    def __init__(self) -> None:
        self.n = 1
    def add(self, o: A_18) -> int:
        return self.n + o.n
    def twice(self) -> int:
        return self.add(self)
class B_18(A_18):
    def extra(self) -> int:
        return 5
print(B_18().twice())

# c19_setter
class A_19:
    def __init__(self) -> None:
        self.v = 0
    def m(self) -> int:
        return 1
    @property
    def p(self) -> int:
        return self.m()
    @p.setter
    def p(self, x: int) -> None:
        self.v = x + self.m()
class B_19(A_19):
    def m(self) -> int:
        return 2
b19 = B_19()
b19.p = 10
print(b19.p, b19.v)

# c20_g1
class A_20:
    def __init__(self) -> None:
        self.xs = []
        self.xs.append(self.m())
    def m(self) -> int:
        return 1
class B_20(A_20):
    def m(self) -> int:
        return 101
print(B_20().xs[0], A_20().xs[0])
