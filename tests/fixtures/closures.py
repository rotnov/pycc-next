from typing import Callable


def compose(f: Callable[[int], int], g: Callable[[int], int]) -> Callable[[int], int]:
    def h(x: int) -> int:
        return f(g(x))
    return h


def make_greeter(greeting: str) -> Callable[[str], str]:
    suffix = "!"

    def greet(name: str) -> str:
        return greeting + ", " + name + suffix
    return greet


def make_acc(items: list[int]) -> Callable[[int], int]:
    def add(x: int) -> int:
        items.append(x)
        return len(items)
    return add


def make_counter(start: int) -> Callable[[], int]:
    count = start

    def inc() -> int:
        nonlocal count
        count += 1
        return count
    return inc


def twice(x: int) -> int:
    return 2 * x


def inc1(x: int) -> int:
    return x + 1


def apply_all(fs: list[int], f: Callable[[int], int]) -> list[int]:
    return [f(v) for v in fs]


def safe_div(d: int) -> Callable[[int], int]:
    def div(x: int) -> int:
        return x // d
    return div


def big_scaler(k: int) -> Callable[[int], int]:
    def scale(x: int) -> int:
        return x * k
    return scale


class Box:
    def __init__(self, v: int) -> None:
        self.v = v

    def adder(self) -> Callable[[int], int]:
        def add(x: int) -> int:
            return x + self.v
        return add


OP = twice


def use_global(x: int) -> int:
    return OP(x)


h = compose(twice, inc1)
print(h(5), compose(inc1, twice)(5))
g = make_greeter("Hello")
print(g("world"), make_greeter("Hi")("there"))
acc_list: list[int] = []
acc = make_acc(acc_list)
acc(1)
acc(2)
print(acc(3), acc_list)
c1 = make_counter(10)
c2 = make_counter(100)
print(c1(), c1(), c2(), c1())
print(apply_all([1, 2, 3], twice), apply_all([4], safe_div(2)))
f = twice
f = inc1
print(f(1), use_global(21))
dz = safe_div(0)
try:
    print(dz(5))
except ZeroDivisionError as e:
    print("caught", e)
print(safe_div(3)(10), big_scaler(2 ** 70)(3))
b = Box(7)
add7 = b.adder()
b.v = 100
print(add7(1))
