# v0.9 accept 1: `except` naming an imported exception class (a CPython
# object), alone, with `as`, among builtin classes and in a tuple; the clause
# is tested when it is reached, in order, as CPython does.
import json
from json import JSONDecodeError
from struct import error as StructError
import struct


def parse(text):
    try:
        return json.loads(text)
    except JSONDecodeError:
        return None


def describe(text):
    try:
        return json.loads(text)
    except KeyError:
        return "key"
    except JSONDecodeError as e:
        return "bad at " + str(e.pos)
    except ValueError:
        return "value"


def unpack(data):
    try:
        return struct.unpack("<i", data)[0]
    except (StructError, TypeError) as e:
        return type(e).__name__


def nested(text, key):
    try:
        try:
            return json.loads(text)[key]
        except JSONDecodeError:
            return "outer " + key
    except KeyError as e:
        return "missing " + str(e)


def reraise(text):
    try:
        json.loads(text)
    except StructError:
        return "struct"


print(parse("[1, 2]"), parse("[1"), describe("{}"), describe("{"), describe('"a"'))
print(unpack(b"\x01\x00\x00\x00"), unpack(b"\x01"), unpack(None))
print(nested('{"a": 1}', "a"), nested("{", "a"), nested("{}", "a"))
try:
    reraise("{")
except JSONDecodeError as e:
    print("propagated", e.msg)
