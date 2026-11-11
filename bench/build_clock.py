"""Build time as a multiple of a fixed C compile on the same machine.

The compile-speed gate (v0.5 accept 4, `package_check.py --build-ratio`)
compares `pycc build`'s CPU time with that of a reference workload measured in
the same run: `clang -O2 -c` of the C text `reference_source()` generates
(always the same). Wall time against an absolute bound moved with the runner
rather than the compiler: GitHub's ubuntu-24.04 runners are several CPU models
(AMD EPYC 7763, 9V74 and 9V45, Intel Xeon 8370C and 8573C), and the same
`--select all` build of diff-match-patch took 4.0 s on one and 7.6 s on
another. Measured on 8 idle runners (2 builds of each package each), a
package's build CPU time spread over 0.72-1.46x its median, its ratio to the
reference over 0.93-1.06x.

The reference is a clang compile because pycc's build time is mostly LLVM's
(optimization and code generation): with gcc (`cc` on Ubuntu) as the
reference, the ratio still differed by CPU model (mistune: 14.0x on an EPYC
9V45, 16.1x on a 7763), with clang it did not (21.5x and 22.5x).

CPU time (user + system of the waited-for child processes, all their threads)
rather than wall time: it does not count time a build spends waiting for a
CPU. Neither measure is load-proof on a 4-CPU runner (with two busy processes
beside it, a build's CPU time grew by half: the processes share cores), but
the reference compile slows down with it, so the ratio fell rather than rose.
CPU time does count work a build skips when it is cached: the first `--ext`
build compiles the shim's runtime object (about 30% more CPU time, little more
wall time), later ones link the cached object
(`src/build_pipeline/object_cache.rs`), so `warm_cache` builds a small module
first.

POSIX only (`resource`); on Windows `child_cpu_seconds` is None.
"""
import pathlib
import subprocess
import tempfile

try:
    import resource
except ImportError:  # Windows
    resource = None

# The reference compiler, found on PATH (CI's runner image: clang 18).
REFERENCE_CC = "clang"


def child_cpu_seconds() -> float | None:
    """User + system CPU time of every child process waited for so far."""
    if resource is None:
        return None
    r = resource.getrusage(resource.RUSAGE_CHILDREN)
    return r.ru_utime + r.ru_stime


def reference_source(functions: int = 160) -> str:
    """The reference workload's C text: `functions` functions with loops,
    a switch, struct and string accesses, each calling the previous one (so
    the optimizer inlines and unrolls). About 1-2 s of `clang -O2` on CI's
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


def reference_seconds(runs: int = 3, cc: str = REFERENCE_CC) -> float:
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
        subprocess.run([pycc, "build", str(src), "-o", str(src.with_suffix("")),
                        "--ext", "--non-strict", "--release", "--select", "all"],
                       check=True, capture_output=True)
