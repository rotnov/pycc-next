# v0.9 accept 1: an instance attribute holding a CPython object called like a
# method, a compiled instance tested for identity against an object (a
# sentinel), and a private helper called with instances of several classes.
import html


class NotFound(object):
    pass


_NOT_FOUND = NotFound()


def get_value(context, key):
    if isinstance(context, dict):
        if key in context:
            return context[key]
    return _NOT_FOUND


class Engine:
    def __init__(self, escape=None, resolve=None):
        self.escape = escape
        self.resolve = resolve

    def fetch(self, context, name):
        value = self.resolve(context, name)
        if value is _NOT_FOUND:
            return "<missing " + name + ">"
        return self.escape(str(value))


def _describe(obj):
    return type(obj).__name__ + ":" + str(obj.size())


class Small:
    def size(self):
        return 1

    def __repr__(self):
        return _describe(self)


class Large:
    def size(self):
        return 100

    def __repr__(self):
        return _describe(self)


def run(context, names):
    engine = Engine(escape=lambda s: html.escape(s, quote=True), resolve=get_value)
    return [engine.fetch(context, name) for name in names]


print(run({"a": "<b>", "n": 3}, ["a", "n", "z"]))
print(run([1], ["a"]))
e = Engine(escape=str.upper, resolve=lambda c, n: c.get(n, _NOT_FOUND))
print(e.fetch({"x": "y"}, "x"), e.fetch({}, "x"))
e.escape = None
try:
    e.fetch({"x": "y"}, "x")
except TypeError as err:
    print("TypeError", err)
print(get_value({}, "k") is _NOT_FOUND, get_value({"k": None}, "k") is _NOT_FOUND)
print(_describe(Small()), _describe(Large()))
