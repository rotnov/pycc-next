import re


def camel(snake):
    return re.sub("_([a-z])", lambda m: m.group(1).upper(), snake)


def scaled(factor):
    return lambda x, offset=1: x * factor + offset


def make_all(n):
    fs = []
    for i in range(n):
        fs.append(lambda: i)
    return [f() for f in fs]


def bind_now(n):
    fs = []
    for i in range(n):
        fs.append(lambda i=i: i)
    return [f() for f in fs]


def rebound():
    base = 10
    f = lambda v: v + base
    base = 20
    return f(1)


def picker(key):
    table = {"a": 1, "b": 2}
    getter = (lambda k: table[k]) if key else None
    return getter(key) if getter else None


def nested_lambda(a):
    add = lambda b: (lambda c: a + b + c)
    inner = add(2)
    return inner(3)


def keyed(items):
    return sorted(items, key=lambda kv: (-kv[1], kv[0]))


def main():
    print(camel("snake_case_name"), scaled(3)(2), scaled(3)(2, offset=5))
    print(make_all(3), bind_now(3), rebound(), picker("b"), nested_lambda(1))
    print(keyed([("x", 1), ("y", 3), ("z", 3)]))
    f = scaled(2)
    print(f.__name__, f.__qualname__, f.__defaults__, (lambda: 0).__qualname__)
    scaled(1)("a")


main()
