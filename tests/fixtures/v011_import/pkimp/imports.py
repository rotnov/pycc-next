"""`from m import a, b` statements of a module CPython imports (`json`) and of one
whose attribute lookups run another such statement (`ext_helper`)."""
from json import dumps, loads, JSONDecoder


def again() -> str:
    from json import dumps, loads
    return dumps(loads("[1]"))


def failing() -> str:
    try:
        from json import dumps, nope, loads  # noqa: F401
    except ImportError:
        return "ImportError"
    return "imported"


def nested() -> str:
    from ext_helper import a, b
    return a + " " + b


def encoder() -> str:
    return dumps({"k": [1, 2]}) + " " + str(loads("[3]")) + " " + JSONDecoder.__name__
