# PEP 3119 (#380): ABC and @abstractmethod.
#
# Exercises: an ABC base class with an @abstractmethod, a concrete
# subclass that overrides the abstract method, a second concrete
# subclass with constructor parameters, super().__init__() chaining,
# and instantiation + method calls on the concrete subclasses.
#
# Both pycc and CPython 3.14.7 produce identical stdout: the ABC and
# @abstractmethod are compile-time-only markers in pycc and runtime
# enforcement mechanisms in CPython, but neither emits anything for the
# class definitions themselves — only the concrete method calls produce
# observable output, and those match byte-for-byte.
from abc import ABC, abstractmethod


class Shape(ABC):
    def __init__(self) -> None:
        self.x = 0

    @abstractmethod
    def area(self) -> int:
        ...


class Circle(Shape):
    def __init__(self) -> None:
        super().__init__()

    def area(self) -> int:
        return 42


class Rectangle(Shape):
    def __init__(self, w: int, h: int) -> None:
        super().__init__()
        self.w = w
        self.h = h

    def area(self) -> int:
        return self.w * self.h


c = Circle()
print(c.area())

r = Rectangle(3, 4)
print(r.area())
