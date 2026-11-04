from typing import Literal, NamedTuple, TypedDict


class Esc(TypedDict):
    # The text `Literal['\d']` warns when it is compiled (an invalid escape).
    a: "Literal['\\d']"
    b: "int"


class EscT(NamedTuple):
    x: "Literal['\\w']"
    y: "str" = ""
