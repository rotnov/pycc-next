# Non-strict rebinding: a name the `try` body and a handler bind with two
# types becomes an object; an object rebinding an `int` parameter is
# converted with a type check; `type()` of a native `str`.
import json


def char_at(src: str, pos: int):
    try:
        char = src[pos]
    except IndexError:
        char = None
    return char


def find(src: str, expect: str, pos: int) -> int:
    try:
        new_pos = src.index(expect, pos)
    except ValueError:
        new_pos = len(src)
    return new_pos


def advance(pos: int, step: str) -> int:
    pos = json.loads(step)
    return pos + 1


def kind(value: str) -> str:
    return type(value).__qualname__


print(char_at("ab", 1), char_at("ab", 5))
print(find("abc", "c", 0), find("abc", "x", 0), find("abcabc", "c", 3))
print(advance(0, "41"), advance(5, "true"))
try:
    advance(0, '"x"')
except TypeError:
    print("TypeError")
print(kind("x"))
