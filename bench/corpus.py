"""Measure how much real-world Python pycc-next compiles and runs correctly.

Corpus: tests/corpus/codecontests/problems (200 untyped competitive-programming
solutions, CC BY 4.0). Never inspect or tune against the holdout set.

Usage: python3 bench/corpus.py [--pycc PATH] [--flags "--non-strict"] [-j 4] [--show-errors]
"""
import argparse
import collections
import concurrent.futures
import json
import pathlib
import re
import subprocess
import sys
import tempfile

ROOT = pathlib.Path(__file__).resolve().parent.parent
CORPUS = ROOT / "tests/corpus/codecontests/problems"


def norm(s: str) -> list[str]:
    return [line.rstrip() for line in s.strip().splitlines()]


def run_one(pdir: pathlib.Path, pycc: str, flags: list[str], tmp: pathlib.Path) -> tuple[str, str]:
    exe = tmp / pdir.name
    b = subprocess.run([pycc, "build", str(pdir / "solution.py"), "-o", str(exe), "--release", *flags],
                       capture_output=True, text=True, timeout=300)
    if b.returncode != 0:
        m = re.search(r"error\[(\w+)\]: ([^\n]{0,90})", b.stdout + b.stderr)
        return "compile", (f"{m.group(1)}: {m.group(2)}" if m else (b.stderr or b.stdout).strip()[:120])
    cases = json.loads((pdir / "tests.json").read_text())["cases"]
    for c in cases:
        try:
            r = subprocess.run([str(exe)], input=c["input"], capture_output=True, text=True, timeout=20)
        except subprocess.TimeoutExpired:
            return "timeout", ""
        if norm(r.stdout) != norm(c["output"]):
            return "wrong", r.stderr.strip()[-120:]
    return "pass", ""


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--pycc", default=str(ROOT / "target/release/pycc"))
    ap.add_argument("--flags", default="")
    ap.add_argument("-j", type=int, default=4)
    ap.add_argument("--show-errors", action="store_true")
    a = ap.parse_args()
    probs = sorted(p for p in CORPUS.iterdir() if p.is_dir())
    tmp = pathlib.Path(tempfile.mkdtemp(prefix="pycc-corpus-"))
    results = {}
    with concurrent.futures.ThreadPoolExecutor(a.j) as ex:
        futs = {ex.submit(run_one, p, a.pycc, a.flags.split(), tmp): p.name for p in probs}
        for f in concurrent.futures.as_completed(futs):
            try:
                results[futs[f]] = f.result()
            except Exception as e:  # noqa: BLE001 - report harness errors per problem
                results[futs[f]] = ("error", str(e)[:120])
    counts = collections.Counter(s for s, _ in results.values())
    n = len(probs)
    print(f"corpus: {n} problems, flags={a.flags!r}")
    for k in ("pass", "wrong", "timeout", "compile", "error"):
        print(f"  {k:8} {counts[k]:4}  ({100 * counts[k] / n:.1f}%)")
    reasons = collections.Counter(d for s, d in results.values() if s == "compile")
    print("top compile errors:")
    for r, c in reasons.most_common(15):
        print(f"  {c:4}  {r}")
    if a.show_errors:
        for name, (s, d) in sorted(results.items()):
            if s != "pass":
                print(f"{name}: {s}: {d}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
