# A `try: d[k] except KeyError:` whose handler cannot see the exception
# takes a missing key of an exact dict straight into the handler
# (docs/design/compile-little.md, section 2.7). Every case prints what
# CPython prints.
import collections
import traceback
import types


class Sub(dict):
    pass


class Missing(dict):
    def __missing__(self, key):
        return "missing:" + str(key)


class MissingRaises(dict):
    def __missing__(self, key):
        raise KeyError("no " + str(key))


class Mapping:
    def __init__(self):
        self.asked = []

    def __getitem__(self, key):
        self.asked.append(key)
        if key == "a":
            return 1
        raise KeyError(key)


class BadHash:
    def __hash__(self):
        raise ValueError("no hash")


class BadEq:
    def __repr__(self):
        return "BadEq()"

    def __init__(self, error):
        self.error = error

    def __hash__(self):
        return 1

    def __eq__(self, other):
        raise self.error


class EqKey:
    def __hash__(self):
        return 1

    def __eq__(self, other):
        return False


class MyKeyError(KeyError):
    pass


class NoisyKey:
    # A key that reports when it is finalized.
    def __init__(self, name, log):
        self.name = name
        self.log = log

    def __hash__(self):
        return hash(self.name)

    def __eq__(self, other):
        return isinstance(other, NoisyKey) and self.name == other.name

    def __del__(self):
        self.log.append("del " + self.name)


def plain(d, k):
    try:
        return d[k]
    except KeyError:
        return "default"


def chained(first, second, k):
    try:
        return first[k]
    except KeyError:
        try:
            return second[k]
        except KeyError:
            raise AttributeError("no " + k) from None


def bound(d, k):
    try:
        return d[k]
    except KeyError as e:
        return "bound " + str(e)


def reraised(d, k):
    try:
        return d[k]
    except KeyError:
        raise


def reraised_nested(d, k):
    try:
        return d[k]
    except KeyError:
        if k == "x":
            raise
        return "kept"


def inner_reraise(d, k):
    # The bare `raise` belongs to the inner handler: the outer one cannot
    # see its own exception.
    try:
        return d[k]
    except KeyError:
        try:
            return int(k)
        except ValueError:
            raise


def tuple_handler(d, k):
    try:
        return d[k]
    except (KeyError, IndexError):
        return "tuple handler"


def tuple_handler_other(d, k):
    try:
        return d[k]
    except (ValueError, IndexError):
        return "wrong handler"


def exception_handler(d, k):
    try:
        return d[k]
    except Exception:
        return "Exception handler"


def bare_handler(d, k):
    try:
        return d[k]
    except:  # noqa: E722
        return "bare handler"


def lookup_first(d, k):
    try:
        return d[k]
    except LookupError:
        return "LookupError handler"
    except KeyError:
        return "KeyError handler"


def index_first(d, k):
    try:
        return d[k]
    except IndexError:
        return "IndexError handler"
    except KeyError:
        return "KeyError handler"


def subclass_first(d, k):
    try:
        return d[k]
    except MyKeyError:
        return "MyKeyError handler"
    except KeyError:
        return "KeyError handler"


def value_then_key(d, k):
    try:
        return d[k]
    except ValueError:
        return "ValueError handler"
    except KeyError:
        return "KeyError handler"


def with_else(d, k, log):
    try:
        v = d[k]
    except KeyError:
        log.append("handler")
        v = None
    else:
        log.append("else")
    return v


def with_finally(d, k, log):
    try:
        return d[k]
    except KeyError:
        log.append("handler")
        return "default"
    finally:
        log.append("finally")


def with_else_finally(d, k, log):
    try:
        v = d[k]
    except KeyError:
        log.append("handler")
        v = "default"
    else:
        log.append("else")
        v = d["missing in else"] if k == "boom" else v
    finally:
        log.append("finally")
    return v


def nested_inner_other(d, k, log):
    # The subscript's innermost `try` has no KeyError handler.
    try:
        try:
            return d[k]
        except ValueError:
            log.append("inner ValueError")
    except KeyError:
        log.append("outer KeyError")
    return "after"


def nested_inner_finally(d, k, log):
    try:
        try:
            return d[k]
        finally:
            log.append("inner finally")
    except KeyError:
        log.append("outer KeyError")
    return "after"


def nested_both(d, k, j, log):
    try:
        a = d[k]
        try:
            b = d[j]
        except KeyError:
            log.append("inner")
            b = 0
        return a + b
    except KeyError:
        log.append("outer")
        return -1


def in_loop(d, keys):
    out = []
    for k in keys:
        try:
            out.append(d[k])
        except KeyError:
            out.append("?")
            continue
        out.append("ok")
    return out


def loop_in_try(d, keys):
    seen = []
    try:
        for k in keys:
            seen.append(d[k])
    except KeyError:
        seen.append("stopped")
    return seen


def comprehension_in_try(d, keys):
    try:
        return [d[k] for k in keys]
    except KeyError:
        return "comprehension stopped"


def while_break(d, keys):
    i = 0
    total = 0
    while True:
        try:
            total += d[keys[i]]
        except KeyError:
            break
        i += 1
    return total, i


def temporaries(d, a, b):
    try:
        return str(d[a]) + "/" + str(d[b]) + "/" + str(len(d))
    except KeyError:
        return "temporaries dropped"


def call_arguments(d, a, b):
    try:
        return max(d[a], d[b], len(str(d)))
    except KeyError:
        return "arguments dropped"


def big_ints(d, a, b):
    try:
        return (10**30 + d[a]) * (10**25 + d[b])
    except KeyError:
        return "big ints dropped"


def augmented(d, k):
    try:
        d[k] += 1
    except KeyError:
        d[k] = 1
    return d[k]


def deleted(d, k):
    try:
        del d[k]
    except KeyError:
        return "nothing to delete"
    return "deleted"


def stored(d, k):
    try:
        d[k] = 1
    except KeyError:
        return "unreachable"
    return "stored"


def nested_subscript(d, k, j):
    try:
        return d[k][j]
    except KeyError:
        return "nested default"


def raised_elsewhere(d, k):
    try:
        return plain_raise(d, k)
    except KeyError:
        return "from a call"


def plain_raise(d, k):
    return d[k]


def handler_raises(d, k):
    try:
        return d[k]
    except KeyError:
        raise ValueError("from the handler") from None


def handler_fails(d, k):
    try:
        return d[k]
    except KeyError:
        return d[k]


def handler_mutates(d, k):
    try:
        return d[k]
    except KeyError:
        d[k] = len(d)
        return d[k]


def many_handlers(d, k):
    for _ in range(3):
        try:
            return d[k]
        except KeyError:
            k = k[1:]
    return "gave up"


def temporary_key(d, name, log):
    # The `KeyError` holds the key until the handler is left.
    try:
        return d[NoisyKey(name, log)]
    except KeyError:
        log.append("handler")
        return "default"


def dropped_key(d, name, log):
    # The handler drops the only other reference to the key.
    box = [NoisyKey(name, log)]
    try:
        return d[box[0]]
    except KeyError:
        box.clear()
        log.append("handler")
    log.append("after")
    return len(box)


def temporary_keys_in_loop(d, names, log):
    out = []
    for name in names:
        try:
            out.append(d[NoisyKey(name, log)])
        except KeyError:
            log.append("handler " + name)
            if name == "skip":
                continue
            if name == "stop":
                break
            out.append(None)
        log.append("next")
    return out


def temporary_key_finally(d, name, log):
    try:
        return d[NoisyKey(name, log)]
    except KeyError:
        log.append("handler")
        return "default"
    finally:
        log.append("finally")


def temporary_key_nested(first, second, name, log):
    try:
        return first[NoisyKey(name, log)]
    except KeyError:
        log.append("outer handler")
        try:
            return second[NoisyKey(name + "2", log)]
        except KeyError:
            log.append("inner handler")
        log.append("between")
    return "none"


def shown(f, *args):
    try:
        result = f(*args)
    except BaseException as e:
        print(f.__name__, "raised", type(e).__name__, e.args)
    else:
        print(f.__name__, repr(result))


def frames(f, *args):
    try:
        f(*args)
    except BaseException as e:
        rows = traceback.extract_tb(e.__traceback__)
        print(f.__name__, type(e).__name__, [(row.name, row.lineno) for row in rows[1:]])


def main():
    d = {"a": 1, "b": 2, (1, 2): "pair", None: "none", 3: "three"}
    for k in ("a", "zz", (1, 2), (3, 4), None, 3, 4, 3.0, True):
        shown(plain, d, k)
    shown(plain, {}, "a")

    # Other mappings take the ordinary path.
    shown(plain, Sub(a=1), "a")
    shown(plain, Sub(a=1), "b")
    shown(plain, Missing(a=1), "a")
    shown(plain, Missing(a=1), "b")
    shown(plain, MissingRaises(a=1), "b")
    shown(bound, MissingRaises(a=1), "b")
    mapping = Mapping()
    shown(plain, mapping, "a")
    shown(plain, mapping, "b")
    print(mapping.asked)
    shown(plain, collections.OrderedDict(a=1), "b")
    shown(plain, collections.defaultdict(list), "b")
    shown(plain, collections.ChainMap({"a": 1}, {"c": 3}), "c")
    shown(plain, collections.ChainMap({"a": 1}, {"c": 3}), "b")
    shown(plain, types.MappingProxyType({"a": 1}), "b")
    shown(plain, collections.Counter("aab"), "z")
    shown(plain, [1, 2], 5)
    shown(plain, [1, 2], 1)
    shown(plain, "text", "k")
    shown(plain, None, "k")
    shown(plain, 5, "k")

    # Failures that are not a missing key propagate.
    shown(plain, d, [1])
    shown(plain, d, {})
    shown(plain, d, BadHash())
    crowded = {EqKey(): 1}
    shown(plain, crowded, BadEq(ValueError("no eq")))
    shown(plain, crowded, BadEq(KeyError("eq says KeyError")))
    shown(bound, crowded, BadEq(KeyError("eq says KeyError")))
    shown(plain, crowded, BadEq(MyKeyError("eq says MyKeyError")))
    shown(plain, crowded, BadEq(IndexError("eq says IndexError")))

    shown(chained, {"a": 1}, {"b": 2}, "a")
    shown(chained, {"a": 1}, {"b": 2}, "b")
    shown(chained, {"a": 1}, {"b": 2}, "c")
    shown(chained, {"a": 1}, Missing(), "c")
    shown(chained, Sub(), {"c": 3}, "c")

    for f in (bound, reraised, reraised_nested, inner_reraise, tuple_handler, tuple_handler_other,
              exception_handler, bare_handler, lookup_first, index_first, subclass_first, value_then_key):
        for k in ("a", "x", "7"):
            shown(f, d, k)
        shown(f, d, [])
        shown(f, Sub(d), "x")
        shown(f, MissingRaises(), "x")
        shown(f, [0], 9)

    for f in (with_else, with_finally, with_else_finally, nested_inner_other, nested_inner_finally):
        for k in ("a", "x", "boom"):
            log = []
            shown(f, d, k, log)
            print(log)
        log = []
        shown(f, d, [], log)
        print(log)
    for k, j in (("a", "b"), ("a", "x"), ("x", "b"), ("x", "y")):
        log = []
        shown(nested_both, d, k, j, log)
        print(log)

    shown(in_loop, d, ["a", "x", "b", "y"])
    shown(loop_in_try, d, ["a", "b", "x", "a"])
    shown(loop_in_try, d, iter(["a", "x"]))
    shown(comprehension_in_try, d, ["a", "b"])
    shown(comprehension_in_try, d, ["a", "x", "b"])
    shown(while_break, d, ["a", "b", "a", "x", "b"])
    shown(while_break, d, ["a"])
    for a, b in (("a", "b"), ("a", "x"), ("x", "b")):
        shown(temporaries, d, a, b)
        shown(call_arguments, d, a, b)
        shown(big_ints, d, a, b)
    counts = {}
    for k in "abca":
        shown(augmented, counts, k)
    print(counts)
    shown(deleted, counts, "a")
    shown(deleted, counts, "a")
    shown(stored, counts, "new")
    shown(stored, counts, [])
    shown(nested_subscript, {"a": {"b": 1}}, "a", "b")
    shown(nested_subscript, {"a": {"b": 1}}, "a", "c")
    shown(nested_subscript, {"a": {"b": 1}}, "c", "b")
    shown(nested_subscript, {"a": Missing()}, "a", "c")
    shown(nested_subscript, {"a": [1]}, "a", 3)
    shown(raised_elsewhere, d, "a")
    shown(raised_elsewhere, d, "x")
    shown(handler_raises, d, "a")
    shown(handler_raises, d, "x")
    shown(handler_fails, d, "x")
    shown(handler_fails, d, [])
    grown = {}
    shown(handler_mutates, grown, "x")
    shown(handler_mutates, grown, "y")
    shown(handler_mutates, grown, "x")
    print(grown)
    shown(many_handlers, {"c": 1}, "abc")
    shown(many_handlers, {"c": 1}, "abcd")

    # The missing key lives as long as its `KeyError` would have.
    for f in (temporary_key, dropped_key, temporary_key_finally):
        log = []
        shown(f, d, "gone", log)
        print(log)
        log = []
        held = {NoisyKey("here", log): 1}
        shown(f, held, "here", log)
        print(log)
    log = []
    shown(temporary_keys_in_loop, d, ["x", "skip", "y", "stop", "z"], log)
    print(log)
    log = []
    shown(temporary_key_nested, {}, {}, "n", log)
    print(log)
    log = []
    shown(temporary_key_nested, Sub(), {}, "n", log)
    print(log)

    # Tracebacks of what a handler entered by a missing key raises.
    frames(handler_raises, d, "x")
    frames(handler_fails, d, "x")
    frames(chained, {}, {}, "c")

main()
