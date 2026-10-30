"""Combine the benchmark JSONs into the published result of the CI benchmark workflow.

Usage: python3 bench/publish.py [--run run.json] [--packages packages.json]
                                [--pydantic pydantic.json] --json OUT.json --md OUT.md

The inputs are the `--json` outputs of `bench/run.py`, `bench/package_check.py --time`
and `bench/pydantic_check.py --time`; any may be missing (its section is then left
out). Writes one JSON document (the inputs plus the machine: CPU model, logical CPUs,
OS, CPython, commit, and the Actions run when run there) and a Markdown report: per
program, package workload and pydantic phase, the median and min-max of each side over
the stated number of interleaved fresh-process runs, and the ratio of the medians.
The machine section comes first: a shared CI runner's CPU varies between runs, and
the numbers mean only what that CPU and the spread say. When the inputs hold the
competitors' results (`--competitors`, `bench/competitors.py`), the report has
Cython and mypyc columns (time, CPython over it, and pycc-next against it), their
versions and flags, the mode of every Cython number, and every build that did not
compile, pass the suite or match CPython's output, with why.
"""
import argparse
import json
import os
import pathlib
import platform
import subprocess
import sys


def cpu_model() -> str:
    # lscpu names Arm cores (e.g. Neoverse-N2), which /proc/cpuinfo does not.
    try:
        r = subprocess.run(["lscpu"], capture_output=True, text=True)
        for line in r.stdout.splitlines():
            if line.startswith("Model name:"):
                return line.split(":", 1)[1].strip()
    except OSError:
        pass
    try:
        for line in pathlib.Path("/proc/cpuinfo").read_text().splitlines():
            if line.startswith(("model name", "Model")):
                return line.split(":", 1)[1].strip()
    except OSError:
        pass
    if sys.platform == "darwin":
        r = subprocess.run(["sysctl", "-n", "machdep.cpu.brand_string"],
                           capture_output=True, text=True)
        if r.returncode == 0:
            return r.stdout.strip()
    return platform.processor() or "unknown"


def machine() -> dict[str, str]:
    env = os.environ
    commit = env.get("GITHUB_SHA") or subprocess.run(
        ["git", "rev-parse", "HEAD"], capture_output=True, text=True).stdout.strip()
    m = {
        "cpu": cpu_model(),
        "logical_cpus": str(os.cpu_count()),
        "os": f"{platform.system()} {platform.release()} {platform.machine()}",
        "runner_image": f"{env.get('ImageOS', '')} {env.get('ImageVersion', '')}".strip(),
        "python": platform.python_version(),
        "commit": commit,
    }
    if env.get("GITHUB_RUN_ID"):
        m["run"] = (f"{env.get('GITHUB_SERVER_URL', 'https://github.com')}/"
                    f"{env['GITHUB_REPOSITORY']}/actions/runs/{env['GITHUB_RUN_ID']}")
        m["run_id"] = env["GITHUB_RUN_ID"]
        m["runner"] = env.get("RUNNER_NAME", "")
    return m


def cell(s: dict[str, float], scale: float = 1.0, unit: str = "s", digits: int = 3) -> str:
    return (f"{s['median'] * scale:.{digits}f}{unit} "
            f"({s['min'] * scale:.{digits}f}-{s['max'] * scale:.{digits}f})")


def vs(other: dict[str, float] | None, pycc: dict[str, float]) -> str:
    """pycc-next against another side, from the ratio of the medians."""
    if other is None:
        return "-"
    r = other["median"] / pycc["median"]
    return f"{r:.2f}x faster" if r >= 1 else f"{1 / r:.2f}x slower"


def competitor_cell(t: dict[str, float] | None, cpython: dict[str, float], **kw) -> str:
    """A competitor's time and, after it, CPython's median over its median."""
    if t is None:
        return "-"
    return f"{cell(t, **kw)}, {cpython['median'] / t['median']:.2f}x"


def competitors_section(versions: dict | None) -> list[str]:
    if not versions:
        return []
    return ["## Competitors", "",
            "CPython is the correctness oracle; Cython and mypyc are competitors, timed "
            "with the same method and workloads on the same unmodified sources "
            "(`bench/competitors.py`). A competitor's cell is its median (min-max) and "
            "CPython's median over its median; \"pycc vs\" is the competitor's median "
            "over pycc-next's. A build that does not compile, fails the package's own "
            "suite or prints other output than CPython is listed, not timed.", "",
            f"- Cython {versions.get('cython')}: `{versions.get('cython_flags')}` "
            f"(setuptools {versions.get('setuptools')})",
            f"- mypyc (mypy {versions.get('mypy')}): `{versions.get('mypyc_flags')}`", ""]


def markdown(doc: dict) -> str:
    m = doc["machine"]
    out = ["# pycc-next benchmarks", ""]
    out += [f"- {k}: {v}" for k, v in m.items() if v]
    out += ["", "Each cell: median (min-max) over the runs given; a run is a fresh process, "
            "the sides interleaved round by round. Ratios are of the medians; above 1, "
            "pycc-next is faster.", ""]
    versions = (doc.get("run", {}).get("competitors")
                or doc.get("packages", {}).get("competitors", {}).get("versions"))
    out += competitors_section(versions)
    if "run" in doc:
        progs = doc["run"]["programs"]
        runs = next((p["runs"] for p in progs if "runs" in p), 0)
        out += [f"## Programs (`bench/run.py`, {runs} runs, process start included)", "",
                "Cython in its default mode (the annotations honored); with annotations "
                "ignored below.", "",
                "| program | CPython 3.13 | pycc-next | Cython | mypyc | Rust (`-O`) | "
                "vs CPython | vs Cython | vs mypyc | vs Rust |",
                "|---|---|---|---|---|---|---|---|---|---|"]
        notes, modes = [], []
        for p in progs:
            if "error" in p:
                out.append(f"| {p['program']} | | {p['error']} | | | | | | | |")
                continue
            t = p["seconds"]
            py, pc, rs = t["cpython"], t["pycc"], t.get("rust")
            comp = {side: c.get("seconds") for side, c in p.get("competitors", {}).items()}
            name = p["program"] + (" (kernel)" if p["kernel"] else "")
            bad = "" if p["output_matches"] else " (OUTPUT MISMATCH)"
            vs_rs = vs(rs, pc)
            out.append(f"| {name} | {cell(py)} | {cell(pc)}{bad} | "
                       f"{competitor_cell(comp.get('cython'), py)} | "
                       f"{competitor_cell(comp.get('mypyc'), py)} | "
                       f"{cell(rs) if rs else '-'} | "
                       f"**{py['median'] / pc['median']:.1f}x faster** | "
                       f"{vs(comp.get('cython'), pc)} | {vs(comp.get('mypyc'), pc)} | {vs_rs} |")
            if "cython" in comp or "cython_untyped" in comp:
                modes.append(f"| {name} | {competitor_cell(comp.get('cython'), py)} | "
                             f"{competitor_cell(comp.get('cython_untyped'), py)} |")
            for side, c in p.get("competitors", {}).items():
                if "error" in c:
                    notes.append(f"- {p['program']}: {side} ({c['mode']}): {c['error']}")
        out.append("")
        if modes:
            out += ["Cython's two modes (`annotation_typing`):", "",
                    "| program | default (annotations honored) | annotations ignored |",
                    "|---|---|---|", *modes, ""]
        if notes:
            out += ["Not timed:", "", *notes, ""]
    if "packages" in doc:
        out += ["## Real packages (`bench/package_check.py --time`)", "",
                "| package | workload | runs | CPython | pycc-next | Cython | mypyc | "
                "CPython / pycc-next | pycc vs Cython | pycc vs mypyc |",
                "|---|---|---|---|---|---|---|---|---|---|"]
        for p in doc["packages"]["packages"]:
            t = p["seconds"]
            py, pc = t["cpython"], t["compiled"]
            # A workload CPython runs in under 0.1 s (an import) is shown in ms.
            kw = {"scale": 1000, "unit": " ms", "digits": 2} if py["median"] < 0.1 else {}
            out.append(f"| {p['package']} {p['version']} | {p['workload']} | {p['runs']} | "
                       f"{cell(py, **kw)} | {cell(pc, **kw)} | "
                       f"{competitor_cell(t.get('cython'), py, **kw)} | "
                       f"{competitor_cell(t.get('mypyc'), py, **kw)} | {p['ratio']:.2f}x | "
                       f"{vs(t.get('cython'), pc)} | {vs(t.get('mypyc'), pc)} |")
        out.append("")
        builds = doc["packages"].get("competitors", {}).get("builds", [])
        if builds:
            out += ["Competitor builds (the package's own modules, unchanged):", ""]
            for b in builds:
                after = f"; own test suite: {b['tests']}" if "tests" in b else ""
                timed = "; timed" if b.get("timed") else ""
                out.append(f"- {b['package']} {b['version']}: {b['tool']} ({b['mode']}): "
                           f"{b['summary']}{after}{timed}")
            out.append("")
    if "pydantic" in doc:
        d = doc["pydantic"]["pydantic"]
        out += [f"## pydantic {d['version']} (`bench/pydantic_check.py --time`, {d['runs']} "
                f"runs, each phase x{d['repeat']} per run, mean per repetition; "
                f"{d['native']} functions native)", "",
                "| phase | CPython | pycc-next | Cython | mypyc | CPython / pycc-next | "
                "pycc vs Cython | pycc vs mypyc |", "|---|---|---|---|---|---|---|---|"]
        ms = {"scale": 1000, "unit": " ms", "digits": 2}
        for ph, v in d["phases"].items():
            t = v["seconds"]
            py, pc = t["CPython"], t["compiled"]
            out.append(f"| {ph} | {cell(py, **ms)} | {cell(pc, **ms)} | "
                       f"{competitor_cell(t.get('cython'), py, **ms)} | "
                       f"{competitor_cell(t.get('mypyc'), py, **ms)} | {v['ratio']:.2f}x | "
                       f"{vs(t.get('cython'), pc)} | {vs(t.get('mypyc'), pc)} |")
        out.append("")
        builds = d.get("competitors", {}).get("builds", [])
        if builds:
            out += ["Competitor builds (pydantic but `pydantic.v1`, unchanged):", ""]
            for b in builds:
                after = f"; the workload: {b['workload']}" if "workload" in b else ""
                timed = "; timed" if b.get("timed") else ""
                out.append(f"- {b['tool']} ({b['mode']}): {b['summary']}{after}{timed}")
            out.append("")
    return "\n".join(out)


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--run")
    ap.add_argument("--packages")
    ap.add_argument("--pydantic")
    ap.add_argument("--json", required=True)
    ap.add_argument("--md", required=True)
    a = ap.parse_args()
    doc: dict = {"machine": machine()}
    for key in ("run", "packages", "pydantic"):
        path = getattr(a, key)
        if path and pathlib.Path(path).is_file():
            doc[key] = json.loads(pathlib.Path(path).read_text())
    pathlib.Path(a.json).write_text(json.dumps(doc, indent=1))
    pathlib.Path(a.md).write_text(markdown(doc) + "\n")
    return 0


if __name__ == "__main__":
    sys.exit(main())
