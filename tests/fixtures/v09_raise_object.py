# v0.9: a compiled exception class raised with a CPython object argument.
class MissingError(Exception):
    pass


def lookup(table, name):
    if name not in table:
        raise MissingError(name)
    return table[name]


def safe(table, name):
    try:
        return lookup(table, name)
    except MissingError:
        return "missing"


print(lookup({"a": 1}, "a"), safe({}, "b"), safe({}, 3))
