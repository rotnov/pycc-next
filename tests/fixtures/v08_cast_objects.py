"""v0.8 accept 4: `typing.cast(T, x)` of a CPython object is that object,
whatever `T` spells -- a scalar, a class, a subscripted generic."""

import json
from typing import Any, Callable, Dict, List, Optional, cast


def raw(token):
    return "<" + cast(str, token["raw"]) + ">"


def level(token):
    return cast(int, token["attrs"]["level"]) + 1


def items(token):
    return cast(List[Dict[str, Any]], token["children"])


def maybe(token):
    v = cast(Optional[int], token.get("n"))
    return -1 if v is None else v


TOKEN = '{"raw": "x", "attrs": {"level": 2}, "children": [{"type": "a"}], "n": 7}'
tok = json.loads(TOKEN)
print(raw(tok), level(tok), items(tok) is tok["children"], maybe(tok), maybe(json.loads("{}")))
print(raw(json.loads('{"raw": 5}')))
