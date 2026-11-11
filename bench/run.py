"""Benchmark pycc-next against CPython, Cython, mypyc and hand-written Rust.

Usage: python3 bench/run.py [--pycc target/release/pycc] [--runs 11] [--json PATH]
                            [--no-competitors] [NAME ...]
       python3 bench/run.py --build-only [--max-build-ratio R] [NAME ...]
Prints a markdown table; every program's output is checked against CPython.
Each program runs `--runs` rounds; a round starts CPython, the pycc-next binary, the
competitors' builds and the Rust twin once each, as fresh processes, in an order rotated
by one per round (interleaved, so drift in a shared machine's load hits all of them
alike). The table shows each side's median and, in parentheses, its min-max; the ratios
are of the medians. `--json PATH` also writes every sample and its summary
(`bench/stats.py`).
With --build-only, only builds each program and prints how long `pycc build`
took, in wall time and in CPU time; with --max-build-ratio, a build whose CPU
time is more than R times that of the reference C compile
(`bench/build_clock.py`, measured once first) fails (v0.5 accept 4: 0.5 on
the CI Linux runner, derived in docs/roadmap/v0.1-v0.7.md). POSIX only.

The competitors (`bench/competitors.py`; `--no-competitors` leaves them out) build the
unmodified program as a module: Cython in its default mode (the annotations honored) and
with annotations ignored, and mypyc. Such a build runs as `python -c "import NAME"` (its
top-level code is the program), so it pays CPython's start like CPython does. It is
timed only when one untimed run printed CPython's output; otherwise the JSON and
stderr say why not (a build failure, a run-time error, different output). Only
pycc-next's mismatch fails the run: CPython is the oracle, the competitors are not
under test.

The Rust twins build with `rustc -O` for the default target CPU, exactly like
pycc-next's own `--release` output: `-C target-cpu=native` would let rustc use
instructions (AVX2, FMA) pycc-next deliberately does not assume, so the ratio
would measure the target flag rather than the compiler.

A program that imports numpy is built as an embedded executable with numpy
bundled from a `pycc.lock` written next to a copy of it, and run once before
timing (to warm the file cache). Such a program and its
Rust twin report the time of their compute section on stderr as
`kernel_seconds=<s>`; when every side reports it, the row compares those times
(marked "kernel"), since loading numpy is not part of what is compared.
"""
import argparse
import json
import pathlib
import re
import shutil
import subprocess
import sys
import tempfile
import time

HERE = pathlib.Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
import build_clock  # noqa: E402
import competitors  # noqa: E402
import stats  # noqa: E402
KERNEL = re.compile(r"kernel_seconds=([0-9.eE+-]+)")
# The competitors' sides: Cython in its default mode (the sources' annotations
# honored), Cython with annotations ignored, and mypyc.
COMPETITORS = ("cython", "cython_untyped", "mypyc")


def once(cmd: list[str], cwd: pathlib.Path | None = None) -> tuple[float, str, float | None]:
    """Wall time, stdout, and the reported kernel time (or None) of one run."""
    t = time.perf_counter()
    done = subprocess.run(cmd, check=True, capture_output=True, text=True, cwd=cwd)
    seconds = time.perf_counter() - t
    found = KERNEL.search(done.stderr)
    return seconds, done.stdout, (float(found.group(1)) if found else None)


def fmt(s: dict[str, float]) -> str:
    return f"{s['median']:.3f}s ({s['min']:.3f}-{s['max']:.3f})"


def vs(r: float) -> str:
    """pycc-next against a competitor, from the competitor's median over pycc-next's."""
    return f"{r:.2f}x faster" if r >= 1 else f"{1 / r:.2f}x slower"


def build(pycc: str, src: pathlib.Path, tmp: pathlib.Path, name: str,
          warm: bool = True) -> tuple[pathlib.Path | None, str, float, float | None]:
    """Compiles `src`, returning the executable (or None), the build log and
    how long `pycc build` took, in wall time and in CPU time (None where
    `build_clock` cannot measure it). An embedded build is run once (`warm`)
    to warm the file cache."""
    if "import numpy" not in src.read_text():
        exe = tmp / name
        t, cpu = time.perf_counter(), build_clock.child_cpu_seconds()
        b = subprocess.run([pycc, "build", str(src), "-o", str(exe), "--release"], capture_output=True, text=True)
        return ((exe if b.returncode == 0 else None), b.stderr, time.perf_counter() - t,
                cpu if cpu is None else build_clock.child_cpu_seconds() - cpu)
    work = tmp / f"{name}.src"
    work.mkdir()
    copy = work / src.name
    shutil.copy(src, copy)
    lock = subprocess.run([pycc, "lock", str(copy)], capture_output=True, text=True)
    if lock.returncode != 0:
        return None, lock.stderr, 0.0, None
    exe = work / name
    t, cpu = time.perf_counter(), build_clock.child_cpu_seconds()
    b = subprocess.run([pycc, "build", str(copy), "-o", str(exe), "--release"], capture_output=True, text=True)
    seconds = time.perf_counter() - t
    cpu = cpu if cpu is None else build_clock.child_cpu_seconds() - cpu
    if b.returncode != 0:
        return None, b.stderr, seconds, cpu
    if warm:
        subprocess.run([str(exe)], check=True, capture_output=True)
    return exe, b.stderr, seconds, cpu


def competitor(side: str, src: pathlib.Path, tmp: pathlib.Path, name: str,
               expected: str) -> tuple[dict, list[str] | None, pathlib.Path]:
    """Builds `src` with the competitor `side` in a directory of its own and checks it
    in one untimed run (which also warms the file cache). Returns its result for the
    JSON, the command to time (None when it is not timed: the result says why) and the
    directory to run it in."""
    d = tmp / f"{name}.{side}"
    d.mkdir()
    copy = d / src.name
    shutil.copy(src, copy)
    if side == "mypyc":
        b = competitors.mypyc(d, [copy])
    else:
        b = competitors.cython(d, [copy], annotation_typing=side == "cython")
    competitors.remove_compiled_sources(d, b)
    res: dict = {"tool": b.tool, "mode": b.mode, "build_seconds": b.seconds}
    if b.error or not b.compiled:
        res["error"] = b.summary()
        return res, None, d
    cmd = [sys.executable, "-c", f"import {name}"]
    try:
        _, out, _ = once(cmd, d)
    except subprocess.CalledProcessError as e:
        last = (e.stderr.strip().splitlines() or ["?"])[-1]
        res["error"] = f"fails at run time: {last[:200]}"
        return res, None, d
    if out != expected:
        res["error"] = "output differs from CPython's"
        return res, None, d
    return res, cmd, d


def build_times(args: argparse.Namespace, names: list[str], tmp: pathlib.Path) -> int:
    """Builds each program once, printing how long the build took (wall
    time, CPU time and, given a reference, CPU time over the reference's)."""
    ref = None
    if args.max_build_ratio is not None:
        if build_clock.child_cpu_seconds() is None:
            print("--max-build-ratio needs a POSIX system (CPU time of child processes)",
                  file=sys.stderr)
            return 2
        ref = build_clock.reference_seconds()
        print(f"reference compile (cc -O2, least of 3): {ref:.2f}s of CPU time\n")
    failed = False
    print("| program | pycc build | CPU time | x reference |")
    print("|---|---|---|---|")
    for name in names:
        exe, log, seconds, cpu = build(args.pycc, HERE / "py" / f"{name}.py", tmp, name, warm=False)
        if exe is None:
            print(f"| {name} | compile error | | |")
            print(log, file=sys.stderr)
            failed = True
            continue
        ratio = cpu / ref if ref else None
        slow = ratio is not None and ratio > args.max_build_ratio
        failed |= slow
        mark = f" (over {args.max_build_ratio:g}x)" if slow else ""
        cpu_text = "" if cpu is None else f"{cpu:.2f}s"
        ratio_text = "" if ratio is None else f"{ratio:.3f}{mark}"
        print(f"| {name} | {seconds:.2f}s | {cpu_text} | {ratio_text} |")
    return 1 if failed else 0


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--pycc", default=str(HERE.parent / "target/release/pycc"))
    ap.add_argument("--runs", type=int, default=11)
    ap.add_argument("--json")
    ap.add_argument("names", nargs="*")
    ap.add_argument("--build-only", action="store_true")
    ap.add_argument("--max-build-ratio", type=float)
    ap.add_argument("--no-competitors", action="store_true")
    args = ap.parse_args()
    names = args.names or sorted(p.stem for p in (HERE / "py").glob("*.py"))
    tmp = pathlib.Path(tempfile.mkdtemp(prefix="pycc-bench-"))
    if args.build_only:
        try:
            return build_times(args, names, tmp)
        finally:
            shutil.rmtree(tmp, ignore_errors=True)
    print("| program | CPython | pycc-next | Cython | mypyc | Rust | pycc vs CPython | "
          "pycc vs Cython | pycc vs mypyc | pycc vs Rust |")
    print("|---|---|---|---|---|---|---|---|---|---|")
    failed = False
    results = []
    try:
        for name in names:
            src = HERE / "py" / f"{name}.py"
            cmds = {"cpython": [sys.executable, str(src)]}
            cwds: dict[str, pathlib.Path] = {}
            exe, log, _, _ = build(args.pycc, src, tmp, name)
            if exe is None:
                print(f"| {name} | | compile error | | | | | | | |")
                print(log, file=sys.stderr)
                failed = True
                results.append({"program": name, "error": "compile error"})
                continue
            cmds["pycc"] = [str(exe)]
            rs = HERE / "rs" / f"{name}.rs"
            if rs.exists():
                subprocess.run(["rustc", "-O", str(rs), "-o", str(tmp / f"{name}_rs")], check=True)
                cmds["rust"] = [str(tmp / f"{name}_rs")]
            others: dict[str, dict] = {}
            if not args.no_competitors:
                _, expected, _ = once(cmds["cpython"])
                for side in COMPETITORS:
                    res, cmd, d = competitor(side, src, tmp, name, expected)
                    others[side] = res
                    if cmd is not None:
                        cmds[side], cwds[side] = cmd, d
            walls = {side: [] for side in cmds}
            kernels = {side: [] for side in cmds}
            outs = {}
            for order in stats.interleaved(args.runs, list(cmds)):
                for side in order:
                    seconds, out, kernel = once(cmds[side], cwds.get(side))
                    walls[side].append(seconds)
                    outs[side] = out
                    if kernel is not None:
                        kernels[side].append(kernel)
            ok = outs["pycc"] == outs["cpython"]
            failed |= not ok
            for side in others:
                if side in outs and outs[side] != outs["cpython"]:
                    others[side]["error"] = "output differs from CPython's (in a timed run)"
            kernel = all(len(kernels[side]) == args.runs for side in cmds)
            times = {side: stats.summary(kernels[side] if kernel else walls[side]) for side in cmds}
            for side, res in others.items():
                if side in times and "error" not in res:
                    res["seconds"] = times.pop(side)
                times.pop(side, None)
            label = f"{name} (kernel)" if kernel else name
            py, pc, rust = times["cpython"], times["pycc"], times.get("rust")
            rs_col = fmt(rust) if rust else "-"
            vs_rs = vs(rust['median'] / pc['median']) if rust else "-"
            mark = "" if ok else " (OUTPUT MISMATCH)"
            cells = []
            for side in ("cython", "mypyc"):
                t = others.get(side, {}).get("seconds")
                cells.append((f"{fmt(t)} {py['median'] / t['median']:.1f}x", vs(t["median"] / pc["median"]))
                             if t else ("-", "-"))
            print(f"| {label} | {fmt(py)} | {fmt(pc)}{mark} | {cells[0][0]} | {cells[1][0]} | "
                  f"{rs_col} | **{py['median'] / pc['median']:.1f}x faster** | "
                  f"{cells[0][1]} | {cells[1][1]} | {vs_rs} |")
            sys.stdout.flush()
            results.append({"program": name, "kernel": kernel, "output_matches": ok,
                            "runs": args.runs, "seconds": times, "competitors": others})
    finally:
        shutil.rmtree(tmp, ignore_errors=True)
    for r in results:
        for side, res in r.get("competitors", {}).items():
            if "error" in res:
                print(f"{r['program']}: {side} ({res['mode']}): {res['error']}", file=sys.stderr)
    if args.json:
        doc: dict = {"programs": results}
        if not args.no_competitors:
            doc["competitors"] = competitors.versions()
        pathlib.Path(args.json).write_text(json.dumps(doc, indent=1))
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
