from functools import partial
from collections.abc import Callable
from typing import Any


def max_digits(x: Any, limit: Any) -> Any:
    return ("max_digits", x, limit)


def places(x: Any, places: Any, limit: Any = None) -> Any:
    return ("places", x, places, limit)


LOOKUP: dict[str, Callable] = {"max_digits": max_digits, "places": places}
PAIR = (max_digits, places)


def chosen(name: str) -> Any:
    return LOOKUP[name]


def bound(name: str, **kwargs: Any) -> Any:
    return partial(LOOKUP[name], **kwargs)


def own_name(x: Any, own_name: Any) -> Any:
    return ("own_name", x, own_name)


def own_default(x: Any, own_default: Any = None) -> Any:
    return ("own_default", x, own_default)
