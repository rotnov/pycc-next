# `sorted`, `list` and `tuple` rebound in `builtins` before the module's
# functions first run: the compiled calls reach the replacements, and no
# list is reused (`tests/fixtures/fresh_list_calls.py` has the builtins'
# own behavior).
import builtins

REAL_SORTED = builtins.sorted
REAL_LIST = builtins.list
REAL_TUPLE = builtins.tuple


def fake_sorted(xs):
    return ["sorted"] + REAL_SORTED(xs)


class FakeList(REAL_LIST):
    def __init__(self, xs=()):
        super().__init__(xs)
        self.append("list")


class FakeTuple(REAL_TUPLE):
    def __new__(cls, xs=()):
        return REAL_TUPLE.__new__(cls, REAL_LIST(xs) + ["tuple"])


builtins.sorted = fake_sorted
builtins.list = FakeList
builtins.tuple = FakeTuple


def calls(xs):
    made = [x * 2 for x in xs]
    return (
        sorted([x for x in xs]),
        list([x for x in xs]),
        tuple([x for x in xs]),
        tuple(sorted(x for x in xs)),
        sorted(made) is made,
        list(made) is made,
        sorted(xs),
        list(xs),
        tuple(xs),
    )


def main():
    for xs in ([3, 1, 2], [], (5, 4)):
        result = calls(xs)
        print(repr(result), [type(part).__name__ for part in result])


main()
