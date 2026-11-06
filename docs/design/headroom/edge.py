import sys; sys.path[:0] = [sys.argv[1], sys.argv[2]]
import pydantic.json_schema as js, pydantic._internal._config as cfg, hrfast
hrfast.setup(cfg, cfg.ConfigWrapper)
class D(dict):
    def items(self): return [("z", 1), ("a", [2, {"q": 3}])]
class L(list): pass
cases = [{"b": 1, "a": [1, {"x": None, "c": 2.5}], "t": True}, D(k=1), L([1, {"a": L([2])}]), "s", 3, None, [],
         {}, {1: "x", "a": "y"}, {"a": {1, 2}}, ({"a": 1},), {"a": [1, 2]}]
def outcome(f, *a):
    try: r = f(*a); return ("ok", r, type(r))
    except Exception as e: return ("exc", type(e), str(e))
for c in cases:
    o1, o2 = outcome(js._make_json_hashable, c), outcome(hrfast._make_json_hashable, c)
    assert o1 == o2, (c, o1, o2)
for sch in ([{"a": 1}, {"a": 1}, {"b": [1]}], ({"a": 1}, {"a": 1}), {"x": {"a": 1}, "y": {"a": 1}}.values(), [{"a": {1}}], [{1: 1, "a": 2}], 5):
    o1, o2 = outcome(js._deduplicate_schemas, sch), outcome(hrfast._deduplicate_schemas, sch)
    assert o1 == o2, (sch, o1, o2)
cw = cfg.ConfigWrapper({"title": "T"}, check=False)
for get in (lambda: cw.title, lambda: cw.strict, lambda: getattr(cw, "nope", "dflt")):
    a = get(); cfg.ConfigWrapper.__getattr__, saved = hrfast.instancemethod(hrfast.config_getattr), cfg.ConfigWrapper.__dict__["__getattr__"]
    b = get(); cfg.ConfigWrapper.__getattr__ = saved; assert a == b, (a, b)
def chain(e):
    out = []
    while e is not None:
        out.append((type(e).__name__, str(e), e.__cause__ is None, e.__suppress_context__)); e = e.__context__
    return out
def err():
    try: cw.nope
    except AttributeError as e: return chain(e)
c1 = err(); cfg.ConfigWrapper.__getattr__ = hrfast.instancemethod(hrfast.config_getattr); c2 = err()
assert c1 == c2, (c1, c2); print("edge cases identical:", c1)
