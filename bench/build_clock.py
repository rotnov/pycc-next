"""Build time as a multiple of a fixed C compile on the same machine.

The compile-speed gates (v0.5 accept 4: `package_check.py --max-build-ratio`,
`run.py --build-only --max-build-ratio`) compare `pycc build`'s CPU time with
that of a reference workload measured in the same run: `cc -O2 -c` of the C
text `reference_source()` generates (always the same). Wall time against an
absolute bound flipped with the runner rather than the compiler: GitHub's
ubuntu-24.04 runners are several CPU models (AMD EPYC 7763, 9V74 and 9V45,
Intel Xeon 8370C and 8573C), and the same build took 4.0 s on one and 7.6 s on
another. Measured on 20 runners (4 builds each), the build's time varied by a
factor of 2.0 and its ratio to the reference by 1.25; the ratio's spread
within one CPU model was 1-3% (coefficient of variation).

CPU time (user + system of the waited-for child processes, all their threads)
rather than wall time: a busy machine stretches the wall time of a build and of
the reference unequally (the build's backend runs on several threads, the
reference on one), but leaves their CPU time almost as it is: on 4 CPUs kept
busy by 4 other processes, the build's wall time grew 37% and its CPU time
not at all. CPU time does count work a build skips when it is cached: the
first `--ext` build compiles the shim's runtime object (about 30% more CPU
time, little more wall time, the compile running beside the backend), later
ones link the cached object (`src/build_pipeline/object_cache.rs`), so
`warm_cache` builds a small module first.

POSIX only (`resource`); on Windows `child_cpu_seconds` is None.
"""
import pathlib
import subprocess
import tempfile

try:
    import resource
except ImportError:  # Windows
    resource = None


def child_cpu_seconds() -> float | None:
    """User + system CPU time of every child process waited for so far."""
    if resource is None:
        return None
    r = resource.getrusage(resource.RUSAGE_CHILDREN)
    return r.ru_utime + r.ru_stime


def reference_source(functions: int = 160) -> str:
    """The reference workload's C text: `functions` functions with loops,
    a switch, struct and string accesses, each calling the previous one (so
    the optimizer inlines and unrolls). About 1.5-2.5 s of `cc -O2` on CI's
    runners."""
    out = ["#include <string.h>", "struct P { int x, y; double w; char tag[16]; };"]
    for k in range(functions):
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
    return "\n".join(out) + "\n"


def reference_seconds(runs: int = 3, cc: str = "cc") -> float:
    """CPU time of `cc -O2 -c` of `reference_source()`, the least of `runs`
    compiles (the least is the one least disturbed)."""
    with tempfile.TemporaryDirectory(prefix="pycc-ref-") as tmp:
        src = pathlib.Path(tmp) / "reference.c"
        src.write_text(reference_source())
        times = []
        for _ in range(runs):
            before = child_cpu_seconds()
            subprocess.run([cc, "-O2", "-c", str(src), "-o", str(src.with_suffix(".o"))],
                           check=True, capture_output=True)
            times.append(child_cpu_seconds() - before)
        return min(times)


def warm_cache(pycc: str) -> None:
    """Builds a one-function `--ext` module, so the shim's runtime object is
    cached before a timed build."""
    with tempfile.TemporaryDirectory(prefix="pycc-warm-") as tmp:
        src = pathlib.Path(tmp) / "warm.py"
        src.write_text("def f(x: int) -> int:\n    return x + 1\n")
        subprocess.run([pycc, "build", str(src), "-o", str(src.with_suffix("")), "--ext", "--non-strict", "--release",
                        "--select", "all"],
                       check=True, capture_output=True)
