# v0.9 accept 1: lambdas bound in a class body -- a method, a static method
# and a class method -- keep the class native; their defaults are evaluated
# where the class body runs, and their names resolve as a lambda's do.
SCALE = 3


class Table:
    key = lambda self, x: (len(x), x)  # noqa: E731
    double = staticmethod(lambda x: x * 2)
    make = classmethod(lambda cls, rows: cls(list(rows)))
    scaled = lambda self, x, k=3: x * k  # noqa: E731

    def __init__(self, rows):
        self.rows = rows

    def ordered(self):
        return sorted(self.rows, key=self.key)

    def first_key(self):
        return self.key(self.rows[0]), self.double(SCALE)


SCALE = 10


def run():
    t = Table.make(["ccc", "a", "bb"])
    return [t.ordered(), t.first_key(), Table.double(4), t.double("ab"), t.scaled(2), t.scaled(2, 5)]


print(run())
