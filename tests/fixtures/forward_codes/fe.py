import typing
from typing import TypedDict


class Eager(TypedDict):
    a: "int"
    b: list["str"]
    c: "typing.Optional[int]"
    d: "*tuple[int]"


try:

    class Bad(TypedDict):
        x: "1 +"

except SyntaxError as e:
    print("Bad:", e)


class Pair(typing.NamedTuple):
    left: "int"
    right: "list[Pair] | None" = None


explicit = typing.ForwardRef("int")


class Again(TypedDict):
    a: "int"
    e: "typing.Optional[int]"
