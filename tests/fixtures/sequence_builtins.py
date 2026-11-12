# `tuple(a)`, `list(a)` and `sorted(a)` of an object run what the builtins
# run without the call through them (docs/design/compile-little.md, section
# 2.6). `sorted()` of a list display or comprehension still copies it, as
# CPython does, and so does `list()` of any list: these cases print what
# would show a list reused (identity, aliasing, what a comparison sees
# through `gc`, the order a failed sort leaves the elements' finalizers
# in). Only `sorted()` of a generator expression sorts its own list in
# place, and `tuple()` of a list nothing else holds moves its items, as
# neither can be seen (`tests/fixtures/dict_pairs.py`).
import gc
class Tracked(list):
    pass


class Bad:
    def __init__(self, n):
        self.n = n

    def __lt__(self, other):
        if self.n == 3 or other.n == 3:
            raise ValueError("no order for 3")
        return self.n < other.n

    def __repr__(self):
        return "Bad(" + str(self.n) + ")"


class Loud:
    log = []

    def __init__(self, n):
        self.n = n

    def __lt__(self, other):
        Loud.log.append((self.n, other.n))
        return self.n < other.n

    def __repr__(self):
        return "Loud(" + str(self.n) + ")"


class Fin:
    # An element that reports when it is finalized; 3 has no order.
    def __init__(self, n, log):
        self.n = n
        self.log = log

    def __lt__(self, other):
        if not isinstance(other, Fin):
            return NotImplemented
        if self.n == 3 or other.n == 3:
            raise ValueError("no order for 3")
        return self.n < other.n

    def __del__(self):
        # The lambda keeps this method in CPython: a compiled finalizer run
        # while an exception unwinds is not what these cases are about.
        (lambda log, n: log.append("del " + str(n)))(self.log, self.n)

    def __repr__(self):
        return "Fin(" + str(self.n) + ")"


class Spy:
    # An element that looks, while it is compared, for the lists holding it.
    def __init__(self, n, log):
        self.n = n
        self.log = log

    def __lt__(self, other):
        holders = [r for r in gc.get_referrers(self) if type(r) is list]
        self.log.append(sorted(len(r) for r in holders))
        return self.n < other.n

    def __repr__(self):
        return "Spy(" + str(self.n) + ")"


class Grabber:
    # An element that keeps the list holding it, found while it is compared.
    taken = []

    def __init__(self, n):
        self.n = n

    def __lt__(self, other):
        if not Grabber.taken:
            Grabber.taken.extend(r for r in gc.get_referrers(self) if type(r) is list)
        return self.n < other.n

    def __repr__(self):
        return "Grabber(" + str(self.n) + ")"


KEPT = []


def keep(xs):
    KEPT.append(xs)
    return xs


def fresh(n):
    return [(n * 7 + i * 5) % 11 for i in range(n)]


def sorted_comprehension(xs):
    return sorted([x * 2 for x in xs])


def sorted_generator(xs):
    return sorted(x * 2 for x in xs)


def tuple_sorted_generator(d):
    return tuple(sorted((k, v) for k, v in d.items()))


def list_comprehension(xs):
    return list([x + 1 for x in xs])


def list_of_call(n):
    return list(fresh(n))


def sorted_of_call(n):
    return sorted(fresh(n))


def sorted_name(xs):
    ys = sorted(xs)
    return ys is xs, xs, ys


def list_name(xs):
    ys = list(xs)
    return ys is xs, xs, ys


def sorted_kept(xs):
    # `keep` hands back the list it also stored: it is not the call's alone.
    ys = sorted(keep([x for x in xs]))
    return ys is KEPT[-1], KEPT[-1], ys


def list_kept(xs):
    ys = list(keep([x for x in xs]))
    ys.append("mine")
    return ys is KEPT[-1], KEPT[-1], ys


def sorted_attribute(holder):
    ys = sorted(holder.items)
    return ys is holder.items, holder.items, ys


def sorted_subscript(d, k):
    ys = sorted(d[k])
    return ys is d[k], d[k], ys


def sorted_method(d):
    return sorted(d.values()), sorted(d.keys()), sorted(d)


def sorted_subclass(xs):
    ys = sorted(Tracked(xs))
    return type(ys).__name__, ys


def list_subclass(xs):
    ys = list(Tracked(xs))
    return type(ys).__name__, ys


def sorted_tuple(xs):
    return sorted(tuple(xs)), list(tuple(xs)), sorted(set(xs)), list(str(len(xs)) * 3)


def negated(x):
    return -x


def mod3(x):
    return x % 3


def sorted_key(xs):
    return sorted([x for x in xs], key=negated)


def sorted_reverse(xs):
    return sorted([x for x in xs], reverse=True)


def sorted_key_reverse(xs):
    return sorted([x for x in xs], key=mod3, reverse=True)


def sorted_generator_key(xs):
    return sorted((x for x in xs), key=mod3)


def sorted_key_fails(xs):
    def key(x):
        if x == 3:
            raise KeyError("key of 3")
        return x

    return sorted([x for x in xs], key=key)


def sorted_compare_fails(ns):
    return sorted([Bad(n) for n in ns])


def sorted_fails_finalized(ns, log):
    # The list the call was given is destroyed as it was built, whatever
    # order the failed sort left its copy in.
    try:
        return sorted([Fin(n, log) for n in ns])
    except ValueError:
        log.append("caught ValueError")
    except TypeError:
        log.append("caught TypeError")
    return None


def sorted_mixed_finalized(ns, log):
    try:
        return sorted([Fin(n, log) if n % 2 else n for n in ns])
    except TypeError:
        log.append("caught TypeError")
    return None


def sorted_spied(ns, log):
    out = sorted([Spy(n, log) for n in ns])
    return out


def sorted_grabbed(ns):
    Grabber.taken.clear()
    out = sorted([Grabber(n) for n in ns])
    taken = list(Grabber.taken)
    Grabber.taken.clear()
    return out, taken, [t is out for t in taken]


def list_finalized(ns, log):
    made = list([Fin(n, log) for n in ns])
    log.append("made " + str(len(made)))
    return len(made)


def sorted_loud(ns):
    Loud.log.clear()
    out = sorted([Loud(n) for n in ns])
    return out, list(Loud.log)


def sorted_mixed(xs):
    return sorted([x for x in xs])


def twice(xs):
    first = sorted([x for x in xs])
    second = sorted(first)
    third = list(second)
    third.reverse()
    return first, second, third, first is second, second is third


def nested(d):
    return sorted([sorted([v for v in vs]) for vs in d.values()])


def identity(xs):
    made = [x for x in xs]
    same = list(made)
    return same is made, sorted(made) is made, made


def stable(pairs):
    return sorted([(p[0], i) for i, p in enumerate(pairs)])


def patched(xs):
    return sorted([x for x in xs]), list([x for x in xs]), tuple([x for x in xs])


class TupleSub(tuple):
    pass


class BadIter:
    def __init__(self, n):
        self.n = n

    def __iter__(self):
        return self

    def __next__(self):
        self.n -= 1
        if self.n < 0:
            raise RuntimeError("iteration failed")
        return self.n


def identities(xs):
    t = tuple(xs)
    l = list(xs)
    sub = TupleSub(xs)
    return (
        tuple(t) is t,
        list(l) is l,
        sorted(l) is l,
        type(tuple(sub)).__name__,
        tuple(sub) is sub,
        tuple(sub) == t,
        tuple(l) == t,
    )


def failing_tuple(n):
    return tuple(BadIter(n))


def failing_list(n):
    return list(BadIter(n))


def failing_sorted(n):
    return sorted(BadIter(n))


class Holder:
    def __init__(self, items):
        self.items = items


def shown(f, *args):
    try:
        result = f(*args)
    except BaseException as e:
        print(f.__name__, "raised", type(e).__name__, e.args)
    else:
        print(f.__name__, repr(result))


def main():
    xs = [5, 3, 9, 1, 3]
    for f in (sorted_comprehension, sorted_generator, list_comprehension, sorted_name, list_name,
              sorted_kept, list_kept, sorted_subclass, list_subclass, sorted_tuple, sorted_key, sorted_reverse,
              sorted_key_reverse, sorted_generator_key,
              sorted_key_fails, sorted_mixed, twice, identity):
        shown(f, xs)
        shown(f, [])
        shown(f, [2])
        shown(f, (4, 2, 8))
    print(xs, KEPT)
    KEPT.clear()
    shown(tuple_sorted_generator, {"b": 2, "a": 1, "c": 0})
    shown(tuple_sorted_generator, {})
    shown(tuple_sorted_generator, {"b": 2, 1: "a"})
    for n in (0, 1, 6, 12):
        shown(list_of_call, n)
        shown(sorted_of_call, n)
    shown(sorted_attribute, Holder([3, 1, 2]))
    shown(sorted_subscript, {"k": [3, 1, 2]}, "k")
    shown(sorted_method, {"b": 2, "a": 3})
    shown(sorted_compare_fails, [5, 1, 4])
    shown(sorted_compare_fails, [5, 3, 4])
    shown(sorted_compare_fails, [3])
    for ns in ([5, 1, 4, 2], [5, 1, 4, 2, 3], [5, 1, 3, 4, 2, 6], [3], [6, 9, 2, 8, 1, 7, 0, 5, 4, 3, 11, 10]):
        log = []
        shown(sorted_fails_finalized, ns, log)
        print(log)
    for ns in ([5, 1, 7], [5, 2, 1], [7, 1, 5, 3, 2], [9, 1, 7, 3, 5, 11, 4]):
        log = []
        shown(sorted_mixed_finalized, ns, log)
        print(log)
    log = []
    shown(sorted_spied, [4, 2, 5, 1, 3], log)
    print(log)
    shown(sorted_grabbed, [4, 2, 5, 1, 3])
    log = []
    shown(list_finalized, [1, 2, 5], log)
    print(log)
    shown(identities, [3, 1, 2])
    shown(identities, [])
    for f in (failing_tuple, failing_list, failing_sorted):
        shown(f, 3)
        shown(f, None)
    shown(sorted_loud, [3, 1, 2, 0])
    shown(sorted_mixed, [1, "a"])
    shown(sorted_mixed, [None, None])
    shown(sorted_mixed, [1.5, 1, True, 0])
    shown(sorted_mixed, ["b", "a", "B"])
    shown(sorted_mixed, 7)
    shown(list_comprehension, 7)
    shown(nested, {"x": [3, 1], "y": [2, 0], "z": []})
    shown(stable, [("b", 0), ("a", 0), ("b", 0), ("a", 0)])
    shown(patched, xs)


main()
