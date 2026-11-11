# A compiled `except` handler publishes the exception it handles, as a
# CPython handler does: `sys.exception()`, `sys.exc_info()` and
# `traceback.format_exc()` see it while the handler runs, an exception
# raised in the handler gets it as its `__context__`, and it is gone once
# the handler is left. A dict miss that goes straight to its handler
# (docs/design/compile-little.md, section 2.7) makes its `KeyError` the first
# time something could see it. Every case prints what CPython prints.
import gc
import sys
import traceback


def seen():
    # What is being handled now.
    e = sys.exception()
    if e is None:
        return "nothing"
    return type(e).__name__ + repr(e.args)


def chain(e):
    # An exception with its contexts and what links them.
    out = []
    while e is not None:
        cause = e.__cause__
        out.append((type(e).__name__, e.args, e.__suppress_context__,
                    None if cause is None else type(cause).__name__,
                    None if cause is None else repr(cause.__context__)))
        e = e.__context__
    return out


def entries(e, base):
    out = []
    tb = e.__traceback__
    while tb is not None:
        out.append((tb.tb_frame.f_code.co_name, tb.tb_lineno - base.__code__.co_firstlineno))
        tb = tb.tb_next
    return out


def fails(message):
    raise ValueError(message)


def info(d, k):
    try:
        return d[k]
    except KeyError:
        kind, value, tb = sys.exc_info()
        return (kind.__name__, value.args, type(sys.exception()).__name__, value is sys.exception(),
                traceback.format_exc().splitlines()[-1], tb is value.__traceback__,
                entries(value, info))


def bound(d, k):
    try:
        return d[k]
    except KeyError as e:
        return str(e), seen()


def restored(d, k):
    before = seen()
    during = None
    try:
        d[k]
    except KeyError:
        during = seen()
    after = seen()
    return before, during, after


def nested(d, k):
    out = []
    try:
        d[k]
    except KeyError:
        out.append(seen())
        try:
            int(k)
        except ValueError:
            out.append(seen())
            try:
                [][1]
            except IndexError:
                out.append(seen())
            out.append(seen())
        out.append(seen())
    out.append(seen())
    return out


def in_loop(d, keys):
    out = []
    for k in keys:
        try:
            out.append(d[k])
        except KeyError:
            out.append(seen())
            if k == "stop":
                break
            continue
        out.append(seen())
    out.append(seen())
    return out


def returns_from_handler(d, k):
    try:
        return d[k]
    except KeyError:
        return seen()


def after_return(d, k):
    return returns_from_handler(d, k), seen()


def raises_new(d, k):
    try:
        return d[k]
    except KeyError:
        raise ValueError("new")


def raises_object(d, k, make):
    try:
        return d[k]
    except KeyError:
        raise make("object")


def raises_from_none(d, k):
    try:
        return d[k]
    except KeyError:
        raise ValueError("no context shown") from None


def raises_from_cause(d, k):
    try:
        return d[k]
    except KeyError:
        raise ValueError("caused") from IndexError("the cause")


def raises_from_bound_cause(d, k):
    try:
        return d[k]
    except KeyError:
        try:
            raise IndexError("raised first")
        except IndexError as cause:
            raise ValueError("caused") from cause


def raises_again_from_none(d, k):
    # `raise e from None` drops the cause an earlier `raise ... from` gave.
    try:
        raise ValueError("new") from IndexError("old")
    except ValueError as e:
        raise e from None


def handled_links():
    # The cause and the `from None` mark of what is being handled now.
    e = sys.exception()
    return repr(e.__cause__), e.__suppress_context__


def inner_sees_from_none(d, k):
    # `raise e from None` seen by the handler that catches it: no cause.
    try:
        raise ValueError("new") from IndexError("old")
    except ValueError as e:
        try:
            raise e from None
        except ValueError:
            return handled_links()


def inner_sees_new_cause(d, k):
    # `raise e from X` seen by the handler that catches it: the new cause.
    try:
        raise ValueError("new") from IndexError("old")
    except ValueError as e:
        try:
            raise e from KeyError("other")
        except ValueError:
            return handled_links()


def self_reraise_from_self():
    try:
        raise ValueError("v")
    except ValueError as e:
        raise e from e


def raises_stored(d, k):
    # A stored exception raised in another handler gets that one as context.
    try:
        return d[k]
    except KeyError as e:
        first = e
    try:
        raise ValueError("second")
    except ValueError:
        raise first


def raises_outer_in_inner(d, k):
    # The outer handled exception raised in an inner handler: the inner one
    # becomes its context, and the inner one's link back to it is cut.
    try:
        return d[k]
    except KeyError as a:
        try:
            raise ValueError("b")
        except ValueError:
            raise a


def raises_caught_from_cpython(d, k):
    # Caught from a CPython callee, stored, raised in another handler.
    try:
        int("x")
    except ValueError as e:
        first = e
    try:
        return d[k]
    except KeyError:
        raise first


def raises_from_handled(d, k):
    try:
        return d[k]
    except KeyError as e:
        raise ValueError("caused by the handled one") from e


def reraises(d, k):
    try:
        return d[k]
    except KeyError:
        raise


def reraises_named(d, k):
    try:
        return d[k]
    except KeyError as e:
        raise e


def callee_raises(d, k):
    try:
        return d[k]
    except KeyError:
        return fails("from a call")


def operation_raises(d, k):
    try:
        return d[k]
    except KeyError:
        return [][1]


def caught_inside(d, k):
    # The exception the handler's own `try` catches has the handled one as
    # its context, and the handled one is back once that handler is left.
    try:
        return d[k]
    except KeyError:
        try:
            fails("inner")
        except ValueError:
            inner = seen()
        return inner, seen()


def three_deep(d, k):
    try:
        return d[k]
    except KeyError:
        try:
            raise ValueError("second")
        except ValueError:
            raise IndexError("third")


def raises_saved(d, k):
    # An exception made before the handler, raised in it.
    saved = ValueError("made earlier")
    try:
        return d[k]
    except KeyError:
        raise saved


def raises_with_key(d, log):
    # The handled `KeyError` is the context of what the handler raises, so
    # the key it holds lives as long as that does.
    try:
        try:
            return d[Watcher(log)]
        except KeyError:
            return [][1]
    except IndexError:
        log.append("caught IndexError")
        return "handled"


def handler_in_callee(d, k):
    # `raises_new` handles its own `KeyError`; this handler's exception is
    # another one.
    try:
        return int("x")
    except ValueError:
        return raises_new(d, k)


def config(first, second, name):
    # pydantic's `ConfigWrapper.__getattr__`.
    try:
        return first[name]
    except KeyError:
        try:
            return second[name]
        except KeyError:
            raise AttributeError(f"Config has no attribute {name!r}") from None


class Probe:
    # A key that shares a `str`'s hash and reports what is being handled
    # when a lookup of that `str` compares the two.
    def __init__(self, name, log):
        self.name = name
        self.log = log

    def __hash__(self):
        return hash(self.name)

    def __eq__(self, other):
        self.log.append("compared while handling " + seen())
        return False


class Watcher:
    # Reports what is being handled when it is finalized.
    def __init__(self, log):
        self.log = log

    def __del__(self):
        self.log.append("finalized while handling " + seen())


def default_seen(d, k):
    try:
        return d[k]
    except KeyError:
        return "default"


def temporary_in_handler(d, k, log):
    try:
        return d[k]
    except KeyError:
        return len([Watcher(log)])


def exception_type(d, k):
    try:
        return d[k]
    except Exception:
        return seen()


def bare_handler(d, k):
    try:
        return d[k]
    except:  # noqa: E722
        return seen()


def second_handler(d, k):
    try:
        return d[k]
    except IndexError:
        return "IndexError"
    except KeyError:
        return seen()


class Mapping:
    def __getitem__(self, key):
        raise KeyError("mapping " + str(key))


def shown(f, *args):
    try:
        result = f(*args)
    except BaseException as e:
        print(f.__name__, "raised", chain(e), entries(e, f)[1:], seen())
    else:
        print(f.__name__, repr(result), seen())


def main():
    for f in (info, bound, restored, nested, returns_from_handler, after_return, raises_new,
              raises_from_none, raises_from_cause, raises_from_bound_cause, raises_again_from_none,
              raises_stored, raises_outer_in_inner, raises_caught_from_cpython, raises_from_handled,
              inner_sees_from_none, inner_sees_new_cause,
              reraises, reraises_named,
              callee_raises, operation_raises, caught_inside, three_deep, raises_saved,
              handler_in_callee,
              default_seen, exception_type, bare_handler, second_handler):
        for d, k in (({}, "k"), ({"k": 1}, "k"), ({}, 5), ({}, ("a", 1)), (Mapping(), "k")):
            shown(f, d, k)
    shown(raises_object, {}, "k", RuntimeError)
    shown(raises_object, {}, "k", fails)
    shown(in_loop, {"a": 1}, ["a", "b", "stop", "c"])
    shown(in_loop, Mapping(), ["a", "stop"])
    shown(config, {"a": 1}, {}, "a")
    shown(config, {}, {"a": 2}, "a")
    shown(config, {}, {}, "a")
    shown(config, Mapping(), {"a": 2}, "a")
    shown(config, {}, Mapping(), "a")
    shown(config, {}, {}, 5)
    shown(config, {}, {}, [1])
    for first, second in (({}, {"b": 2}), ({}, {"a": 2}), ({"b": 1}, {"a": 2})):
        log = []
        probed = first if first else second
        probed[Probe("a", log)] = 1
        log.clear()
        shown(config, first, second, "a")
        # How often a lookup compares the two depends on the process's
        # hash key (a probe may come back to the same slot).
        print(sorted(set(log)))
    for d in ({}, Mapping()):
        log = []
        shown(temporary_in_handler, d, "k", log)
        print(log)
    log = []
    shown(raises_with_key, {}, log)
    print(log)
    # An exception that is its own cause is freed with its handler.
    print("self-caused left alive:", left_alive(self_reraise_from_self))


def left_alive(f):
    # How many `ValueError`s 1000 calls of `f` leave alive.
    def live():
        gc.collect()
        return sum(1 for o in gc.get_objects() if type(o) is ValueError)
    before = live()
    for _ in range(1000):
        try:
            f()
        except ValueError:
            pass
    return live() - before


main()
