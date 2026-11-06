"""Record the inputs of the three functions during define+schema (REPEAT=5) in the plain tree."""
import sys, pickle, copy, os
HERE = os.path.dirname(os.path.abspath(__file__)); tree = sys.argv[1]
sys.path[:0] = [HERE, tree]
import pydantic, pydantic.json_schema as js, pydantic._internal._config as cfg
rec = {"dedup": [], "hash_total": 0, "getattr": [], "getattr_miss": 0}
o_dedup, o_hash, o_get = js._deduplicate_schemas, js._make_json_hashable, cfg.ConfigWrapper.__getattr__
def dedup(schemas):
    schemas = list(schemas)
    rec["dedup"].append(copy.deepcopy(schemas))
    return o_dedup(schemas)
def hsh(v):
    rec["hash_total"] += 1
    return o_hash(v)
def ga(self, name):
    try:
        r = o_get(self, name)
    except AttributeError:
        rec["getattr_miss"] += 1; raise
    rec["getattr"].append((self.config_dict, name))
    return r
js._deduplicate_schemas, js._make_json_hashable, cfg.ConfigWrapper.__getattr__ = dedup, hsh, ga
src = open(os.path.join(HERE, "..", "pydantic_workload.py")).read(); src = src[: src.index("\nmain()\n")]
g = {"__name__": "wl"}; sys.argv = ["wl", tree, "5", "schema"]
exec(compile(src, "wl", "exec"), g)
models, _ = g["timed"](g["define"]); g["timed"](g["schemas"], models)
ok = []
for cd, name in rec["getattr"]:
    try: pickle.dumps(cd); ok.append((cd, name))
    except Exception: pass
print("dedup calls", len(rec["dedup"]), "schemas", sum(map(len, rec["dedup"])), "hashable calls (incl. recursion)", rec["hash_total"],
      "getattr calls", len(rec["getattr"]), "picklable", len(ok), "getattr AttributeError", rec["getattr_miss"])
from collections import Counter; print(Counter(n for _, n in rec["getattr"]).most_common(8))
pickle.dump({"dedup": rec["dedup"], "getattr": ok}, open(os.path.join(HERE, "inputs.pkl"), "wb"))
