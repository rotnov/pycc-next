# Every operation on CPython objects that fails inside a compiled generic body
# (`--ext --non-strict --select all`) is caught by a handler in the same
# function, or reaches the caller, as in CPython: each operation routes its
# own failure (`foreign_fail`), with no pending-exception check after it.


class Bad:
    def __len__(self):
        raise ValueError("len")

    def __bool__(self):
        raise ValueError("bool")

    def __contains__(self, item):
        raise ValueError("contains")

    def __getitem__(self, key):
        raise ValueError("getitem")

    def __iter__(self):
        raise ValueError("iter")

    def __lt__(self, other):
        raise ValueError("lt")

    def __eq__(self, other):
        raise ValueError("eq")

    def __hash__(self):
        raise ValueError("hash")

    def __neg__(self):
        raise ValueError("neg")

    def __add__(self, other):
        raise ValueError("add")

    def __call__(self, *args, **kwargs):
        raise ValueError("call")

    def method(self):
        raise ValueError("method")


def attr(o):
    try:
        v = o.missing
        print("attr no error", v)
    except AttributeError:
        print("attr AttributeError")
    return "after"


def method(o):
    try:
        v = o.method()
        print("method no error", v)
    except ValueError as e:
        print("method", e)
    try:
        v = o.nothing()
        print("method no error", v)
    except AttributeError:
        print("method AttributeError")
    return "after"


def call(o, f):
    try:
        v = o(1)
        print("call no error", v)
    except ValueError as e:
        print("call", e)
    try:
        v = o(key=1)
        print("keyword call no error", v)
    except ValueError as e:
        print("keyword call", e)
    try:
        v = f(1)
        print("call no error", v)
    except TypeError as e:
        print("call", e)
    return "after"


def length(o):
    try:
        n = len(o)
        print("len no error", n)
    except ValueError as e:
        print("len", e)
    try:
        if o:
            print("truth no error")
    except ValueError as e:
        print("truth", e)
    return "after"


def subscript(o, d, xs, s):
    for key in ("k", 1):
        try:
            v = o[key]
            print("subscript no error", v)
        except ValueError as e:
            print("subscript", e)
    try:
        v = d["absent"]
        print("subscript no error", v)
    except KeyError as e:
        print("subscript KeyError", e)
    try:
        v = xs[10]
        print("subscript no error", v)
    except IndexError as e:
        print("subscript", e)
    try:
        v = s[99]
        print("subscript no error", v)
    except IndexError as e:
        print("subscript", e)
    try:
        v = s["x"]
        print("subscript no error", v)
    except TypeError as e:
        print("subscript", e)
    try:
        v = xs[1:3:0]
        print("slice no error", v)
    except ValueError as e:
        print("slice", e)
    return "after"


def unpack(o, xs):
    try:
        a, b = xs
        print("unpack no error", a, b)
    except ValueError as e:
        print("unpack", e)
    try:
        a, b = o
        print("unpack no error", a, b)
    except ValueError as e:
        print("unpack", e)
    return "after"


def compare(o, n, s):
    try:
        print("isinstance no error", isinstance(o, n))
    except TypeError as e:
        print("isinstance", e)
    try:
        print("in no error", 1 in o)
    except ValueError as e:
        print("in", e)
    try:
        print("in no error", [] in {1, 2})
    except TypeError as e:
        print("in", e)
    try:
        print("in no error", 1 in n)
    except TypeError as e:
        print("in", e)
    try:
        print("lt no error", o < n)
    except ValueError as e:
        print("lt", e)
    try:
        print("lt no error", s < n)
    except TypeError as e:
        print("lt", e)
    try:
        print("eq no error", o == s)
    except ValueError as e:
        print("eq", e)
    print("is", o is n, o is o)
    return "after"


def arith(o, n, s):
    try:
        print("add no error", o + n)
    except ValueError as e:
        print("add", e)
    try:
        print("add no error", s + n)
    except TypeError as e:
        print("add", e)
    try:
        print("div no error", n // 0)
    except ZeroDivisionError as e:
        print("div", e)
    try:
        n += s
        print("iadd no error", n)
    except TypeError as e:
        print("iadd", e)
    try:
        print("neg no error", -o)
    except ValueError as e:
        print("neg", e)
    try:
        print("neg no error", -s)
    except TypeError as e:
        print("neg", e)
    return "after"


def display(o, xs):
    try:
        d = {o: 1}
        print("dict display no error", d)
    except ValueError as e:
        print("dict display", e)
    try:
        st = {1, xs}
        print("set display no error", st)
    except TypeError as e:
        print("set display", e)
    return "after"


def nested(o, d):
    # A failing operand of another operation: nothing after it runs.
    try:
        print("nested no error", len(d["absent"]) + o.missing)
    except KeyError as e:
        print("nested KeyError", e)
    try:
        print("nested no error", [x for x in (d, o) if x["k"]])
    except KeyError as e:
        print("nested KeyError", e)
    except ValueError as e:
        print("nested ValueError", e)
    return "after"


def uncaught(o):
    # No handler here: the failure leaves the function, after the caller's
    # arguments and before anything else in it runs.
    v = o.method()
    print("uncaught no error", v)
    return v


def main():
    o = Bad()
    d = {"k": 0}
    xs = [1, 2, 3]
    print(attr(o))
    print(method(o))
    print(call(o, 5))
    print(length(o))
    print(subscript(o, d, xs, "text"))
    print(unpack(o, xs))
    print(compare(o, 3, "text"))
    print(arith(o, 3, "text"))
    print(display(o, xs))
    print(nested(o, d))
    try:
        uncaught(o)
    except ValueError as e:
        print("uncaught", e)


if __name__ == "__main__":
    main()
