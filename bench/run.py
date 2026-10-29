"""Benchmark pycc-next against CPython and hand-written Rust.

Usage: python3 bench/run.py [--pycc target/release/pycc] [--runs 11] [--json PATH] [NAME ...]
       python3 bench/run.py --build-only [--max-build-seconds S] [NAME ...]
Prints a markdown table; every program's output is checked against CPython.
Each program runs `--runs` rounds; a round starts CPython, the pycc-next binary and
the Rust twin once each, as fresh processes, in an order rotated by one per round
(interleaved, so drift in a shared machine's load hits all three alike). The table
shows each side's median and, in parentheses, its min-max; the ratios are of the
medians. `--json PATH` also writes every sample and its summary (`bench/stats.py`).
With --build-only, only builds each program and prints how long `pycc build`
took; with --max-build-seconds, a build slower than S seconds fails (v0.5
accept 4: 2 on the CI Linux runner).

The Rust twins build with `rustc -O` for the default target CPU, exactly like
pycc-next's own `--release` output: `-C target-cpu=native` would let rustc use
instructions (AVX2, FMA) pycc-next deliberately does not assume, so the ratio
would measure the target flag rather than the compiler.

A program that imports numpy is built as an embedded executable with numpy
bundled from a `pycc.lock` written next to a copy of it, and run once before
timing (to warm the file cache). Such a program and its
Rust twin report the time of their compute section on stderr as
`kernel_seconds=<s>`; when all three report it, the row compares those times
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
import stats  # noqa: E402
KERNEL = re.compile(r"kernel_seconds=([0-9.eE+-]+)")


def once(cmd: list[str], cwd: pathlib.Path | None = None) -> tuple[float, str, float | None]:
    """Wall time, stdout, and the reported kernel time (or None) of one run."""
    t = time.perf_counter()
    done = subprocess.run(cmd, check=True, capture_output=True, text=True, cwd=cwd)
    seconds = time.perf_counter() - t
    found = KERNEL.search(done.stderr)
    return seconds, done.stdout, (float(found.group(1)) if found else None)


def fmt(s: dict[str, float]) -> str:
    return f"{s['median']:.3f}s ({s['min']:.3f}-{s['max']:.3f})"


def build(pycc: str, src: pathlib.Path, tmp: pathlib.Path, name: str,
          warm: bool = True) -> tuple[pathlib.Path | None, str, float]:
    """Compiles `src`, returning the executable (or None), the build log and
    how long `pycc build` took. An embedded build is run once (`warm`) to
    warm the file cache."""
    if "import numpy" not in src.read_text():
        exe = tmp / name
        t = time.perf_counter()
        b = subprocess.run([pycc, "build", str(src), "-o", str(exe), "--release"], capture_output=True, text=True)
        return (exe if b.returncode == 0 else None), b.stderr, time.perf_counter() - t
    work = tmp / f"{name}.src"
    work.mkdir()
    copy = work / src.name
    shutil.copy(src, copy)
    lock = subprocess.run([pycc, "lock", str(copy)], capture_output=True, text=True)
    if lock.returncode != 0:
        return None, lock.stderr, 0.0
    exe = work / name
    t = time.perf_counter()
    b = subprocess.run([pycc, "build", str(copy), "-o", str(exe), "--release"], capture_output=True, text=True)
    seconds = time.perf_counter() - t
    if b.returncode != 0:
        return None, b.stderr, seconds
    if warm:
        subprocess.run([str(exe)], check=True, capture_output=True)
    return exe, b.stderr, seconds


def build_times(args: argparse.Namespace, names: list[str], tmp: pathlib.Path) -> int:
    """Builds each program once, printing how long the build took."""
    failed = False
    print("| program | pycc build |")
    print("|---|---|")
    for name in names:
        exe, log, seconds = build(args.pycc, HERE / "py" / f"{name}.py", tmp, name, warm=False)
        if exe is None:
            print(f"| {name} | compile error |")
            print(log, file=sys.stderr)
            failed = True
            continue
        slow = args.max_build_seconds is not None and seconds > args.max_build_seconds
        failed |= slow
        mark = f" (over {args.max_build_seconds:g}s)" if slow else ""
        print(f"| {name} | {seconds:.2f}s{mark} |")
    return 1 if failed else 0


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--pycc", default=str(HERE.parent / "target/release/pycc"))
    ap.add_argument("--runs", type=int, default=11)
    ap.add_argument("--json")
    ap.add_argument("names", nargs="*")
    ap.add_argument("--build-only", action="store_true")
    ap.add_argument("--max-build-seconds", type=float)
    args = ap.parse_args()
    names = args.names or sorted(p.stem for p in (HERE / "py").glob("*.py"))
    tmp = pathlib.Path(tempfile.mkdtemp(prefix="pycc-bench-"))
    if args.build_only:
        try:
            return build_times(args, names, tmp)
        finally:
            shutil.rmtree(tmp, ignore_errors=True)
    print("| program | CPython | pycc-next | Rust | pycc vs CPython | pycc vs Rust |")
    print("|---|---|---|---|---|---|")
    failed = False
    results = []
    try:
        for name in names:
            src = HERE / "py" / f"{name}.py"
            cmds = {"cpython": [sys.executable, str(src)]}
            exe, log, _ = build(args.pycc, src, tmp, name)
            if exe is None:
                print(f"| {name} | | compile error | | | |")
                print(log, file=sys.stderr)
                failed = True
                results.append({"program": name, "error": "compile error"})
                continue
            cmds["pycc"] = [str(exe)]
            rs = HERE / "rs" / f"{name}.rs"
            if rs.exists():
                subprocess.run(["rustc", "-O", str(rs), "-o", str(tmp / f"{name}_rs")], check=True)
                cmds["rust"] = [str(tmp / f"{name}_rs")]
            walls = {side: [] for side in cmds}
            kernels = {side: [] for side in cmds}
            outs = {}
            for order in stats.interleaved(args.runs, list(cmds)):
                for side in order:
                    seconds, out, kernel = once(cmds[side])
                    walls[side].append(seconds)
                    outs[side] = out
                    if kernel is not None:
                        kernels[side].append(kernel)
            ok = outs["pycc"] == outs["cpython"]
            failed |= not ok
            kernel = all(len(kernels[side]) == args.runs for side in cmds)
            times = {side: stats.summary(kernels[side] if kernel else walls[side]) for side in cmds}
            label = f"{name} (kernel)" if kernel else name
            py, pc, rust = times["cpython"], times["pycc"], times.get("rust")
            rs_col = fmt(rust) if rust else "-"
            vs_rs = f"{pc['median'] / rust['median']:.2f}x slower" if rust else "-"
            mark = "" if ok else " (OUTPUT MISMATCH)"
            print(f"| {label} | {fmt(py)} | {fmt(pc)}{mark} | {rs_col} | "
                  f"**{py['median'] / pc['median']:.1f}x faster** | {vs_rs} |")
            sys.stdout.flush()
            results.append({"program": name, "kernel": kernel, "output_matches": ok,
                            "runs": args.runs, "seconds": times})
    finally:
        shutil.rmtree(tmp, ignore_errors=True)
    if args.json:
        pathlib.Path(args.json).write_text(json.dumps({"programs": results}, indent=1))
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
