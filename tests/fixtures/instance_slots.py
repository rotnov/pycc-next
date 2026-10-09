# `__slots__` in a class body (#1368): every admitted spelling, slotted
# inheritance in both directions, the empty-slots diamond and a slotted
# exception subclass. Only shapes whose CPython behaviour is the same on
# 3.9 through 3.14 appear here.


class ParseConf:
    __slots__ = 'parse_table', 'start', 'states'

    parse_table: int
    start: int
    states: int

    def __init__(self, parse_table: int, start: int) -> None:
        self.parse_table = parse_table
        self.start = start
        self.states = start


class Single:
    __slots__ = 'value'

    def __init__(self, value: int) -> None:
        self.value = value


class Parenthesised:
    __slots__ = ('a', 'b')

    def __init__(self) -> None:
        self.a = 1
        self.b = 2


class AsList:
    __slots__ = ['a']

    def __init__(self) -> None:
        self.a = 'listed'


class Empty:
    __slots__ = ()

    def total(self) -> int:
        return 3


class NeverAssigned:
    __slots__ = ('a', 'unused')

    def __init__(self) -> None:
        self.a = 4


class Annotated:
    __slots__: tuple = ('a',)

    def __init__(self) -> None:
        self.a = 5


class Base:
    __slots__ = ('x',)

    def __init__(self) -> None:
        self.x = 10


class Derived(Base):
    __slots__ = ('y',)

    def __init__(self) -> None:
        self.x = 11
        self.y = 12


class Redeclared(Base):
    __slots__ = ('x',)

    def __init__(self) -> None:
        self.x = 13


class Open(Base):
    def __init__(self) -> None:
        self.x = 14
        self.extra = 15


class Plain:
    def __init__(self) -> None:
        self.p = 16


class SlottedOnPlain(Plain):
    __slots__ = ('q',)

    def __init__(self) -> None:
        self.p = 17
        self.q = 18


class Root:
    __slots__ = ('r',)

    def __init__(self) -> None:
        self.r = 19


class Left(Root):
    __slots__ = ('left',)

    def __init__(self) -> None:
        self.r = 20
        self.left = 21


class Right(Root):
    __slots__ = ()


class Diamond(Left, Right):
    __slots__ = ()


class CodedError(Exception):
    __slots__ = ('code',)


def main() -> None:
    conf = ParseConf(2, 3)
    print(conf.parse_table, conf.start, conf.states)
    print(Single(7).value)
    both = Parenthesised()
    print(both.a + both.b)
    print(AsList().a)
    print(Empty().total())
    print(NeverAssigned().a)
    print(Annotated().a)
    print(Base().x)
    derived = Derived()
    print(derived.x, derived.y)
    print(Redeclared().x)
    opened = Open()
    print(opened.x, opened.extra)
    slotted = SlottedOnPlain()
    print(slotted.p, slotted.q)
    diamond = Diamond()
    print(diamond.r, diamond.left)
    try:
        raise CodedError('coded')
    except CodedError:
        print('caught')


main()
