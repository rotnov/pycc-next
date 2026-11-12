"""Build pydantic v2's Python part with pycc-next and count what compiles (v0.7).

pydantic 2.14.2 (MIT), pinned by the sha256 of its PyPI sdist, is built unchanged
with `--ext --non-strict`: whole, into one extension `pydantic/__init__.abi3.so` holding
every module (`pycc build pydantic --ext`, v0.8), or with `--separate` module by module,
each as its own `--foreign-relative-imports` extension module (pydantic-core, its Rust
part, is not built: it is the PyPI wheel the test interpreter has installed;
`pydantic.v1`, the bundled copy of pydantic 1, is kept in Python). By default only the
functions the training profile selects are compiled (`--select`, below); the modules
stay bytecode, so their `.py` files stay. The script prints what was compiled and what
was not, from `--report-fallbacks`. The archive is untrusted data and is handled as in
`bench/package_check.py` (checked members, the `data` extraction filter).

v0.7 accept 4's measurement, not a CI gate; `.github/workflows/bench.yml` runs `--time`.

Usage: python3 bench/pydantic_check.py [--pycc PATH] [--cache DIR] [--keep DIR]
                                       [--jobs N] [--tests PYTHON]
                                       [--time [PYTHON]] [--runs N] [--repeat R]
                                       [--separate] [--json PATH] [--competitors]
                                       [--cachegrind [PYTHON]] [--cg-repeat R]
                                       [--select profile|all] [MODULE ...]
  MODULE   build only the modules whose path contains one of these strings
           (implies --separate).
  --select profile (default)
           compile only the functions the training profile selects
           (docs/design/compile-little.md): `pycc profile` first runs
           `bench/pydantic_workload.py --train` against the unchanged sources
           (with the `--time` interpreter, else this one, and `--exclude v1`, as
           the build), the build passes it as `--profile` and targets the same
           interpreter (`PYCC_PYTHON`, unless set): a winning set measured with
           other excludes or another Python is not valid for the build.
  --select all
           compile every eligible function (`pycc build --select all`).
  --separate
           build each module as its own extension (before v0.8's whole-package
           build).
  --keep   leave the built tree (the extension beside the modules' sources, the
           sdist's tests beside them) in DIR instead of a temporary directory.
  --tests  then run pydantic's own test suite against the compiled tree with
           PYTHON, an interpreter with pydantic-core 2.50.2 and the suite's test
           dependencies installed (pytest, pytest-mock, dirty-equals, inline-snapshot,
           jsonschema, pytz, tzdata, hypothesis, faker, email-validator, ...). The
           mypy, docs and benchmark tests are skipped.
  --time   first time `bench/pydantic_workload.py` (import; model definition, i.e.
           class creation and core-schema building; `model_json_schema()`;
           validation and serialization round-trips) against the compiled tree and
           against the unchanged sources, both with PYTHON (default: this
           interpreter) and the pydantic-core it has installed. Every run is a fresh
           `-I` subprocess. One discarded warm-up run per tree comes first (it writes
           the sources' `.pyc` files, as an installed package has them), then N
           (`--runs`, default 15) runs per tree, interleaved in alternating order,
           each phase repeated R (`--repeat`, default 20) times per run. Prints the
           best and the median per phase and the ratio of the medians, and the same
           for `process`: each run's whole-process wall time, from starting the
           interpreter to its exit (every phase's R repetitions, the interpreter's
           start-up and shutdown included).
  --json PATH
           with --time, also write every sample per phase and tree and their
           summaries (`bench/stats.py`).
  --cachegrind
           also count, per phase and for the whole process, the instructions, I1
           misses and branch mispredictions of the compiled tree and of the unchanged
           sources, with PYTHON (default: this interpreter), under `valgrind
           --tool=callgrind --cache-sim=yes --branch-sim=yes` with
           `PYTHONHASHSEED=0` (`docs/design/compile-little.md`, section 3). Each phase
           is its own process, the phase run inside `bench/headroom/hrmark.c`'s
           `hr_mark_region` (built here with `cc` against PYTHON's headers) and
           callgrind collecting only there (`--toggle-collect`); `define+schema`
           counts both phases of one process, as docs/design/compile-little.md's
           section 1 does. Each phase is repeated R
           (`--cg-repeat`, default 5) times. One discarded warm-up run per tree comes
           first. Also prints the size of the compiled tree's extensions (`.so` and
           their `.text`). With --json, written under `cachegrind`.
  --competitors
           with --time, also build pydantic (but `pydantic.v1`), unchanged, with
           Cython and with mypyc (`bench/competitors.py`; Cython in its default
           mode, then, should that not build or run the workload, with annotations
           ignored), and time every build that runs the workload beside the other
           two trees, interleaved. What does not build or run is reported, not
           timed, and does not fail the check.
"""
import argparse
import concurrent.futures
import os
import pathlib
import json
import re
import shutil
import statistics
import subprocess
import sys
import tempfile
import time

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))
import competitors  # noqa: E402
import package_check  # noqa: E402
import stats  # noqa: E402

PYDANTIC = package_check.Package(
    name="pydantic",
    version="2.14.2",
    url="https://files.pythonhosted.org/packages/source/p/pydantic/pydantic-2.14.2.tar.gz",
    archive="pydantic-2.14.2.tar.gz",
    sha256="fb7b2db6cefe8414dd612edeede49c4c433acb0432b0f92b95024e45f66c505d",
    tree_sha256=None,
    layout=(("pydantic", "pydantic-2.14.2/pydantic"),),
    import_name="pydantic",
    modules=(),
    tests="",
    speed="",
    speed_label="",
    instructions="",
    instructions_label="",
    max_instructions=0,
    instruction_calls=0,
    max_build_ratio=0,
)

def compiled_lines(r: subprocess.CompletedProcess) -> list[str]:
    """The `compiled: ` and `not compiled: ` lines of build `r`'s report."""
    return [line for line in r.stderr.splitlines()
            if line.startswith(("compiled: ", "not compiled: "))]


FLAGS = ["--ext", "--non-strict", "--release", "--report-fallbacks"]
# `pydantic.v1` stays Python. The training profile names the same excludes: a
# winning set pass 3 measured with other excludes is not valid for the build,
# which then compiles nothing.
EXCLUDE = ["--exclude", "v1"]


def training_profile(pycc: str, python: str, src: pathlib.Path, out: pathlib.Path) -> None:
    """`pycc profile`s the unchanged sources in `src` with the training input
    (`bench/pydantic_workload.py --train`, docs/design/compile-little.md, section
    4) into `out`."""
    r = subprocess.run([pycc, "profile", str(src / "pydantic"), "-o", str(out), *EXCLUDE, "--",
                        python, str(WORKLOAD), str(src.resolve()), "5", "--train"],
                       capture_output=True, text=True, timeout=1800)
    if r.returncode != 0:
        sys.exit(f"pycc profile failed:\n{r.stdout}{r.stderr}")
    # Pass 3's checks, then the summary.
    for line in r.stderr.splitlines():
        if line.startswith("pycc profile:"):
            print(line)


def build(pycc: str, py: pathlib.Path, select: list[str],
          env: dict[str, str]) -> subprocess.CompletedProcess:
    """Builds one module as its own extension (`--separate`)."""
    out = py.parent / "__init__" if py.name == "__init__.py" else py.with_suffix("")
    r = subprocess.run([pycc, "build", str(py), "-o", str(out), *FLAGS, *select,
                        "--foreign-relative-imports"],
                       capture_output=True, text=True, timeout=1800, env=env)
    if r.returncode != 0:
        print(f"{py}: BUILD FAILED: {(r.stderr.strip().splitlines() or ['?'])[-1]}")
    return r


WORKLOAD = pathlib.Path(__file__).resolve().parent / "pydantic_workload.py"
PHASES = ("import", "define", "schema", "roundtrip")
# What `time_trees` reports per run: the workload's phases, then the whole
# process's wall time.
ROWS = (*PHASES, "process")


def time_trees(python: str, trees: dict[str, pathlib.Path], cwd: pathlib.Path,
               runs: int, repeat: int) -> dict[str, list[dict[str, float]]]:
    """Runs the workload `runs` times per tree, interleaved in alternating order,
    after one discarded warm-up run each. Each run's seconds per phase, and its
    whole-process wall time as `process`."""
    def once(tree: pathlib.Path) -> dict[str, float]:
        start = time.perf_counter()
        r = subprocess.run([python, "-I", str(WORKLOAD), str(tree.resolve()), str(repeat)], cwd=cwd,
                           capture_output=True, text=True, timeout=900)
        process = time.perf_counter() - start
        if r.returncode != 0:
            sys.exit(f"the workload failed on {tree}:\n{r.stderr}")
        return {**json.loads(r.stdout), "process": process}

    for tree in trees.values():
        once(tree)
    out: dict[str, list[dict[str, float]]] = {name: [] for name in trees}
    for i in range(runs):
        for name in list(trees) if i % 2 == 0 else list(reversed(trees)):
            out[name].append(once(trees[name]))
    return out


HEADROOM = pathlib.Path(__file__).resolve().parent / "headroom"
CG_FIELDS = {"instructions": r"I\s+refs:\s+([\d,]+)",
             "i1_misses": r"I1\s+misses:\s+([\d,]+)",
             "mispredicts_cond": r"Mispredicts:\s+[\d,]+\s+\(\s*([\d,]+) cond",
             "mispredicts_ind": r"Mispredicts:\s+[\d,]+\s+\(\s*[\d,]+ cond \+\s+([\d,]+) ind"}


# The phases callgrind counts: each alone, define and schema together in one
# process (as docs/design/compile-little.md's section 1 measured them), and
# the whole process.
CG_ROWS = (*PHASES, "define+schema", "process")


def build_hrmark(python: str, out: pathlib.Path) -> pathlib.Path:
    """Builds `bench/headroom/hrmark.c` against `python`'s headers into `out`."""
    info = subprocess.run([python, "-c", "import sysconfig; print(sysconfig.get_paths()['include']);"
                           "print(sysconfig.get_config_var('EXT_SUFFIX'))"],
                          capture_output=True, text=True, check=True).stdout.split()
    out.mkdir(parents=True, exist_ok=True)
    subprocess.run(["cc", "-O2", "-shared", "-fPIC", "-I", info[0], str(HEADROOM / "hrmark.c"),
                    "-o", str(out / ("hrmark" + info[1]))], check=True)
    return out


def callgrind(python: str, tree: pathlib.Path, cwd: pathlib.Path, mark: str | None,
              repeat: int, hrmark: pathlib.Path) -> dict[str, int]:
    """One callgrind run of the workload on `tree`: of phase `mark` alone, or of the
    whole process when `mark` is None."""
    env = dict(os.environ, PYTHONHASHSEED="0")
    cmd = ["valgrind", "--tool=callgrind", "--cache-sim=yes", "--branch-sim=yes",
           f"--callgrind-out-file={cwd / 'callgrind.out'}"]
    if mark:
        env["PYDANTIC_WORKLOAD_MARK"] = f"{mark}:{hrmark}"
        cmd.append("--toggle-collect=hr_mark_region")
    r = subprocess.run([*cmd, python, "-I", str(WORKLOAD), str(tree.resolve()), str(repeat)],
                       cwd=cwd, capture_output=True, text=True, timeout=7200, env=env)
    if r.returncode != 0:
        sys.exit(f"callgrind failed on {tree}:\n{r.stderr[-4000:]}")
    out = {}
    for k, rx in CG_FIELDS.items():
        m = re.search(rx, r.stderr)
        if not m:
            sys.exit(f"no {k} in callgrind's output:\n{r.stderr[-4000:]}")
        out[k] = int(m[1].replace(",", ""))
    return out


def cachegrind(python: str, trees: dict[str, pathlib.Path], cwd: pathlib.Path,
               repeat: int, hrmark: pathlib.Path) -> dict[str, dict[str, dict[str, int]]]:
    """Callgrind counts per tree and per row of `CG_ROWS` (`process`: the whole run)."""
    for tree in trees.values():
        subprocess.run([python, "-I", str(WORKLOAD), str(tree.resolve()), "1"], cwd=cwd,
                       capture_output=True, check=True, timeout=900)
    return {name: {row: callgrind(python, tree, cwd, None if row == "process" else row,
                                  repeat, hrmark) for row in CG_ROWS}
            for name, tree in trees.items()}


def so_sizes(tree: pathlib.Path) -> dict[str, int]:
    """Total bytes of the extensions under `tree`, and of their `.text` sections."""
    files = [q for q in tree.rglob("*") if q.suffix in (".so", ".pyd")]
    text = 0
    for q in files:
        r = subprocess.run(["size", "-A", str(q)], capture_output=True, text=True)
        for line in r.stdout.splitlines():
            f = line.split()
            if f and f[0] == ".text":
                text += int(f[1])
    return {"so_bytes": sum(q.stat().st_size for q in files), "text_bytes": text}


def report_cachegrind(counts: dict[str, dict[str, dict[str, int]]], sizes: dict[str, int]) -> None:
    """Prints the callgrind counts in millions, compiled against CPython."""
    print(f"callgrind (millions)  {'instructions':>22} {'I1 misses':>18} "
          f"{'cond mispredicts':>18} {'ind mispredicts':>18}")
    print(f"{'phase':10} {'':10}" + "".join(f" {'compiled':>8} {'CPython':>8} " for _ in CG_FIELDS))
    for row in CG_ROWS:
        c, p = counts["compiled"][row], counts["CPython"][row]
        print(f"{row:21}" + "".join(f" {c[k] / 1e6:8.2f} {p[k] / 1e6:8.2f} " for k in CG_FIELDS))
    print(f"extension size: {sizes['so_bytes']} bytes, .text {sizes['text_bytes']} bytes")


def competitor_trees(src: pathlib.Path, tmp: pathlib.Path, cwd: pathlib.Path, python: str,
                     jobs: int) -> tuple[dict[str, pathlib.Path], list[dict]]:
    """Builds pydantic (but `pydantic.v1`) with Cython and with mypyc
    (`bench/competitors.py`; Cython in its default mode, then, should that not build or
    run the workload, with annotations ignored), each in a tree of its own, and runs
    the workload once against each. Returns the trees that ran it, by tool, and every
    build's result."""
    out: dict[str, pathlib.Path] = {}
    builds = []
    for tool, typing in (("cython", True), ("cython", False), ("mypyc", None)):
        if tool in out:
            continue
        tree = tmp / f"{tool}-{typing}"
        shutil.copytree(src / "pydantic", tree / "pydantic")
        files = sorted(q for q in (tree / "pydantic").rglob("*.py") if "v1" not in q.parts)
        b = competitors.cython(tree, files, typing, jobs) if tool == "cython" \
            else competitors.mypyc(tree, files)
        res = b.as_json()
        builds.append(res)
        print(f"{tool} ({b.mode}): {b.summary()} in {b.seconds:.1f}s")
        if b.error or not b.compiled:
            continue
        competitors.remove_compiled_sources(tree, b)
        r = subprocess.run([python, "-I", str(WORKLOAD), str(tree.resolve()), "1"], cwd=cwd,
                           capture_output=True, text=True, timeout=900)
        tail = (r.stderr.strip().splitlines() or ["?"])[-1]
        res["workload"] = "pass" if r.returncode == 0 else f"fail: {tail[:200]}"
        print(f"  the workload against it: {res['workload']}")
        if r.returncode == 0:
            out[tool] = tree
            res["timed"] = True
    return out, builds


def report(times: dict[str, list[dict[str, float]]]) -> None:
    """Prints best and median per phase and for the whole process, in ms, and
    CPython's median over the compiled tree's (above 1: the compiled tree is
    faster)."""
    print(f"{'phase':10} {'compiled best / median':>23} {'CPython best / median':>22} "
          f"{'CPython/compiled':>17}")
    for ph in ROWS:
        c = [r[ph] * 1000 for r in times["compiled"]]
        p = [r[ph] * 1000 for r in times["CPython"]]
        print(f"{ph:10} {min(c):10.2f} / {statistics.median(c):8.2f} "
              f"{min(p):9.2f} / {statistics.median(p):8.2f} "
              f"{statistics.median(p) / statistics.median(c):16.2f}x")
    for side in times:
        if side in ("compiled", "CPython"):
            continue
        print(f"{side}: " + ", ".join(
            f"{ph} {statistics.median(r[ph] for r in times[side]) * 1000:.2f} ms"
            for ph in ROWS) + " (medians)")


def main() -> int:
    p = argparse.ArgumentParser()
    p.add_argument("modules", nargs="*")
    p.add_argument("--pycc", default=str(package_check.ROOT / "target/release/pycc"))
    p.add_argument("--cache", default=str(package_check.ROOT / "target/package_check"))
    p.add_argument("--keep")
    p.add_argument("--jobs", type=int, default=4)
    p.add_argument("--tests")
    p.add_argument("--time", nargs="?", const=sys.executable)
    p.add_argument("--runs", type=int, default=15)
    p.add_argument("--repeat", type=int, default=20)
    p.add_argument("--separate", action="store_true")
    p.add_argument("--json")
    p.add_argument("--competitors", action="store_true")
    p.add_argument("--cachegrind", nargs="?", const=sys.executable)
    p.add_argument("--cg-repeat", type=int, default=5)
    p.add_argument("--select", choices=("profile", "all"), default="profile")
    a = p.parse_args()
    archive = package_check.fetch(PYDANTIC, pathlib.Path(a.cache))
    with tempfile.TemporaryDirectory() as tmp_name:
        tmp = pathlib.Path(tmp_name)
        try:
            src = package_check.unpack(PYDANTIC, archive, tmp / "archive") / "pydantic-2.14.2"
        except package_check.UnsafeArchive as e:
            sys.exit(str(e))
        package_check.regular_files(src)
        tree = pathlib.Path(a.keep) if a.keep else tmp / "tree"
        if tree.exists():
            shutil.rmtree(tree)
        shutil.copytree(src / "pydantic", tree / "pydantic")
        mods = sorted(q for q in (tree / "pydantic").rglob("*.py") if "v1" not in q.parts)
        if a.modules:
            mods = [m for m in mods if any(o in str(m) for o in a.modules)]
        # The interpreter the profile runs is the one the build targets: a
        # winning set measured with another interpreter is not valid for the
        # build, which would then compile nothing.
        target = a.time or sys.executable
        target = str(pathlib.Path(target).absolute()) if "/" in target else target
        env = {**os.environ, "PYCC_PYTHON": os.environ.get("PYCC_PYTHON", target),
               "PYCC_JOBS": str(a.jobs)}
        if a.select == "profile":
            profile = tmp / "pycc.profile.json"
            training_profile(a.pycc, target, src, profile)
            select = ["--profile", str(profile)]
        else:
            select = ["--select", "all"]
        if a.separate or a.modules:
            with concurrent.futures.ThreadPoolExecutor(a.jobs) as ex:
                results = list(ex.map(lambda q: build(a.pycc, q, select, env), mods))
        else:
            package = tree / "pydantic"
            r = subprocess.run([a.pycc, "build", str(package), "-o", str(package / "__init__"),
                                *FLAGS, *select, *EXCLUDE], capture_output=True,
                               text=True, timeout=3600, env=env)
            if r.returncode != 0:
                sys.exit(f"the build failed:\n{r.stderr}")
            results = [r]
        # Only the compiled functions are native; the modules run as bytecode.
        compiled = [line for r in results for line in compiled_lines(r)]
        print("\n".join(compiled))
        failed = sum(r.returncode != 0 for r in results)
        n = sum(line.startswith("compiled: ") for line in compiled)
        print(f"TOTAL: {n} functions compiled, {failed} modules failed to build")
        if not a.tests and not a.time and not a.cachegrind:
            return 1 if failed else 0
        plain = tmp / "plain"
        shutil.copytree(src / "pydantic", plain / "pydantic")
        work = tmp / "work"
        work.mkdir()
        doc: dict = {"version": PYDANTIC.version, "compiled": n}
        if a.cachegrind:
            cgpy = str(pathlib.Path(a.cachegrind).absolute()) if "/" in a.cachegrind \
                else a.cachegrind
            counts = cachegrind(cgpy, {"compiled": tree, "CPython": plain}, work, a.cg_repeat,
                                build_hrmark(cgpy, tmp / "hrmark"))
            sizes = so_sizes(tree / "pydantic")
            report_cachegrind(counts, sizes)
            doc["cachegrind"] = {"repeat": a.cg_repeat, "counts": counts, **sizes}
            sys.stdout.flush()
        if a.time:
            python = str(pathlib.Path(a.time).absolute()) if "/" in a.time else a.time
            others, builds = competitor_trees(src, tmp, work, python, a.jobs) \
                if a.competitors else ({}, [])
            times = time_trees(python, {"compiled": tree, "CPython": plain, **others}, work,
                               a.runs, a.repeat)
            report(times)
            doc.update({
                "competitors": {"versions": competitors.versions(), "builds": builds,
                                "modes": {b["tool"]: b["mode"] for b in builds
                                          if b.get("timed")}},
                "runs": a.runs, "repeat": a.repeat,
                "phases": {ph: {"seconds": {side: stats.summary([r[ph] for r in rs])
                                            for side, rs in times.items()},
                                "ratio": statistics.median(r[ph] for r in times["CPython"])
                                / statistics.median(r[ph] for r in times["compiled"])}
                           for ph in ROWS}})
        if a.json:
            pathlib.Path(a.json).write_text(json.dumps({"pydantic": doc}, indent=1))
        sys.stdout.flush()
        if not a.tests:
            return 1 if failed else 0
        shutil.copytree(src / "tests", tree / "tests")
        shutil.copy(src / "pyproject.toml", tree / "pyproject.toml")
        tests = str(pathlib.Path(a.tests).absolute()) if "/" in a.tests else a.tests
        r = subprocess.run([tests, "-m", "pytest", "tests", "-q", "-p", "no:cacheprovider",
                            "--ignore=tests/mypy", "--ignore=tests/test_docs.py",
                            "--ignore=tests/benchmarks"], cwd=tree, timeout=3600)
        return r.returncode or (1 if failed else 0)


if __name__ == "__main__":
    sys.exit(main())
