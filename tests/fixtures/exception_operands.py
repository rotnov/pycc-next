# An exception instance has no form as an operand of an operation CPython
# runs: `"..." % e` formats it in CPython, so the function that does it is
# not compiled (it read the exception as another kind of object and printed
# garbage, or crashed). Every case prints what CPython prints.


class Custom(ValueError):
    pass


def one(d, k):
    try:
        return d[k]
    except KeyError as e:
        return "caught %s" % e


def pair(d, keys):
    out = []
    for j in range(len(keys)):
        try:
            out.append(d[keys[j]])
        except KeyError as e:
            out.append("%d %s" % (j, e))
    return out


def repr_of(text):
    try:
        return int(text)
    except ValueError as e:
        return "%r" % e


def raised(message):
    try:
        raise Custom(message)
    except Custom as e:
        return "%s and %s" % (e, message)


def native(n, text):
    # Scalars still format natively.
    return "%d %s" % (n, text)


def main():
    print(one({}, "k"), one({"k": 1}, "k"))
    print(pair({"a": 1}, ["a", "b", "c"]))
    print(repr_of("x"), repr_of("7"))
    print(raised("m"))
    print(native(3, "three"))


main()
