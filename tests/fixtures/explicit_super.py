"""`super(C, self)` in the methods of a class CPython creates
(`tests/explicit_super.rs`)."""


def keep(f):
    return f


class Base:
    def __init__(self, n):
        self.n = n

    def describe(self):
        return "base " + str(self.n)


class Child(Base):
    @keep
    def __init__(self, n, label):
        super(Child, self).__init__(n * 2)
        self.label = label

    def describe(self):
        inner = super(Child, self).describe()
        return inner + " child " + self.label

    def skip(self):
        # `super` with another class: the MRO after `Base`.
        return super(Base, self).__repr__ is not None


def main():
    c = Child(3, "x")
    print(c.n, c.label, c.describe(), c.skip())
    try:
        super(Child, 5)
    except TypeError as e:
        print("TypeError", e)


if __name__ == "__main__":
    main()
