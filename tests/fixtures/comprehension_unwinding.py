# What a comprehension over an object releases, and in which order: the
# exhausted iterator before the last item, and, when the element or the
# condition raises, the iterator, the result (and what it holds) and then the
# item. Every case prints what CPython prints.


class Fin:
    # Reports when it is finalized.
    def __init__(self, name, log):
        self.name = name
        self.log = log

    def __hash__(self):
        if self.name == "bad":
            raise TypeError("no hash for " + self.name)
        return hash(self.name)

    def __eq__(self, other):
        return isinstance(other, Fin) and self.name == other.name

    def __del__(self):
        self.log.append("del " + self.name)


class Source:
    # An iterator of fresh items that reports when it is finalized itself.
    def __init__(self, names, log):
        self.names = list(names)
        self.log = log

    def __iter__(self):
        return self

    def __next__(self):
        if not self.names:
            raise StopIteration
        return Fin(self.names.pop(0), self.log)

    def __del__(self):
        self.log.append("del source")


def names_of(names, log):
    # The last item outlives the iterator.
    return [x.name for x in Source(names, log)]


def name_set(names, log):
    return sorted({x.name for x in Source(names, log)})


def name_dict(names, log):
    return {x.name: len(x.name) for x in Source(names, log)}


def pairs_of(names, log):
    # The element is a tuple.
    return [(x.name, len(x.name)) for x in Source(names, log)]


def filtered(names, log):
    return [x.name for x in Source(names, log) if len(x.name) > 1]


def element_raises(names, log):
    try:
        return [x.name[2] for x in Source(names, log)]
    except IndexError:
        log.append("caught IndexError")
        return "handled"


def element_raises_escapes(names, log):
    return [x.name[2] for x in Source(names, log)]


def condition_raises(names, log):
    try:
        return [x.name for x in Source(names, log) if x.name[2] == "c"]
    except IndexError:
        log.append("caught IndexError")
        return "handled"


def key_raises(names, log):
    try:
        return {x.name[2]: x.name for x in Source(names, log)}
    except IndexError:
        log.append("caught IndexError")
        return "handled"


def tuple_element_raises(names, log):
    try:
        return [(x.name, x.name[2]) for x in Source(names, log)]
    except IndexError:
        log.append("caught IndexError")
        return "handled"


def items_kept(names, log):
    # The result holds the items collected so far: it goes after the
    # iterator and before the item that failed.
    try:
        return len({x: x.name for x in Source(names, log)})
    except TypeError:
        log.append("caught TypeError")
        return "handled"


def items_kept_in_set(names, log):
    try:
        return len({x for x in Source(names, log)})
    except TypeError:
        log.append("caught TypeError")
        return "handled"


def produced(names, log):
    try:
        for name in names:
            yield Fin(name, log)
    finally:
        log.append("generator closed")


def items_kept_from_generator(names, log):
    try:
        return len({x: x.name for x in produced(names, log)})
    except TypeError:
        log.append("caught TypeError")
        return "handled"


def over_list(items, log):
    # The list keeps its items: only the comprehension's own reference to
    # the item that failed is released.
    try:
        return [x.name[2] for x in items]
    except IndexError:
        log.append("caught IndexError")
        return "handled"


def miss_in_comprehension(d, names, log):
    try:
        return [d[x.name] for x in Source(names, log)]
    except KeyError:
        log.append("caught KeyError")
        return "handled"


def twice(names, log):
    # The second run starts from an empty slot.
    first = element_raises(names, log)
    second = element_raises(["abc"], log)
    return first, second


def shown(f, *args):
    try:
        result = f(*args)
    except BaseException as e:
        print(f.__name__, "raised", type(e).__name__, e.args)
    else:
        print(f.__name__, repr(result))


def main():
    for f in (names_of, name_set, name_dict, pairs_of, filtered, element_raises,
              element_raises_escapes, condition_raises, key_raises, tuple_element_raises, twice,
              items_kept, items_kept_from_generator):
        for names in (["abc", "xyz"], ["abc", "d", "xyz"], ["d"], [], ["abc", "bad", "xyz"]):
            log = []
            shown(f, names, log)
            print(log)
    log = []
    shown(items_kept_in_set, ["abc", "bad", "xyz"], log)
    print(log)
    log = []
    items = [Fin("abc", log), Fin("d", log), Fin("xyz", log)]
    shown(over_list, items, log)
    print(log)
    del items
    print(log)
    for names in (["a", "b"], ["a", "zz", "b"]):
        log = []
        shown(miss_in_comprehension, {"a": 1, "b": 2}, names, log)
        print(log)


main()
