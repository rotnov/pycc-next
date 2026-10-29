"""Build pydantic v2's Python part with pycc-next and count what compiles (v0.7).

pydantic 2.13.5 (MIT), pinned by the sha256 of its PyPI sdist, is built unchanged
with `--ext --non-strict`: whole, into one extension `pydantic/__init__.abi3.so` holding
every module (`pycc build pydantic --ext`, v0.8), or with `--separate` module by module,
each as its own `--foreign-relative-imports` extension module (pydantic-core, its Rust
part, is not built: it is the PyPI wheel the test interpreter has installed;
`pydantic.v1`, the bundled copy of pydantic 1, is kept in Python). For every module the script prints how many of its
functions and methods compile natively, from `--report-fallbacks`: a `def` inside a
range the report says runs in CPython (`fallback: ... at f:a-b runs in CPython`) is
not native; a class statement CPython runs with its methods compiled reports only its
header, so its methods count as native. The archive is untrusted data and is handled
as in `bench/package_check.py` (checked members, the `data` extraction filter).

v0.7 accept 4's measurement, not a CI gate; `.github/workflows/bench.yml` runs `--time`.

Usage: python3 bench/pydantic_check.py [--pycc PATH] [--cache DIR] [--keep DIR]
                                       [--jobs N] [--tests PYTHON]
                                       [--time [PYTHON]] [--runs N] [--repeat R]
                                       [--separate] [--json PATH] [--competitors]
                                       [MODULE ...]
  MODULE   build only the modules whose path contains one of these strings
           (implies --separate).
  --separate
           build each module as its own extension (before v0.8's whole-package
           build).
  --keep   leave the built tree (compiled modules, `.py` files removed, the sdist's
           tests beside them) in DIR instead of a temporary directory.
  --tests  then run pydantic's own test suite against the compiled tree with
           PYTHON, an interpreter with pydantic-core 2.46.5 and the suite's test
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
           best and the median per phase and the ratio of the medians.
  --json PATH
           with --time, also write every sample per phase and tree and their
           summaries (`bench/stats.py`).
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

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))
import competitors  # noqa: E402
import package_check  # noqa: E402
import stats  # noqa: E402

PYDANTIC = package_check.Package(
    name="pydantic",
    version="2.13.5",
    url="https://files.pythonhosted.org/packages/source/p/pydantic/pydantic-2.13.5.tar.gz",
    archive="pydantic-2.13.5.tar.gz",
    sha256="51a9c5f7b2f8e636f04c6cada605d9b6a3bf1348fdf945a3d8869b19bba0ee08",
    tree_sha256=None,
    layout=(("pydantic", "pydantic-2.13.5/pydantic"),),
    import_name="pydantic",
    modules=(),
    tests="",
    speed="",
    speed_label="",
    instructions="",
    instructions_label="",
    max_instructions=0,
    instruction_calls=0,
)

FALLBACK = re.compile(r"^fallback: .*? at (.*?):(\d+)(?:-(\d+))? runs in CPython: (.*)$", re.M)


def count_defs(text: str) -> int:
    return len(re.findall(r"^\s*(?:async\s+)?def ", text, re.M))


def count(py: pathlib.Path, r: subprocess.CompletedProcess,
          whole: bool) -> tuple[pathlib.Path, int, int, list[str]]:
    """`py`, its function count, how many run in CPython and the report's fallback
    reasons (or the build failure), from the build `r` (of the whole package when
    `whole`, whose report names each module's file)."""
    text = py.read_text(encoding="utf-8")
    defs = count_defs(text)
    if r.returncode != 0:
        last = (r.stderr.strip().splitlines() or ["?"])[-1]
        return py, defs, defs, ["BUILD FAILED: " + last]
    lines = text.splitlines()
    fell, notes = 0, []
    for m in FALLBACK.finditer(r.stderr):
        if whole and pathlib.Path(m[1]).resolve() != py.resolve():
            continue
        lo, hi = int(m[2]), int(m[3] or m[2])
        n = count_defs("\n".join(lines[lo - 1:hi]))
        fell += n
        notes.append(f"{lo}-{hi} ({n}): {m[4][:160]}")
    return py, defs, fell, notes


FLAGS = ["--ext", "--non-strict", "--release", "--report-fallbacks"]


def build(pycc: str, py: pathlib.Path) -> tuple[pathlib.Path, int, int, list[str]]:
    """Builds one module as its own extension (`--separate`)."""
    out = py.parent / "__init__" if py.name == "__init__.py" else py.with_suffix("")
    r = subprocess.run([pycc, "build", str(py), "-o", str(out), *FLAGS,
                        "--foreign-relative-imports"],
                       capture_output=True, text=True, timeout=1800)
    return count(py, r, whole=False)


WORKLOAD = pathlib.Path(__file__).resolve().parent / "pydantic_workload.py"
PHASES = ("import", "define", "schema", "roundtrip")


def time_trees(python: str, trees: dict[str, pathlib.Path], cwd: pathlib.Path,
               runs: int, repeat: int) -> dict[str, list[dict[str, float]]]:
    """Runs the workload `runs` times per tree, interleaved in alternating order,
    after one discarded warm-up run each."""
    def once(tree: pathlib.Path) -> dict[str, float]:
        r = subprocess.run([python, "-I", str(WORKLOAD), str(tree.resolve()), str(repeat)], cwd=cwd,
                           capture_output=True, text=True, timeout=900)
        if r.returncode != 0:
            sys.exit(f"the workload failed on {tree}:\n{r.stderr}")
        return json.loads(r.stdout)

    for tree in trees.values():
        once(tree)
    out: dict[str, list[dict[str, float]]] = {name: [] for name in trees}
    for i in range(runs):
        for name in list(trees) if i % 2 == 0 else list(reversed(trees)):
            out[name].append(once(trees[name]))
    return out


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
    """Prints best and median per phase, in ms, and CPython's median over the
    compiled tree's (above 1: the compiled tree is faster)."""
    print(f"{'phase':10} {'compiled best / median':>23} {'CPython best / median':>22} "
          f"{'CPython/compiled':>17}")
    for ph in PHASES:
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
            for ph in PHASES) + " (medians)")


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
    a = p.parse_args()
    archive = package_check.fetch(PYDANTIC, pathlib.Path(a.cache))
    with tempfile.TemporaryDirectory() as tmp_name:
        tmp = pathlib.Path(tmp_name)
        try:
            src = package_check.unpack(PYDANTIC, archive, tmp / "archive") / "pydantic-2.13.5"
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
        total = fell_total = failed = 0
        if a.separate or a.modules:
            with concurrent.futures.ThreadPoolExecutor(a.jobs) as ex:
                results = list(ex.map(lambda q: build(a.pycc, q), mods))
        else:
            package = tree / "pydantic"
            r = subprocess.run([a.pycc, "build", str(package), "-o", str(package / "__init__"),
                                *FLAGS, "--exclude", "v1"], capture_output=True, text=True,
                               timeout=3600, env=dict(os.environ, PYCC_JOBS=str(a.jobs)))
            results = [count(q, r, whole=True) for q in mods]
        for py, defs, fell, notes in results:
            total += defs
            fell_total += fell
            failed += bool(notes and notes[0].startswith("BUILD FAILED"))
            print(f"{py.relative_to(tree)}: {defs - fell}/{defs} native")
            for n in notes:
                print("    " + n)
            sys.stdout.flush()
        print(f"TOTAL: {total - fell_total}/{total} functions native, "
              f"{failed} modules failed to build")
        if not a.tests and not a.time:
            return 1 if failed else 0
        for py in mods:
            # A module whose separate build failed stays Python.
            if not (a.separate or a.modules) or any(
                    q.name.startswith(py.stem + ".") and q.suffix in (".so", ".pyd")
                    for q in py.parent.iterdir()):
                py.unlink()
        if a.time:
            plain = tmp / "plain"
            shutil.copytree(src / "pydantic", plain / "pydantic")
            work = tmp / "work"
            work.mkdir()
            python = str(pathlib.Path(a.time).absolute()) if "/" in a.time else a.time
            others, builds = competitor_trees(src, tmp, work, python, a.jobs) \
                if a.competitors else ({}, [])
            times = time_trees(python, {"compiled": tree, "CPython": plain, **others}, work,
                               a.runs, a.repeat)
            report(times)
            if a.json:
                pathlib.Path(a.json).write_text(json.dumps({"pydantic": {
                    "competitors": {"versions": competitors.versions(), "builds": builds,
                                    "modes": {b["tool"]: b["mode"] for b in builds
                                              if b.get("timed")}},
                    "version": PYDANTIC.version, "runs": a.runs, "repeat": a.repeat,
                    "native": f"{total - fell_total}/{total}",
                    "phases": {ph: {"seconds": {side: stats.summary([r[ph] for r in rs])
                                                for side, rs in times.items()},
                                    "ratio": statistics.median(r[ph] for r in times["CPython"])
                                    / statistics.median(r[ph] for r in times["compiled"])}
                               for ph in PHASES}}}, indent=1))
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
