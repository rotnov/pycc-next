import sys


def dumps(value):
    import json

    return json.dumps(value, sort_keys=True)


def joined(*parts):
    import os.path

    return os.path.join(*parts)


def leaf():
    import os.path as osp

    return osp.basename("/a/b/c.txt")


def names():
    from collections import OrderedDict, deque as dq

    d = OrderedDict()
    d["x"] = 1
    return list(d), list(dq([1, 2]))


def cached():
    import json

    first = json
    import json as again

    return first is again is sys.modules["json"]


def lazy():
    # Imported only when the function runs, and only once.
    before = "colorsys" in sys.modules
    import colorsys

    return before, "colorsys" in sys.modules, colorsys.rgb_to_hsv(1, 0, 0)


def missing():
    try:
        import json.pycc_no_such_module
    except ImportError as e:
        return "ImportError: " + str(e)
    return "imported", json.pycc_no_such_module


def missing_name():
    try:
        from json import no_such_name
    except ImportError as e:
        return "ImportError: " + str(e).split(" (")[0]
    return no_such_name


def conditional(flag):
    if flag:
        import json as m
    else:
        import pickle as m
    return m.__name__


class Box:
    def encode(self, v):
        import base64

        return base64.b64encode(v.encode()).decode()


def main():
    print(dumps({"b": 1, "a": [2, 3]}))
    print(joined("a", "b", "c"), leaf(), names(), cached())
    print(lazy())
    print(missing(), missing_name())
    print(conditional(True), conditional(False), Box().encode("hi"))
    from sys import pycc_no_such_name_at_all


main()
