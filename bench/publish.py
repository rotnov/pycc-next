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
the numbers mean only what that CPU and the spread say.
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


def markdown(doc: dict) -> str:
    m = doc["machine"]
    out = ["# pycc-next benchmarks", ""]
    out += [f"- {k}: {v}" for k, v in m.items() if v]
    out += ["", "Each cell: median (min-max) over the runs given; a run is a fresh process, "
            "the sides interleaved round by round. Ratios are of the medians; above 1, "
            "pycc-next is faster.", ""]
    if "run" in doc:
        progs = doc["run"]["programs"]
        runs = next((p["runs"] for p in progs if "runs" in p), 0)
        out += [f"## Programs (`bench/run.py`, {runs} runs, process start included)", "",
                "| program | CPython 3.13 | pycc-next | Rust (`-O`) | vs CPython | vs Rust |",
                "|---|---|---|---|---|---|"]
        for p in progs:
            if "error" in p:
                out.append(f"| {p['program']} | | {p['error']} | | | |")
                continue
            t = p["seconds"]
            py, pc, rs = t["cpython"], t["pycc"], t.get("rust")
            name = p["program"] + (" (kernel)" if p["kernel"] else "")
            bad = "" if p["output_matches"] else " (OUTPUT MISMATCH)"
            vs_rs = f"{pc['median'] / rs['median']:.2f}x slower" if rs else "-"
            out.append(f"| {name} | {cell(py)} | {cell(pc)}{bad} | "
                       f"{cell(rs) if rs else '-'} | "
                       f"**{py['median'] / pc['median']:.1f}x faster** | {vs_rs} |")
        out.append("")
    if "packages" in doc:
        out += ["## Real packages (`bench/package_check.py --time`)", "",
                "| package | workload | runs | CPython | compiled | CPython / compiled |",
                "|---|---|---|---|---|---|"]
        for p in doc["packages"]["packages"]:
            t = p["seconds"]
            out.append(f"| {p['package']} {p['version']} | {p['workload']} | {p['runs']} | "
                       f"{cell(t['cpython'])} | {cell(t['compiled'])} | "
                       f"{p['ratio']:.2f}x |")
        out.append("")
    if "pydantic" in doc:
        d = doc["pydantic"]["pydantic"]
        out += [f"## pydantic {d['version']} (`bench/pydantic_check.py --time`, {d['runs']} "
                f"runs, each phase x{d['repeat']} per run, mean per repetition; "
                f"{d['native']} functions native)", "",
                "| phase | CPython | compiled | CPython / compiled |", "|---|---|---|---|"]
        for ph, v in d["phases"].items():
            t = v["seconds"]
            out.append(f"| {ph} | {cell(t['CPython'], 1000, ' ms', 2)} | "
                       f"{cell(t['compiled'], 1000, ' ms', 2)} | {v['ratio']:.2f}x |")
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
