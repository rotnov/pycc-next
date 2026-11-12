# A comprehension that unpacks its item into two names read only by
# position (`[f(k, v) for k, v in d.items()]`) binds the two names straight
# from the source: over an exact dict's `items()` with `PyDict_Next`, with
# no view, iterator or pair tuple, and with the dict iterator's checks for a
# dict changed meanwhile; over anything else by unpacking each item as
# `UNPACK_SEQUENCE` does. `sorted()` of a generator expression sorts the list
# it was materialized into in place, `tuple()` of a list only it holds moves
# the items, and a tuple display stores its items straight into a new tuple
# (docs/design/compile-little.md, section 2.6). These cases print the
# results, the exceptions and when each item is finalized.
import collections
import gc

LOG = []


class Noisy:
    def __init__(self, name):
        self.name = name

    def __del__(self):
        LOG.append("del " + self.name)

    def __repr__(self):
        return "Noisy(" + self.name + ")"

    def __hash__(self):
        return hash(self.name)

    def __eq__(self, other):
        return isinstance(other, Noisy) and self.name == other.name

    def __lt__(self, other):
        # No order for "bad": CPython raises the `TypeError`, so no frame
        # of this method outlives the comparison in the traceback.
        if self.name == "bad" or other.name == "bad":
            return NotImplemented
        return self.name < other.name


class ItemsLike:
    # A mapping that is not a dict: `items()` is a generator of pairs.
    def __init__(self, n):
        self.n = n

    def items(self):
        for i in range(self.n):
            LOG.append("pair " + str(i))
            yield Noisy("k" + str(i)), Noisy("v" + str(i))


class DictSub(dict):
    def items(self):
        LOG.append("items()")
        return [(k, self[k]) for k in sorted(self)]


class PlainSub(dict):
    pass


class Pairish:
    # An item that is iterable but not subscriptable.
    def __init__(self, a, b):
        self.a = a
        self.b = b

    def __iter__(self):
        LOG.append("iter pair")
        return iter((self.a, self.b))


class Spy:
    # An element that records, while it is compared, how many lists hold it.
    def __init__(self, n):
        self.n = n

    def __lt__(self, other):
        LOG.append(len([r for r in gc.get_referrers(self) if type(r) is list]))
        return self.n < other.n

    def __repr__(self):
        return "Spy(" + str(self.n) + ")"


def note(x):
    LOG.append("use " + repr(x))
    return x


def pairs(d):
    return [(k, v) for k, v in d.items()]


def keys_only(d):
    return [k for k, v in d.items()]


def values_filtered(d):
    return [v for k, v in d.items() if k != "b"]


def swapped(d):
    return {v: k for k, v in d.items()}


def value_set(d):
    return sorted({repr(v) for k, v in d.items()})


def used_twice(d):
    return [(k, k, v, v) for k, v in d.items() if v is not None and k]


def nested(d):
    return [(k, [(a, b) for a, b in v.items()]) for k, v in d.items()]


def grown(d):
    return [d.__setitem__(k + "!", v) for k, v in d.items()]


def shrunk(d):
    return [d.pop(k) for k, v in d.items()]


def keys_changed(d):
    # The size stays, but a key is replaced while the loop runs.
    return [d.pop(k) is None or d.__setitem__(k + "!", v) for k, v in d.items()]


def values_replaced(d):
    return [d.__setitem__(k, None) or note(v) for k, v in d.items()]


def body_fails(d):
    return [note(k) and 10 // v.n for k, v in d.items()]


def subclass(d):
    return [(k, v) for k, v in DictSub(d).items()]


def plain_subclass(d):
    return [(k, v) for k, v in PlainSub(d).items()]


def ordered(d):
    return [(k, v) for k, v in collections.OrderedDict(d).items()]


def mapping(n):
    return [(note(k), v) for k, v in ItemsLike(n).items()]


def over_list(src):
    return [(b, a) for a, b in src]


def over_iterator(src):
    return [(b, a) for a, b in iter(src)]


def over_zip(xs, ys):
    return [a + b for a, b in zip(xs, ys)]


def over_pairish(n):
    return [(a, b) for a, b in [Pairish(i, -i) for i in range(n)]]


def generator_pairs(d):
    return list((v, k) for k, v in d.items())


def sorted_pairs(d):
    return tuple(sorted((k, v) for k, v in d.items()))


def sorted_generator(n):
    return sorted(Noisy(str(i)) for i in range(n, 0, -1))


def sorted_generator_fails(names):
    return sorted(Noisy(name) for name in names)


def sorted_key(d):
    return sorted((k for k, v in d.items()), key=lambda k: d[k])


def sorted_key_reverse(d):
    return sorted((v for k, v in d.items()), key=repr, reverse=True)


def sorted_reverse(n):
    return sorted((i * 7 % 5 for i in range(n)), reverse=True)


def generator_raises(n):
    return sorted(10 // (2 - i) for i in range(n))


def tuple_generator_raises(n):
    return tuple(Noisy(str(10 // (2 - i))) for i in range(n))


def sorted_spied_generator(n):
    # One list: the one `sorted` fills from the generator.
    return sorted(Spy(i) for i in range(n, 0, -1))


def sorted_spied_named_like_fresh(n):
    # A list comprehension, whose variable only looks like the compiler's
    # own mark: `sorted` sorts a copy of it.
    return sorted([x_pycc_fresh_y for x_pycc_fresh_y in [Spy(i) for i in range(n, 0, -1)]])


def tuple_of_listcomp(n):
    return tuple([Noisy(str(i)) for i in range(n)])


def tuple_of_sorted(n):
    t = tuple(sorted(Noisy(str(i)) for i in range(n)))
    return t, type(t).__name__


def tuple_of_kept(n):
    kept = [i for i in range(n)]
    t = tuple(kept)
    kept.append("more")
    return t, kept


def sorted_of_kept(n):
    kept = [n - i for i in range(n)]
    s = sorted(kept)
    return s, kept, s is kept


def displays(d):
    out = []
    for k in sorted(d):
        out.append((k, d[k], (k,), ()))
    return out


def display_fails(d):
    return [(note(k), 10 // v) for k, v in d.items()]


def display_item_fails(x):
    return (note(Noisy("first")), note(Noisy("second")), 1 // x)


def shadowed_sorted(d, sorted=lambda xs: ["shadow", list(xs)]):
    return sorted(k for k, v in d.items())


def shadowed_tuple(n, tuple=lambda xs: ("shadow", list(xs))):
    return tuple([i for i in range(n)])


def shadowed_isinstance(x, isinstance=lambda x, t: "shadow"):
    return isinstance(x, dict)


def hashable(value):
    # The shape of pydantic's `_make_json_hashable`.
    if isinstance(value, dict):
        return tuple(sorted((k, hashable(v)) for k, v in value.items()))
    elif isinstance(value, list):
        return tuple(hashable(v) for v in value)
    else:
        return value


def shown(f, *args):
    LOG.clear()
    try:
        result = f(*args)
    except BaseException as e:
        LOG.append("raised")
        print(f.__name__, "raised", type(e).__name__, e.args)
        del e
    else:
        print(f.__name__, repr(result))
        del result
    LOG.append("returned")
    print(" ", LOG)


def main():
    for d in ({}, {"a": 1}, {"a": 1, "b": 2, "c": 3}, {(1, 2): 3, None: 4, "x": 5}):
        for f in (pairs, keys_only, values_filtered, swapped, value_set, used_twice, subclass,
                  plain_subclass, ordered, generator_pairs, sorted_pairs, displays,
                  shadowed_sorted):
            shown(f, dict(d))
    shown(sorted_key, {"a": 3, "b": 1, "c": 2})
    shown(sorted_key_reverse, {"a": 3, "b": 1, "c": 2})
    shown(nested, {"x": {"a": 1, "b": 2}, "y": {}, "z": {"c": 3}})
    for f in (grown, shrunk, keys_changed):
        shown(f, {"a": 1, "b": 2})
    shown(shrunk, {})
    shown(body_fails, {Noisy("ka"): Noisy("va"), Noisy("kb"): 0, Noisy("kc"): Noisy("vc")})
    shown(pairs, {Noisy("ka"): Noisy("va"), Noisy("kb"): Noisy("vb")})
    shown(pairs, [1, 2])
    shown(pairs, None)
    shown(display_fails, {"a": 1, "b": 0, "c": 2})
    for n in (0, 1, 3):
        shown(mapping, n)
        shown(over_pairish, n)
        shown(sorted_generator, n)
        shown(sorted_reverse, n)
        shown(generator_raises, n)
        shown(tuple_generator_raises, n)
        shown(sorted_spied_named_like_fresh, n)
        shown(tuple_of_listcomp, n)
        shown(tuple_of_sorted, n)
        shown(tuple_of_kept, n)
        shown(sorted_of_kept, n)
        shown(shadowed_tuple, n)
    for src in ([], [(1, 2), [3, 4], "ab"], [(1, 2, 3)], [(1,)], [1], [(Noisy("x"), Noisy("y")), None],
                {(1, 2)}, [[Noisy("p"), Noisy("q")], [Noisy("r")]]):
        shown(over_list, src)
        shown(over_iterator, src)
    shown(over_zip, [1, 2, 3], [10, 20])
    shown(sorted_generator_fails, ["b", "a", "c"])
    shown(sorted_generator_fails, ["b", "bad", "a"])
    shown(display_item_fails, 1)
    shown(display_item_fails, 0)
    shown(shadowed_isinstance, {})
    for value in ({"b": [1, {"y": 2, "x": [3]}], "a": None}, [], {}, [{"k": "v"}, 1], 5):
        shown(hashable, value)


def replaced():
    # A value the loop drops from the dict is finalized where CPython
    # finalizes it: once the pair holding it is gone, before the next one.
    shown(values_replaced, {"a": Noisy("va"), "b": Noisy("vb"), "c": Noisy("vc")})
    # `sorted` of a generator sorts the generator's own list, as CPython's
    # does: no other list holds an element while it is compared. (With the
    # fast paths off the list is copied, which is why `main` leaves these
    # cases out.)
    for n in (0, 1, 3):
        shown(sorted_spied_generator, n)


main()
