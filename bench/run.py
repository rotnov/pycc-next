"""Benchmark pycc-next against CPython and hand-written Rust.

Usage: python3 bench/run.py [--pycc target/release/pycc] [--runs 7]
Prints a markdown table; every program's output is checked against CPython.

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
import pathlib
import re
import shutil
import subprocess
import sys
import tempfile
import time

HERE = pathlib.Path(__file__).resolve().parent
KERNEL = re.compile(r"kernel_seconds=([0-9.eE+-]+)")


def best(cmd: list[str], runs: int, cwd: pathlib.Path | None = None) -> tuple[float, str, float | None]:
    """Best wall time, stdout, and best reported kernel time (or None)."""
    out, times, kernels = "", [], []
    for _ in range(runs):
        t = time.perf_counter()
        done = subprocess.run(cmd, check=True, capture_output=True, text=True, cwd=cwd)
        times.append(time.perf_counter() - t)
        out = done.stdout
        found = KERNEL.search(done.stderr)
        if found:
            kernels.append(float(found.group(1)))
    return min(times), out, (min(kernels) if kernels and len(kernels) == runs else None)


def build(pycc: str, src: pathlib.Path, tmp: pathlib.Path, name: str) -> tuple[pathlib.Path | None, str]:
    """Compiles `src`, returning the executable (or None) and the build log."""
    if "import numpy" not in src.read_text():
        exe = tmp / name
        b = subprocess.run([pycc, "build", str(src), "-o", str(exe), "--release"], capture_output=True, text=True)
        return (exe if b.returncode == 0 else None), b.stderr
    work = tmp / f"{name}.src"
    work.mkdir()
    copy = work / src.name
    shutil.copy(src, copy)
    lock = subprocess.run([pycc, "lock", str(copy)], capture_output=True, text=True)
    if lock.returncode != 0:
        return None, lock.stderr
    exe = work / name
    b = subprocess.run([pycc, "build", str(copy), "-o", str(exe), "--release"], capture_output=True, text=True)
    if b.returncode != 0:
        return None, b.stderr
    subprocess.run([str(exe)], check=True, capture_output=True)
    return exe, b.stderr


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--pycc", default=str(HERE.parent / "target/release/pycc"))
    ap.add_argument("--runs", type=int, default=7)
    ap.add_argument("names", nargs="*")
    args = ap.parse_args()
    names = args.names or sorted(p.stem for p in (HERE / "py").glob("*.py"))
    tmp = pathlib.Path(tempfile.mkdtemp(prefix="pycc-bench-"))
    print("| program | CPython | pycc-next | Rust | pycc vs CPython | pycc vs Rust |")
    print("|---|---|---|---|---|---|")
    failed = False
    try:
        for name in names:
            src = HERE / "py" / f"{name}.py"
            t_py, ref, k_py = best([sys.executable, str(src)], args.runs)
            exe, log = build(args.pycc, src, tmp, name)
            if exe is None:
                print(f"| {name} | {t_py:.3f}s | compile error | | | |")
                print(log, file=sys.stderr)
                failed = True
                continue
            t_pc, out, k_pc = best([str(exe)], args.runs)
            ok = out == ref
            failed |= not ok
            rs = HERE / "rs" / f"{name}.rs"
            t_rs, k_rs = None, None
            if rs.exists():
                subprocess.run(["rustc", "-O", str(rs), "-o", str(tmp / f"{name}_rs")], check=True)
                t_rs, _, k_rs = best([str(tmp / f"{name}_rs")], args.runs)
            label = name
            if k_py is not None and k_pc is not None and (t_rs is None or k_rs is not None):
                label, t_py, t_pc, t_rs = f"{name} (kernel)", k_py, k_pc, k_rs
            rs_col = f"{t_rs:.3f}s" if t_rs else "-"
            vs_rs = f"{t_pc / t_rs:.2f}x slower" if t_rs else "-"
            mark = "" if ok else " (OUTPUT MISMATCH)"
            print(f"| {label} | {t_py:.3f}s | {t_pc:.3f}s{mark} | {rs_col} | **{t_py / t_pc:.1f}x faster** | {vs_rs} |")
    finally:
        shutil.rmtree(tmp, ignore_errors=True)
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
