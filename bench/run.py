"""Benchmark pycc-next against CPython and hand-written Rust.

Usage: python3 bench/run.py [--pycc target/release/pycc] [--runs 7]
Prints a markdown table; every program's output is checked against CPython.

The Rust twins build with `rustc -O` for the default target CPU, exactly like
pycc-next's own `--release` output: `-C target-cpu=native` would let rustc use
instructions (AVX2, FMA) pycc-next deliberately does not assume, so the ratio
would measure the target flag rather than the compiler.
"""
import argparse
import pathlib
import subprocess
import sys
import tempfile
import time

HERE = pathlib.Path(__file__).resolve().parent


def best(cmd: list[str], runs: int) -> tuple[float, str]:
    out, times = "", []
    for _ in range(runs):
        t = time.perf_counter()
        out = subprocess.run(cmd, check=True, capture_output=True, text=True).stdout
        times.append(time.perf_counter() - t)
    return min(times), out


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
    for name in names:
        src = HERE / "py" / f"{name}.py"
        t_py, ref = best([sys.executable, str(src)], args.runs)
        b = subprocess.run([args.pycc, "build", str(src), "-o", str(tmp / name), "--release"], capture_output=True, text=True)
        if b.returncode != 0:
            print(f"| {name} | {t_py:.3f}s | compile error | | | |")
            failed = True
            continue
        t_pc, out = best([str(tmp / name)], args.runs)
        ok = out == ref
        failed |= not ok
        rs = HERE / "rs" / f"{name}.rs"
        t_rs = None
        if rs.exists():
            subprocess.run(["rustc", "-O", str(rs), "-o", str(tmp / f"{name}_rs")], check=True)
            t_rs, _ = best([str(tmp / f"{name}_rs")], args.runs)
        rs_col = f"{t_rs:.3f}s" if t_rs else "-"
        vs_rs = f"{t_pc / t_rs:.2f}x slower" if t_rs else "-"
        mark = "" if ok else " (OUTPUT MISMATCH)"
        print(f"| {name} | {t_py:.3f}s | {t_pc:.3f}s{mark} | {rs_col} | **{t_py / t_pc:.1f}x faster** | {vs_rs} |")
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
