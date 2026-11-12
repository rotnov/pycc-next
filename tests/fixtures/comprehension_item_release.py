# A comprehension that unpacks its item (`for k, v in d.items()`) over a
# dict's own `items()` releases each item before it advances: two names
# read only by position are bound straight from the dict
# (`tests/fixtures/dict_pairs.py`), and any other unpacking target releases
# the pair tuple, so the iterator hands it out again
# (docs/design/compile-little.md, section 2.6). An item bound whole, and
# every other iterable's item, lives until the next one is bound, as in
# CPython: these cases print when each item is finalized.
import collections

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
        return self.name == other.name

    def __lt__(self, other):
        return self.name < other.name


def produce(n):
    for i in range(n):
        LOG.append("yield " + str(i))
        yield Noisy(str(i))


class Countdown:
    def __init__(self, n):
        self.n = n

    def __iter__(self):
        return self

    def __next__(self):
        LOG.append("next " + str(self.n))
        if self.n == 0:
            raise StopIteration
        self.n -= 1
        return Noisy("c" + str(self.n))


class ItemsLike:
    # Not a dict: `items()` is a generator of fresh pairs.
    def __init__(self, n):
        self.n = n

    def items(self):
        for i in range(self.n):
            LOG.append("pair " + str(i))
            yield Noisy("k" + str(i)), i


class DictSub(dict):
    def items(self):
        LOG.append("items()")
        return [(k, self[k]) for k in sorted(self)]


def note(x):
    LOG.append("use " + x.name)
    return x.name


def over_generator(n):
    return [note(x) for x in produce(n)]


def over_iterator(n):
    return [note(x) for x in Countdown(n)]


def over_list(n):
    return [note(x) for x in [Noisy("l" + str(i)) for i in range(n)]]


def filtered(n):
    return [note(x) for x in produce(n) if x.name != "1"]


def set_and_dict(n):
    return sorted({note(x) for x in produce(n)}), {note(x): 1 for x in produce(n)}


def items_like(n):
    return [(note(k), v) for k, v in ItemsLike(n).items()]


def items_subclass(d):
    return [(k, v) for k, v in DictSub(d).items()]


def items_pairs(d):
    return [(k, v) for k, v in d.items()]


def items_kept(d):
    # Each pair is kept: the iterator cannot reuse it.
    pairs = [item for item in d.items()]
    return pairs, len({id(p) for p in pairs}) == len(pairs)


def items_filtered(d):
    return [k for k, v in d.items() if v % 2]


def items_nested(d):
    return [(k, [(a, b) for a, b in v.items()]) for k, v in d.items()]


def items_nested_target(d):
    # Not two names: the pair tuple is unpacked, then released.
    return [(a, v) for (a, *b), v in d.items()]


def items_sorted(d):
    return tuple(sorted((k, v) for k, v in d.items()))


def items_dict(d):
    return {v: k for k, v in d.items()}, sorted({repr(k) for k, v in d.items()})


def items_values_replaced(d):
    # The value a pair held is replaced while the loop runs.
    out = []
    for name in [k for k, v in d.items()]:
        out.append(name)
    seen = [d.__setitem__(k, None) or note(v) for k, v in d.items()]
    return out, seen


def items_whole_replaced(d):
    # The pair is bound whole, so it lives until the next one is bound and
    # the iterator cannot reuse it: a replaced value outlives the next use.
    return [d.__setitem__(pair[0], None) or note(pair[1]) for pair in d.items()]


def items_whole_named_like_a_temporary(d):
    # The names only look like the compiler's own: each pair is bound whole.
    first = [d.__setitem__(q0__pycc_1[0], None) or note(q0__pycc_1[1]) for q0__pycc_1 in d.items()]
    second = [note(x9__pycc_unpack_0[0]) for x9__pycc_unpack_0 in [(Noisy("t" + str(i)),) for i in range(3)]]
    return first, second


def items_whole_kept(d):
    return [note(pair[1]) for pair in d.items() if pair[0] != "b"]


def items_filtered_replaced(d):
    return [d.__setitem__(k, None) or note(v) for k, v in d.items() if k != "b"]


class PlainSub(dict):
    # A dict subclass that keeps `dict.items`: the same iterator.
    pass


def items_plain_subclass_replaced(d):
    d = PlainSub(d)
    return [d.__setitem__(k, None) or note(v) for k, v in d.items()]


def items_dict_replaced(d):
    return {k: d.__setitem__(k, None) or note(v) for k, v in d.items()}


def items_set_replaced(d):
    return sorted({d.__setitem__(k, None) or note(v) for k, v in d.items()})


def items_resized_replaced(d):
    # The loop replaces a value, then grows the dict: the next advance fails.
    return [d.__setitem__(k, None) or d.__setitem__(k + "!", 0) or note(v) for k, v in d.items()]


def items_grown(d):
    return [d.__setitem__(k + "!", v) for k, v in d.items()]


def items_fails(d):
    return [10 // v for k, v in d.items()]


def items_views(d):
    return [k for k in d.keys()], [v for v in d.values()], [k for k in d], [p for p in d.items()]


def items_ordered(d):
    return [(k, v) for k, v in collections.OrderedDict(d).items()]


def generator_fails(n):
    return [x if x != 2 else 1 // 0 for x in range(n)]


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


def whole_case(f):
    # The last two values are one object: which of the iterator and the
    # last pair goes first at the end of the loop is not compared.
    last = Noisy("vz")
    shown(f, {"a": Noisy("va"), "b": Noisy("vb"), "c": last, "d": last})


def main():
    for n in (0, 1, 3):
        for f in (over_generator, over_iterator, over_list, filtered, set_and_dict, items_like):
            shown(f, n)
    shown(generator_fails, 4)
    for d in ({}, {"a": 1}, {"a": 1, "b": 2, "c": 3}, {(1, 2): 3, None: 4, "x": 5}):
        for f in (items_subclass, items_pairs, items_kept, items_filtered, items_sorted, items_dict,
                  items_views, items_ordered):
            shown(f, dict(d))
    shown(items_nested, {"x": {"a": 1, "b": 2}, "y": {}, "z": {"c": 3}})
    shown(items_nested_target, {(1, 2): 3, "xy": 4, (5,): 6})
    shown(items_nested_target, {7: 8})
    for f in (items_whole_replaced, items_whole_named_like_a_temporary, items_whole_kept):
        whole_case(f)
    shown(items_grown, {"a": 1, "b": 2})
    shown(items_fails, {"a": 1, "b": 0, "c": 2})
    shown(items_pairs, {Noisy("ka"): Noisy("va"), Noisy("kb"): Noisy("vb")})
    shown(items_pairs, [1, 2])
    shown(items_pairs, None)


def replaced():
    # With the items released early, a value the loop itself drops from the
    # dict is finalized where CPython finalizes it: before the next pair is
    # used. (With the fast paths off it outlives the next pair's use, which
    # is why `main` leaves this case out.)
    shown(items_values_replaced, {"a": Noisy("va"), "b": Noisy("vb"), "c": Noisy("vc")})
    shown(items_filtered_replaced, {"a": Noisy("va"), "b": Noisy("vb"), "c": Noisy("vc"), "d": Noisy("vd")})
    for f in (items_plain_subclass_replaced, items_dict_replaced, items_set_replaced, items_resized_replaced):
        shown(f, {"a": Noisy("va"), "b": Noisy("vb"), "c": Noisy("vc")})


main()
