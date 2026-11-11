"""Experiment: build-time metrics and calibration candidates, repeated.

Usage: python3 calib.py PYCC PKGDIR REPS OUT.json [--load N]
PKGDIR: a copy of the cpython tree of diff-match-patch (package dir inside).
"""
import json, os, resource, shutil, subprocess, sys, tempfile, time, pathlib, multiprocessing


def c_source(n: int = 160) -> str:
    out = ["#include <string.h>", "struct P { int x, y; double w; char tag[16]; };"]
    for k in range(n):
        callee = f"f{k - 1}(a, n / 2 + 1, acc, p)" if k else "0"
        out.append(f"""
int f{k}(int *a, int n, int s, struct P *p) {{
    int acc = s + {k};
    for (int i = 0; i < n; i++) {{
        switch ((a[i] + {k}) & 7) {{
        case 0: acc += a[i] * {k + 3}; break;
        case 1: acc -= a[(i + {k}) % n] >> 1; break;
        case 2: acc ^= (int)(p[i % 4].w * {k + 1}); break;
        case 3: p[i % 4].x += acc & {k | 1}; break;
        case 4: acc += (int)strlen(p[i % 4].tag) * {k}; break;
        default: acc = acc * 31 + a[i];
        }}
        for (int j = i; j < n && j < i + {k % 5 + 2}; j++)
            a[j] = (a[j] + acc) % {k + 97};
        if (acc > {k * 100 + 7}) acc -= p[(i + 1) % 4].y;
    }}
    return acc + {callee};
}}""")
    return "\n".join(out)


def cpu_children() -> float:
    r = resource.getrusage(resource.RUSAGE_CHILDREN)
    return r.ru_utime + r.ru_stime


def timed(cmd, **kw):
    c0, t0 = cpu_children(), time.perf_counter()
    subprocess.run(cmd, check=True, capture_output=True, **kw)
    return time.perf_counter() - t0, cpu_children() - c0


PY_LOOP = "s=0\nfor i in range(6_000_000):\n    s=(s*31+i)%1000003\n"


def burn():
    while True:
        pass


def main():
    pycc, pkg, reps, out = sys.argv[1], pathlib.Path(sys.argv[2]), int(sys.argv[3]), sys.argv[4]
    load = int(sys.argv[6]) if len(sys.argv) > 6 and sys.argv[5] == "--load" else 0
    procs = [multiprocessing.Process(target=burn, daemon=True) for _ in range(load)]
    for p in procs:
        p.start()
    tmp = pathlib.Path(tempfile.mkdtemp())
    src = tmp / "ref.c"
    src.write_text(c_source())
    rows = []
    for r in range(reps):
        work = tmp / f"w{r}"
        shutil.copytree(pkg, work)
        d = work / "diff_match_patch"
        pre = [timed(["cc", "-O2", "-c", str(src), "-o", str(tmp / "ref.o")]) for _ in range(3)]
        bw, bc = timed([pycc, "build", str(d), "-o", str(d / "__init__"), "--ext", "--non-strict",
                        "--release", "--select", "all", "--exclude", "tests"])
        ccs = [timed(["cc", "-O2", "-c", str(src), "-o", str(tmp / "ref.o")]) for _ in range(3)]
        ccs = pre + ccs
        cw, cc = min(c[0] for c in ccs), min(c[1] for c in ccs)
        pw, pc = timed([sys.executable, "-I", "-S", "-c", PY_LOOP])
        row = dict(build_wall=bw, build_cpu=bc, cc_wall=cw, cc_cpu=cc, py_wall=pw, py_cpu=pc,
                   cc_all=ccs)
        rows.append(row)
        print({k: round(v, 3) for k, v in row.items() if k != 'cc_all'}, flush=True)
        shutil.rmtree(work)
    pathlib.Path(out).write_text(json.dumps(dict(load=load, rows=rows, cpu=open("/proc/cpuinfo").read().split("model name")[1].split("\n")[0], ncpu=os.cpu_count(), cc=subprocess.run(["cc", "--version"], capture_output=True, text=True).stdout.split("\n")[0])))
    for p in procs:
        p.terminate()


if __name__ == "__main__":
    main()
