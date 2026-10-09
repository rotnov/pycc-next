class Vehicle:
    def __init__(self, wheels: int) -> None:
        self.wheels = wheels

    def describe(self) -> int:
        return self.wheels


class Car(Vehicle):
    def __init__(self, wheels: int, doors: int) -> None:
        super().__init__(wheels)
        self.doors = doors

    def describe(self) -> int:
        return super().describe() + self.doors


class SportsCar(Car):
    def __init__(self) -> None:
        super().__init__(4, 2)

    def describe(self) -> int:
        return super().describe() * 10


v = Vehicle(6)
print(v.describe())

c = Car(4, 4)
print(c.wheels)
print(c.doors)
print(c.describe())

s = SportsCar()
print(s.wheels)
print(s.doors)
print(s.describe())


# #587: `super().<name>` resolves the *class-level* members a CPython
# `super` object actually proxies along the MRO. A `@property` is such a
# member (a descriptor found on a class), so `super().power` calls the base
# class's getter rather than the subclass's override. An instance attribute
# established by `self.<attr> = ...` is not proxied and is rejected at
# compile time, so it cannot appear in a fixture that must match the oracle
# byte for byte.
class Engine:
    def __init__(self, power: int) -> None:
        self._power = power

    @property
    def power(self) -> int:
        return self._power


class TurboEngine(Engine):
    def __init__(self, power: int) -> None:
        super().__init__(power)

    @property
    def power(self) -> int:
        return self._power * 2

    def base_power(self) -> int:
        return super().power


t = TurboEngine(50)
print(t.power)
print(t.base_power())


# #915: a base class *class attribute* is also a class-level member a
# CPython `super` object proxies -- it is a real entry in that class's
# `__dict__` -- so `super().<class attribute>` reads the base class's value
# even when the current class declares its own. Resolution walks one class
# at a time along the MRO after the current class, so an earlier entry's
# class attribute outranks a later entry's `@property` of the same name.
class Limits:
    MAX: int = 10

    def __init__(self) -> None:
        self.used = 0


class TightLimits(Limits):
    MAX: int = 3

    def __init__(self) -> None:
        self.used = 0

    def base_max(self) -> int:
        return super().MAX

    def own_max(self) -> int:
        return self.MAX


tl = TightLimits()
print(tl.base_max())
print(tl.own_max())


class Slow:
    RATE: int = 1

    def __init__(self) -> None:
        self.used = 0


class Fast:
    def __init__(self) -> None:
        self.used = 0

    @property
    def RATE(self) -> int:
        return 100


class Mixed(Slow, Fast):
    def __init__(self) -> None:
        self.used = 0

    def rate(self) -> int:
        return super().RATE


print(Mixed().rate())
