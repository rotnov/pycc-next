# A value released while an exception unwinds may run compiled code: a
# `__del__`, a generator's `finally`. CPython holds the exception outside
# the thread's state meanwhile, so that code neither sees it nor loses it,
# and the handler it is propagating to still runs. Every case prints what
# CPython prints.


class Fin:
    # Reports when it is finalized; the finalizer is compiled.
    def __init__(self, name, log):
        self.name = name
        self.log = log

    def __lt__(self, other):
        return self.name < other.name

    def __hash__(self):
        return hash(self.name)

    def __eq__(self, other):
        return isinstance(other, Fin) and self.name == other.name

    def __del__(self):
        self.log.append("del " + self.name)


class Busy:
    # A finalizer that raises and handles an exception of its own.
    def __init__(self, name, log):
        self.name = name
        self.log = log

    def __del__(self):
        try:
            self.log.append("del " + self.name + " " + str({}[self.name]))
        except KeyError:
            self.log.append("del " + self.name + " handled its own KeyError")


class Named:
    def __init__(self, name):
        self.name = name

    def __lt__(self, other):
        return self.name < other.name


def sorted_mixed():
    # `3 < Named("b")` has no answer.
    return sorted([Named("b"), 3, Named("a")])


def sorted_finalized(log):
    # The elements are finalized while the `TypeError` propagates.
    try:
        return sorted([Fin("b", log), 3, Fin("a", log)])
    except TypeError:
        log.append("caught TypeError")
        return "handled"


def sorted_finalized_escapes(log):
    return sorted([Fin("b", log), 3, Fin("a", log)])


def nested_handler_raises(d, k):
    try:
        try:
            return d[k]
        except KeyError:
            return [][1]
    except ValueError:
        return "ValueError"


def nested_handler_reraises(d, k):
    try:
        try:
            return d[k]
        except KeyError:
            raise
    except ValueError:
        return "ValueError"


def handler_raises_with_key(d, log):
    # The handler holds the missing key, which is finalized while the
    # handler's own `IndexError` leaves it.
    try:
        return d[Fin("key", log)]
    except KeyError:
        return [][1]


def local_released(log):
    # A local is finalized when the frame an exception leaves is dropped.
    held = Fin("local", log)
    return [held][1]


def temporary_released(log):
    try:
        return len([Fin("temporary", log)]) + [][1]
    except IndexError:
        log.append("caught IndexError")
        return "handled"


def busy_finalizer(log):
    try:
        return [Busy("busy", log)][1]
    except IndexError:
        log.append("caught IndexError")
        return "handled"


def finally_released(log):
    try:
        try:
            return [Fin("in try", log)][1]
        finally:
            log.append("finally")
    except IndexError:
        log.append("caught IndexError")
        return "handled"


def shown(f, *args):
    try:
        result = f(*args)
    except BaseException as e:
        print(f.__name__, "raised", type(e).__name__, e.args)
    else:
        print(f.__name__, repr(result))


def main():
    shown(sorted_mixed)
    for f in (sorted_finalized, sorted_finalized_escapes, local_released, temporary_released,
              busy_finalizer, finally_released):
        log = []
        shown(f, log)
        print(log)
    for f in (nested_handler_raises, nested_handler_reraises):
        shown(f, {}, "k")
        shown(f, {"k": 1}, "k")
    log = []
    shown(handler_raises_with_key, {}, log)
    print(log)


main()
