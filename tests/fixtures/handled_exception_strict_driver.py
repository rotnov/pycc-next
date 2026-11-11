# Calls each function of `handled_exception_strict.py`, built as a strict
# `--ext` module or run as source, and prints what it returned or raised,
# with the chain of contexts and causes and what is handled afterwards.
import gc
import sys


def chain(e):
    out = []
    while e is not None:
        cause = e.__cause__
        out.append((type(e).__name__, e.args, e.__suppress_context__,
                    None if cause is None else type(cause).__name__,
                    None if cause is None else repr(cause.__context__)))
        e = e.__context__
    return out


def run(m):
    calls = [
        ("handled_name", ()),
        ("miss_in_handler", (5,)), ("miss_in_handler", (1,)),
        ("tuple_miss_in_handler", ("b", 2)), ("tuple_miss_in_handler", ("a", 1)),
        ("str_miss_in_handler", ("k",)),
        ("raises_new", ("k",)), ("raises_from_none", ("k",)), ("raises_from_cause", ("k",)),
        ("raises_from_handled", ("k",)), ("raises_again_from_none", ()),
        ("raises_stored", ("k",)), ("raises_outer_in_inner", ("k",)),
        ("default_after_miss", ("k",)), ("default_after_miss", ("a",)),
        ("inner_sees_from_none", ()), ("inner_sees_new_cause", ()),
        ("self_reraise_from_self", ()),
    ]
    for name, args in calls:
        try:
            result = getattr(m, name)(*args)
        except BaseException as e:
            print(name, "raised", chain(e), repr(sys.exception()))
        else:
            print(name, repr(result), repr(sys.exception()))
    try:
        raise OSError("host")
    except OSError:
        try:
            m.miss_in_handler(7)
        except KeyError as e:
            print("from a host handler", chain(e))
    print("self-caused left alive:", left_alive(m.self_reraise_from_self))


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
