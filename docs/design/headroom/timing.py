import subprocess, json, statistics, sys, os
# timing.py N PYTHON PLAIN_TREE PYCC_TREE: N interleaved fresh-process rounds of drive.py.
H = os.path.dirname(os.path.abspath(__file__))
N, P, PLAIN, PYCC = int(sys.argv[1]), sys.argv[2], sys.argv[3], sys.argv[4]
cfgs = {"a_cpython": (PLAIN, "0"), "b_cpython+C": (PLAIN, "1"), "c_pycc": (PYCC, "0")}
res = {k: {"define": [], "schema": []} for k in cfgs}
env = dict(os.environ, PYTHONHASHSEED="0")
for i in range(N):
    order = list(cfgs) if i % 2 == 0 else list(cfgs)[::-1]
    for k in order:
        tree, inj = cfgs[k]
        out = subprocess.run([P, "-I", f"{H}/drive.py", tree, "5", inj, "none"], capture_output=True, text=True, env=env, check=True).stdout
        d = json.loads(out.strip().splitlines()[-1])
        for ph in ("define", "schema"): res[k][ph].append(d[ph] * 1e3)
json.dump(res, open(f"{H}/timing.json", "w"))
for k in cfgs:
    for ph in ("define", "schema"):
        q = statistics.quantiles(res[k][ph], n=4)
        print(f"{k:14s} {ph:7s} median {statistics.median(res[k][ph]):6.2f} ms  IQR [{q[0]:6.2f}-{q[2]:6.2f}]  n={N}")
