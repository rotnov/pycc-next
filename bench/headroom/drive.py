"""Driver: drive.py TREE REPEAT INJECT(0/1) MARK(none|define|schema|both) [DUMP]
Runs the workload's define/schema phases. INJECT=1 swaps in hand-written C (hrfast).
MARK wraps the chosen phases in hrmark.run so callgrind can toggle collection."""
import sys, time, json, os
HERE = os.path.dirname(os.path.abspath(__file__))
tree, REPEAT, INJECT, MARK = sys.argv[1], int(sys.argv[2]), sys.argv[3] == "1", sys.argv[4]
DUMP = sys.argv[5] if len(sys.argv) > 5 else None
src = open(os.path.join(HERE, "..", "pydantic_workload.py")).read()
src = src[: src.index("\nmain()\n")]
sys.path.insert(0, HERE)
import hrmark
if INJECT:
    sys.path.insert(0, tree); import pydantic  # noqa
    import inject; inject.install()
g = {"__name__": "wl"}
sys.argv = ["wl", tree, str(REPEAT), "schema"]
exec(compile(src, "wl", "exec"), g)
times = g["times"]

def run(name, fn):
    out = {}
    def body():
        out["r"] = g["timed"](fn, *([out_models["m"]] if name == "schema" else []))
    if MARK in (name, "both"):
        hrmark.run(body)
    else:
        body()
    return out["r"]
out_models = {}
models, times["define"] = run("define", g["define"])
out_models["m"] = models
_, times["schema"] = run("schema", g["schemas"])
if DUMP:
    res = []
    for m in models[:4]:
        res.append(m.model_json_schema()); res.append(m.model_json_schema(mode="serialization"))
    json.dump(res, open(DUMP, "w"), sort_keys=True, indent=1, default=repr)
print(json.dumps(times))
