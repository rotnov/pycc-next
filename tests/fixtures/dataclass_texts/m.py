import dataclasses
import typing
from dataclasses import InitVar, KW_ONLY, dataclass, field
from typing import TYPE_CHECKING, ClassVar


Imported = dataclasses.make_dataclass("Imported", [("base", int, 1)])


@dataclass(kw_only=False)
class Plain:
    x: int
    y: str = "y"
    z: list = field(default_factory=list)


@dataclass(frozen=True, slots=True)
class FrozenSlots:
    """A point."""

    a: int
    b: int = 2
    c: tuple = ()

    def norm(self) -> int:
        return self.a + self.b


@dataclass(order=True, unsafe_hash=True, repr=False, match_args=False)
class Ordered:
    k: int
    v: float = field(default=0.5, compare=False)
    w: int = field(default=1, repr=False, hash=False)


@dataclass(eq=False, kw_only=True)
class KwOnly:
    p: int
    q: int = 3


@dataclasses.dataclass
class Specials:
    count: ClassVar[int] = 0
    seed: InitVar[int] = 1
    name: str = "n"
    _: KW_ONLY
    flag: bool = False
    hidden: int = field(default=7, init=False)

    def __post_init__(self, seed: int) -> None:
        self.name = self.name * seed

    if TYPE_CHECKING:

        def typed(self) -> None: ...


@dataclasses.dataclass(kw_only=False)
class Child(Plain):
    extra: int = 9


@dataclass(frozen=True)
class FrozenChild(FrozenSlots.__mro__[0] if False else object):
    m: int = 0


@dataclass(kw_only=False)
class OwnEq:
    e: int

    def __eq__(self, other):
        return True

    __hash__ = None


@dataclass(kw_only=False)
class Twin1:
    t: int


@dataclass(kw_only=False)
class Twin2:
    t: int


@dataclass(kw_only=False)
class FromElsewhere(Imported):
    u: int = 0


@dataclass(slots=True, weakref_slot=True)
class Weak:
    r: int = 0


try:

    @dataclass(order=True)
    class BadOrder:
        o: int

        def __lt__(self, other):
            return False

except TypeError as e:
    print("BadOrder:", e)

try:

    @dataclass
    class BadDefault:
        d: list = []

except ValueError as e:
    print("BadDefault:", e)

try:

    @dataclass
    class BadOrderOfDefaults:
        a: int = 1
        b: int

except TypeError as e:
    print("BadOrderOfDefaults:", e)
