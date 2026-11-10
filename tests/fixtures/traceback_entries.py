# The traceback entries a compiled frame adds while an exception passes its
# `try` statements: one where it is raised or propagates into the frame,
# none for passing an unmatched handler, a bare `raise` or a `finally`
# clause, and one more for a `raise e` of the caught exception. Every case
# prints what CPython prints.


def fails(d, k):
    return d[k]


def unmatched(d, k):
    try:
        return d[k]
    except ValueError:
        return "no"


def unmatched_call(d, k):
    try:
        return fails(d, k)
    except ValueError:
        return "no"


def bare(d, k):
    try:
        return d[k]
    except KeyError:
        raise


def bare_after_work(d, k, log):
    try:
        return d[k]
    except KeyError:
        log.append("seen")
        raise


def named(d, k):
    try:
        return d[k]
    except KeyError as e:
        raise e


def replaced(d, k):
    try:
        return d[k]
    except KeyError:
        raise KeyError("replaced")


def through_finally(d, k, log):
    try:
        return d[k]
    finally:
        log.append("finally")


def unmatched_then_finally(d, k, log):
    try:
        return d[k]
    except ValueError:
        return "no"
    finally:
        log.append("finally")


def nested(d, k, log):
    try:
        try:
            return d[k]
        except ValueError:
            return "no"
    except IndexError:
        return "no"
    finally:
        log.append("finally")


def in_else(d, k):
    try:
        pass
    except ValueError:
        return "no"
    else:
        return d[k]


def in_loop(d, keys):
    for k in keys:
        try:
            d[k]
        except ValueError:
            continue
    return "done"


def entries(f, *args):
    try:
        f(*args)
    except KeyError as e:
        tb = e.__traceback__.tb_next
        out = []
        while tb is not None:
            out.append((tb.tb_frame.f_code.co_name, tb.tb_lineno - f.__code__.co_firstlineno))
            tb = tb.tb_next
        print(f.__name__, out)


def main():
    log = []
    for f in (unmatched, unmatched_call, bare, named, replaced, in_else):
        entries(f, {}, "k")
    for f in (bare_after_work, through_finally, unmatched_then_finally, nested):
        entries(f, {}, "k", log)
    entries(in_loop, {}, ["k"])
    print(log)


main()
