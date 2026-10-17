# Method-level fallback (v0.7 accept 3): one construct pycc cannot compile
# sends only its own method to CPython; the rest of the class compiles.
# Oracle: CPython runs this file unchanged.


class Bag:
    """A bag of numbers."""

    def __init__(self, scale: int) -> None:
        self.scale = scale
        self.items: list[int] = []
        self.count = 0
        self.label = "bag"

    def add(self, value: int) -> None:
        if value > 100:
            raise ValueError("too big: " + str(value))
        self.items.append(value)
        self.count += 1

    def total(self) -> int:
        s = 0
        for v in self.items:
            s += v * self.scale
        return s

    def ranked(self):
        # Falls back alone: a walrus in a short-circuited operand.
        values = [self.count, self.scale, self.total()]
        if values and (first := values[0]) is not None:
            values.append(first - first)
        return sorted(values, reverse=True)[:3]

    def tagged(self, suffix: str) -> str:
        # Falls back alone, attached to the compiled class.
        words = sorted([self.label, suffix], key=lambda s: len(s))
        self.label = words[-1].upper() + words[0]
        return self.label + ":" + str(self.total())

    def checked(self, value):
        try:
            self.add(value)
            return "added"
        except ValueError as e:
            return "refused: " + str(e)

    def __repr__(self) -> str:
        return "Bag(" + str(self.scale) + ", " + str(self.items) + ")"


class Rich:
    """A class CPython creates: its methods are compiled on their own."""

    limit = 100

    def __init__(self, scale: int) -> None:
        self.scale = scale
        self.items = []
        self.__secret = scale * 7

    def add(self, value: int) -> None:
        if value > self.limit:
            raise ValueError("too big: " + str(value))
        self.items.append(value)

    def total(self) -> int:
        s = 0
        for v in self.items:
            s += v * self.scale
        return s

    def ordered(self):
        # Falls back alone: a walrus in a short-circuited operand.
        items = list(self.items)
        if items and (first := items[0]) is not None:
            items.append(first - first)
        return sorted(items, reverse=True)[: len(self.items)]

    def secret(self) -> int:
        return self.__secret + 1

    @property
    def size(self) -> int:
        return len(self.items)

    @staticmethod
    def double(x: int) -> int:
        return 2 * x

    @classmethod
    def of(cls, *values: int):
        bag = cls(1)
        for v in values:
            bag.add(v)
        return bag

    def describe(self, prefix="Rich", **extra):
        parts = [prefix, str(self.size)]
        for key in sorted(extra):
            parts.append(key + "=" + str(extra[key]))
        return " ".join(parts)

    def evens(self):
        for v in self.items:
            if v % 2 == 0:
                yield v

    def __repr__(self) -> str:
        return "Rich(" + str(self.scale) + ", " + str(self.items) + ")"


class Labeled(Rich):
    def __init__(self, scale: int, label: str) -> None:
        super().__init__(scale)
        self.label = label

    def total(self) -> int:
        return super().total() + 1000

    def ordered(self):
        pick = lambda xs: xs[0]
        return [pick(super().ordered())]

    def __repr__(self) -> str:
        return self.label + ":" + super().__repr__()


def logged(f):
    def inner(*args, **kwargs):
        print("call", f.__name__)
        return f(*args, **kwargs)
    inner.__name__ = f.__name__
    return inner


@logged
def summed(n: int, step: int = 1) -> int:
    s = 0
    for i in range(0, n, step):
        s += i
    return s


def keywords(**options):
    return sorted(options.items())


if summed.__name__ == "summed":
    def in_block(x):
        return x * 3
else:
    def in_block(x):
        return x


b = Bag(2)
b.add(3)
b.add(1)
b.add(4)
print(b.total(), b.ranked(), b.tagged("!"), b.label, b.count)
print(b.checked(5), b.checked(500), repr(b))
r = Rich(2)
r.add(3)
r.add(1)
r.add(4)
print(r.total(), r.ordered(), r.secret(), r.size, Rich.double(21))
print(r.describe(), r.describe("R", color="red", n=2), list(r.evens()))
print(repr(r), Rich.of(5, 6).total(), Rich.limit, Rich.__doc__)
lb = Labeled(3, "L")
lb.add(2)
lb.add(9)
print(lb.total(), lb.ordered(), repr(lb), isinstance(lb, Rich))
print(summed(10), summed(10, step=3), summed.__name__)
print(keywords(a=1, b=2), in_block(5))
if __name__ == "__main__":
    r.add(1000)
