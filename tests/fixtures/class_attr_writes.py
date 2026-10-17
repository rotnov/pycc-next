class Base:
    count: int = 0
    label = "base"

    def __init__(self, v: int, tag: str) -> None:
        self.v = v
        self.tag = tag
        Base.count += 1

    def score(self) -> int:
        return self.v * 3

    def describe(self) -> str:
        return f"{self.tag}:{self.score()}"


class Derived(Base):
    made: int = 0

    def __init__(self, v: int, tag: str, extra: int) -> None:
        super().__init__(v, tag)
        self.extra = extra

    def score(self) -> int:
        return super().score() + self.extra

    @classmethod
    def make(cls, v: int) -> "Derived":
        Derived.made = Derived.made + 1
        return cls(v, "made", v + 1)


class Leaf(Derived):
    def score(self) -> int:
        return -self.v


class Plain(Base):
    def twin(self) -> "Plain":
        return type(self)(self.v + 1, self.tag + "'")


class PlainLeaf(Plain):
    def score(self) -> int:
        return 0


def relabel(text: str) -> None:
    if text:
        for _ in range(2):
            Base.label = Base.label + text
    else:
        Base.label = "empty"


print(Derived.make(3).describe(), Base.count, Derived.made)
d = Derived(1, "x", 2)
print(d.make(4).describe(), Base.count)
leaf = Leaf.make(5)
print(leaf.describe(), leaf.extra, Derived.made)
print(Plain(7, "p").twin().describe(), PlainLeaf(1, "q").twin().describe(), Base.count)
relabel("!")
print(Base.label)
relabel("")
Base.count += 100
print(Base.label, Base.count, f"{Base.count:>6}|{Derived.made}")
words = "a b a c a"
print(words.count("a"))
