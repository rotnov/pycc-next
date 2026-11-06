"""replay.py TREE IMPL FUNC MODE
IMPL: native (the tree's own function: interpreted in plain, pycc-compiled in pyccbuild) | c (hrfast) | noop
FUNC: dedup | hashable | getattr      MODE: cg (one pass in hr_replay_region) | time (median of passes, ms)
Replays the exact inputs recorded during define+schema (REPEAT=5)."""
import sys, os, pickle, time, json, statistics
HERE = os.path.dirname(os.path.abspath(__file__))
tree, impl, func, mode = sys.argv[1:5]
sys.path[:0] = [HERE, tree]
import pydantic, pydantic.json_schema as js, pydantic._internal._config as cfg
import hrmark, hrfast
hrfast.setup(cfg, cfg.ConfigWrapper)
inp = pickle.load(open(os.path.join(HERE, "inputs.pkl"), "rb"))
if func == "dedup":
    calls = [(s,) for s in inp["dedup"]]
    f = {"native": js._deduplicate_schemas, "c": hrfast._deduplicate_schemas, "noop": hrmark.noop}[impl]
elif func == "hashable":   # the top-level calls dedup makes
    calls = [(x,) for s in inp["dedup"] for x in s]
    f = {"native": js._make_json_hashable, "c": hrfast._make_json_hashable, "noop": hrmark.noop}[impl]
else:
    wrappers = {}
    calls = []
    for cd, name in inp["getattr"]:
        k = id(cd)
        if k not in wrappers:
            wrappers[k] = cfg.ConfigWrapper(cd, check=False)
        calls.append((wrappers[k], sys.intern(name)))
    f = {"native": cfg.ConfigWrapper.__dict__["__getattr__"], "c": hrfast.config_getattr, "noop": hrmark.noop}[impl]
# correctness against the native function, every recorded input
if impl == "c":
    nat = {"dedup": js._deduplicate_schemas, "hashable": js._make_json_hashable,
           "getattr": cfg.ConfigWrapper.__dict__["__getattr__"]}[func]
    for a in calls:
        r1, r2 = nat(*a), f(*a)
        assert r1 == r2 and type(r1) is type(r2), (a, r1, r2)
        if func == "dedup":
            assert all(x is y for x, y in zip(r1, r2))
if mode == "cg":
    hrmark.replay(f, calls)   # warm-up outside the region? no: region counts this call only
    print(json.dumps({"calls": len(calls)}))
else:
    ts = []
    for _ in range(int(sys.argv[5]) if len(sys.argv) > 5 else 40):
        t0 = time.perf_counter(); hrmark.replay(f, calls); ts.append(time.perf_counter() - t0)
    print(json.dumps({"calls": len(calls), "median_ms": statistics.median(ts) * 1e3, "min_ms": min(ts) * 1e3}))
