# v0.10: `map`/`filter` of a nested function used as a value run CPython's
# own lazy builtins over the compiled function: a `StopIteration` the
# function raises ends `list(map(...))` early, a starred or unpacked `map`
# calls it only as far as CPython consumes it.


def stop_at(limit, values):
    limit = limit + 0

    def until(v):
        if v >= limit:
            raise StopIteration
        return v * 10

    def small(v):
        if v >= limit:
            raise StopIteration
        return True

    return list(map(until, values)), tuple(map(until, values)), list(filter(small, values))


def joined(sep, values):
    sep = sep + ""

    def text(v):
        return sep + str(v)

    return "".join(map(text, values))


def unpacked(values):
    seen = []

    def note(v):
        seen.append(v)
        return v

    try:
        a, b = map(note, values)
    except ValueError as e:
        return str(e), seen
    return a + b, seen


def show(*args, sep):
    return sep.join(str(a) for a in args)


def starred(values):
    values = values + []

    def twice(v):
        return v * 2

    return show(*map(twice, values), sep="-")


print(stop_at(3, [1, 2, 3, 4]))
print(joined("#", [1, 2]))
print(unpacked([1, 2]))
print(unpacked([1, 2, 3, 4]))
print(starred([1, 2, 3]))
