"""Build real PyPI packages with pycc-next and run their own test suites against them.

Each package in PACKAGES is pinned by version and by the sha256 of what is downloaded. Its
modules are built unchanged with `--ext --non-strict`: a package (a directory) whole, into
one extension `pkg/__init__.abi3.so` serving its modules with compiled functions (`pycc
build PKG --ext`, v0.8; `--separate` builds each module as its own extension instead), a
single-module package as its own extension; the modules stay bytecode, so their `.py`
files stay, and the package's own unittest suite runs in CPython against the compiled
tree (and, first, against the sources), but for its tests that assert only a
wall-time ratio (`SuiteLoader`, `bench/package_check_workloads.py`). The downloaded
archive is untrusted data: every member must be a regular file or a directory (a
symlink, hard link or special file fails the check before anything is extracted, so
what is hashed is what is built and run), it is extracted with tarfile's `data` filter
into a fresh directory, the trees it is copied into are walked without following links,
and every Python process that imports from it runs isolated (`-I`), from a directory of
its own.

Packages:
  diff-match-patch 20241021 (Apache-2.0): Google's diff/match/patch text algorithms. The
    PyPI sdist ships the module and its unittest suite.
  tomli 2.5.0 (MIT): a TOML parser (the source of the standard library's `tomllib`). Its
    sdist has no tests, so the tagged GitHub source archive is used: package, unittest
    suite with the toml-test data, and the benchmark document. It is pinned by its
    sha256, and the files used also by a sha256 over their paths and contents.
  pypng 0.20220715.0 (MIT): a PNG encoder and decoder in one module, `png`. The PyPI
    sdist ships it with its unittest suite and the PngSuite test images (`pngsuite`,
    which stays Python with the tests). Its hot path is undoing the scanline filters
    when decoding.
  mistune 3.3.4 (BSD-3-Clause): a CommonMark Markdown parser and renderer, 37
    modules (core, block and inline parsers, plugins, directives, renderers). The
    PyPI sdist ships it with its unittest suite (1155 tests, the CommonMark spec
    examples among them) and its own benchmark's documents (`benchmark/cases`),
    which `--time` renders as `benchmark/bench.py` does; `--time` also times its
    import.
  pystache 0.6.8 (MIT): a Mustache template engine (v0.8 accept 4's package; the
    selection rule is in docs/ROADMAP.md), 17 modules. The PyPI sdist ships it with
    its unittest suite (a subpackage, `pystache.tests`, which stays Python) and its
    own benchmark (`pystache/tests/benchmark.py`), whose examples `--time` renders;
    `--time` also times its import.

Usage: python3 bench/package_check.py [PACKAGE ...] [--pycc PATH] [--cache DIR]
                                      [--keep DIR] [--time] [--runs N]
                                      [--build-ratio] [--jobs N]
                                      [--instructions] [--separate] [--json PATH]
                                      [--competitors] [--select profile|all]
  PACKAGE  names from PACKAGES (default: all of them).
  --select profile (default)
           compile only the functions a profile selects: `pycc profile` runs
           the package's own test suite and then each of its timed workloads
           once (the training input, `training`) against the unchanged sources
           first, and the build passes it as `--profile`. The modules the
           build keeps in Python (`excludes`) are excluded from the profile's
           candidates too, the layout's other entries are given to pass 3
           (`--verify-include`), and the check fails unless the build
           compiles exactly the set pass 3 measured as the winner.
  --select all
           compile every eligible function (`pycc build --select all`): what
           the build makes of each function does not depend on timing, so
           `--instructions` counts are stable from run to run.
  Either way the suite then checks that each module with a compiled
  function is imported through the extension.
  --time   also time each package's hot paths compiled and in CPython (pypng:
           its 256x256 filter image, and every PngSuite image its tests use;
           mistune and pystache: also `import PACKAGE`, see `import_timed`).
           Every timing is a fresh `-I` process (one untimed call, then one
           timed; an import is timed once); after one discarded warm-up
           process per tree (when N > 1), N (`--runs`) rounds run the CPython
           and the compiled tree once each, in alternating order. Prints the median and min-max per tree and
           the ratio of the medians.
  --json PATH
           write `--time`'s samples and their summaries (`bench/stats.py`), and
           `--competitors`' builds.
  --competitors
           also build the package's modules, unchanged, with Cython and with mypyc
           (`bench/competitors.py`; Cython in its default mode, then, should that not
           build or pass the suite, with annotations ignored), each in a copy of the
           CPython tree, run the package's suite against each, and with `--time`
           time every tree that passes it beside the other two, interleaved. A
           competitor that does not build, fails the suite or fails a workload is
           reported and not timed; it does not fail the check (CPython is the oracle).
  --instructions
           also count, under valgrind's callgrind, the instructions one call
           of each package's hot path executes against the compiled modules
           (with N = 1 and N = 1 + K calls; per call = the difference / K),
           and fail a package over its bound (`max_instructions`); CPython's
           count on the sources, in the same run, is printed beside it, with
           the ratio. Unlike
           wall time, the count is the same on every run of a shared CI
           runner, so it gates the code the compiler generates for the hot
           path; it cannot see what only changes speed (code placement).
           The bounds are for CI's Linux x86_64 runner and its CPython
           (actions/setup-python), 5% above its counts: off CI the counts
           differ by up to about 12% (CPython 3.15.2 from uv runs fewer
           instructions than CI's 3.15), so a local count does not calibrate
           a bound.
  --build-ratio
           with --select all: fail a package whose build's CPU time is more
           than its bound (`max_build_ratio`) times that of a fixed reference
           compile, `clang -O2` of `bench/build_clock.py`'s C text, measured at
           the start of the run (v0.5 accept 4's compile speed). Unlike wall
           time against seconds, the ratio does not move with the runner's CPU
           model: on CI's ubuntu-24.04 runners the same build took 0.72-1.46x
           its median CPU time, and 0.93-1.06x its median ratio. The bounds
           are 15% above CI's median, so a build 25% slower fails. POSIX only;
           needs `clang`.
  Each module's build time, the package's elapsed time and the sum of the
  modules' times are always printed; the modules build in parallel, as the
  `pycc.build` backend builds them (--jobs, by default one per CPU).
  --jobs N  build N modules at a time.
  --separate
           build each module of a package as its own extension, as before
           v0.8's whole-package build (for comparison).
Exit status 0 only when every module of every package builds (within the bound,
when one is given) and every suite passes.
"""
import argparse
import concurrent.futures
import hashlib
import json
import os
import pathlib
import re
import shutil
import subprocess
import sys
import tarfile
import tempfile
import time
import urllib.error
import urllib.request

ROOT = pathlib.Path(__file__).resolve().parent.parent
sys.path.insert(0, str(ROOT / "bench"))
import build_clock  # noqa: E402
import competitors  # noqa: E402
import stats  # noqa: E402
from package_check_workloads import (  # noqa: E402
    CHECK_COMPILED, PACKAGES, Package, import_timed, training)
# The compiler's file name in a Cargo profile directory.
PYCC_EXE = "pycc.exe" if os.name == "nt" else "pycc"



class UnsafeArchive(Exception):
    """An archive member, or a file under a path used, that is not a plain file or
    directory (a symlink, a hard link, a device, a FIFO)."""


def check_members(tar: tarfile.TarFile) -> None:
    """Refuses any member of `tar` that is not a regular file or a directory."""
    for member in tar.getmembers():
        if not (member.isreg() or member.isdir()):
            raise UnsafeArchive(f"archive member {member.name!r} is not a regular file "
                                "or a directory")


def regular_files(path: pathlib.Path) -> list[pathlib.Path]:
    """`path` itself when it is a file, else every file under it, walked without
    following links; anything that is not a regular file or a directory is refused."""
    if path.is_symlink() or not (path.is_file() or path.is_dir()):
        raise UnsafeArchive(f"{path} is not a regular file or a directory")
    if path.is_file():
        return [path]
    files = []
    for root, dirs, names in os.walk(path, followlinks=False):
        for name in dirs + names:
            q = pathlib.Path(root) / name
            if q.is_symlink() or not (q.is_file() or q.is_dir()):
                raise UnsafeArchive(f"{q} is not a regular file or a directory")
            if q.is_file():
                files.append(q)
    return files


def tree_digest(base: pathlib.Path, paths: list[str]) -> str:
    """sha256 over the relative path and contents of every file under `paths`."""
    h = hashlib.sha256()
    files = []
    for rel in paths:
        files.extend(regular_files(base / rel))
    for f in sorted(files, key=lambda q: q.relative_to(base).as_posix()):
        h.update(f.relative_to(base).as_posix().encode() + b"\0")
        h.update(hashlib.sha256(f.read_bytes()).digest())
    return h.hexdigest()


def download(url: str, attempts: int = 4) -> bytes:
    """The bytes at `url`, retrying a timed-out or dropped connection.

    A CI runner's download of a source archive sometimes stalls; the pinned
    sha256 in `fetch` still decides whether the bytes are accepted.
    """
    for attempt in range(1, attempts + 1):
        try:
            with urllib.request.urlopen(url, timeout=60) as r:
                return r.read()
        except (TimeoutError, ConnectionError, urllib.error.URLError) as e:
            if attempt == attempts:
                raise
            print(f"  download of {url} failed ({e}); retry {attempt} of {attempts - 1}",
                  file=sys.stderr)
            time.sleep(5 * attempt)
    raise AssertionError("unreachable")


def fetch(pkg: Package, cache: pathlib.Path) -> pathlib.Path:
    cache.mkdir(parents=True, exist_ok=True)
    path = cache / pkg.archive
    if pkg.sha256 is not None and path.exists() \
            and hashlib.sha256(path.read_bytes()).hexdigest() != pkg.sha256:
        path.unlink()
    if not path.exists():
        data = download(pkg.url)
        got = hashlib.sha256(data).hexdigest()
        if pkg.sha256 is not None and got != pkg.sha256:
            sys.exit(f"sha256 mismatch for {pkg.url}: got {got}, want {pkg.sha256}")
        if pkg.sha256 is None:
            print(f"  archive sha256 (not pinned): {got}")
        path.write_bytes(data)
    return path


def unpack(pkg: Package, archive: pathlib.Path, dest: pathlib.Path) -> pathlib.Path:
    """Extracts `archive` and checks its tree digest; returns the extracted root."""
    with tarfile.open(archive) as tar:
        check_members(tar)
        tar.extractall(dest, filter="data")
    if pkg.tree_sha256 is not None:
        got = tree_digest(dest, [src for _, src in pkg.layout])
        if got != pkg.tree_sha256:
            archive.unlink()
            sys.exit(f"tree sha256 mismatch for {pkg.url}: got {got}, want {pkg.tree_sha256}")
    return dest


def python(code: str, *args: str, cwd: pathlib.Path) -> subprocess.CompletedProcess:
    return subprocess.run([sys.executable, "-I", "-c", code, *args], cwd=cwd,
                          capture_output=True, text=True, timeout=900)


# `--time`'s measurements, for `--json`.
TIMINGS: list[dict] = []
# `--competitors`' builds, for `--json`.
BUILDS: list[dict] = []


def competitor_trees(pkg: Package, a: argparse.Namespace, plain: pathlib.Path,
                     tmp: pathlib.Path, cwd: pathlib.Path, single: bool,
                     names: str) -> dict[str, pathlib.Path]:
    """Builds the package's modules with Cython and with mypyc (`bench/competitors.py`),
    each in a copy of the CPython tree, and runs the package's own suite against what
    was built. Returns the trees that built and pass it, by tool. Cython is tried in its
    default mode first and, should that not build or pass, with annotations ignored."""
    out = {}
    dotted = dict(zip(pkg.modules, names.split(",")))
    attempts = [("cython", True), ("cython", False), ("mypyc", None)]
    for tool, typing in attempts:
        if tool in out:
            continue
        # The build sees only the package (a tree's other files, a `pyproject.toml`
        # among them, would configure setuptools); the tree is the CPython tree with
        # the built package in its place.
        top = pkg.layout[0][0]
        work = tmp / f"{tool}-{typing}-build"
        work.mkdir()
        if single:
            shutil.copyfile(plain / top, work / top)
        else:
            shutil.copytree(plain / top, work / top, symlinks=True,
                            ignore=shutil.ignore_patterns("__pycache__"))
        base = work if single else work / top
        files = [base / m for m in pkg.modules]
        b = competitors.cython(work, files, typing, a.jobs) if tool == "cython" \
            else competitors.mypyc(work, files)
        competitors.remove_compiled_sources(work, b)
        tree = tmp / f"{tool}-{typing}"
        shutil.copytree(plain, tree, symlinks=True,
                        ignore=lambda d, names: [n for n in names if n == "__pycache__"
                                                 or (pathlib.Path(d) == plain and n == top)])
        shutil.copytree(work, tree, symlinks=True, dirs_exist_ok=True)
        base = tree if single else tree / top
        res = {"package": pkg.name, "version": pkg.version} | b.as_json()
        BUILDS.append(res)
        print(f"  {tool} ({b.mode}): {b.summary()} in {b.seconds:.1f}s")
        if b.error or not b.compiled:
            continue
        compiled = ",".join(dotted[m] for m in pkg.modules
                            if (base / m).relative_to(tree).as_posix() in b.compiled)
        r = python(CHECK_COMPILED + pkg.tests, str(tree), "compiled", compiled, cwd=cwd)
        tail = (r.stderr.strip().splitlines() or ["?"])[-1]
        print(f"  test suite ({tool}): {tail}")
        res["tests"] = "pass" if r.returncode == 0 else f"fail: {tail[:200]}"
        if r.returncode == 0:
            out[tool] = tree
            res["timed"] = True
    return out


def check(pkg: Package, a: argparse.Namespace, tmp: pathlib.Path) -> bool:
    print(f"{pkg.name} {pkg.version}")
    archive = fetch(pkg, pathlib.Path(a.cache))
    try:
        src = unpack(pkg, archive, tmp / "archive")
    except UnsafeArchive as e:
        archive.unlink()
        print(f"FAIL: {pkg.url}: {e}")
        return False
    plain = tmp / "cpython"
    built = tmp / "compiled"
    for tree in (plain, built):
        for dst, rel in pkg.layout:
            (tree / dst).parent.mkdir(parents=True, exist_ok=True)
            regular_files(src / rel)  # refuses links and special files
            if (src / rel).is_dir():
                shutil.copytree(src / rel, tree / dst, symlinks=True)
            else:
                shutil.copyfile(src / rel, tree / dst, follow_symlinks=False)
            regular_files(tree / dst)
    cwd = tmp / "cwd"
    cwd.mkdir()
    # A single-module package's module sits at the root of the tree.
    single = pkg.layout[0][0].endswith(".py")
    pkg_dir = built if single else built / pkg.layout[0][0]

    flags = ["--ext", "--non-strict", "--release", "--report-fallbacks"]
    if a.select == "profile":
        # The training input is every workload the check runs (`training`):
        # a profile of one workload must not decide what is compiled for
        # another.
        train = tmp / "train.py"
        train.write_text(training(pkg))
        profile = tmp / "pycc.profile.json"
        target = plain / pkg.layout[0][0]
        # Pass 3's copy is the package alone when the tree has no project
        # marker: the layout's other entries (tests, benchmark data) are
        # added explicitly.
        include = [arg for dst, _ in pkg.layout[1:]
                   for arg in ("--verify-include", str(plain / dst))]
        # The modules the build keeps in Python are never candidates.
        include += [arg for e in pkg.excludes for arg in ("--exclude", e)]
        r = subprocess.run([a.pycc, "profile", str(target), "-o", str(profile), *include, "--",
                            sys.executable, str(train), str(plain)], cwd=cwd,
                           # Pass 3 may take up to MAX_PRUNE_BUILDS extra builds.
                           capture_output=True, text=True, timeout=2700)
        if r.returncode != 0:
            print(r.stdout + r.stderr)
            print(f"FAIL: pycc profile of {pkg.name}'s test suite failed")
            return False
        # Pass 3's checks, then the summary.
        for line in r.stderr.splitlines():
            if line.startswith("pycc profile:"):
                print("  " + line)
        flags += ["--profile", str(profile)]
    else:
        flags += ["--select", "all"]

    def build(mod: str) -> tuple[subprocess.CompletedProcess, float]:
        py = pkg_dir / mod
        out = py.with_suffix("") if mod != "__init__.py" else py.parent / "__init__"
        started = time.perf_counter()
        b = subprocess.run([a.pycc, "build", str(py), "-o", str(out), *flags,
                            "--foreign-relative-imports"],
                           capture_output=True, text=True, timeout=900)
        return b, time.perf_counter() - started

    env = dict(os.environ, PYCC_JOBS=str(a.jobs))
    started, cpu_started = time.perf_counter(), build_clock.child_cpu_seconds()
    if single or a.separate:
        # In parallel, as the `pycc.build` backend builds separate modules.
        with concurrent.futures.ThreadPoolExecutor(a.jobs) as pool:
            results = list(pool.map(build, pkg.modules))
    else:
        # The whole package, into one extension; its member builds run in
        # parallel (PYCC_JOBS at a time).
        excludes = [arg for e in pkg.excludes for arg in ("--exclude", e)]
        b = subprocess.run([a.pycc, "build", str(pkg_dir), "-o", str(pkg_dir / "__init__"),
                            *flags, *excludes], capture_output=True, text=True, timeout=900,
                           env=env)
        times = dict(re.findall(r"^pycc: (\S+): built in ([\d.]+)s$", b.stderr, re.M))
        results = []
        for mod in pkg.modules:
            dotted = ".".join([pkg.import_name, *pathlib.PurePosixPath(mod).with_suffix("")
                               .parts]).removesuffix(".__init__")
            results.append((b, float(times.get(dotted, "nan"))))
    build_seconds = time.perf_counter() - started
    # Every build process has been waited for: their CPU time is counted.
    build_cpu = None if cpu_started is None else build_clock.child_cpu_seconds() - cpu_started
    for mod, (b, seconds) in zip(pkg.modules, results):
        if seconds == seconds:  # a module without selected functions is not built (nan)
            print(f"  {mod}: built in {seconds:.2f}s")
        if b.returncode != 0:
            print(b.stdout + b.stderr)
            print(f"FAIL: pycc could not build {pkg.import_name}/{mod}")
            return False
        if single or a.separate:
            for line in b.stderr.splitlines():
                if line.startswith(("specialized:", "not specialized:", "region:")):
                    print(f"  {mod}: {line}")
    if not (single or a.separate):
        for line in dict.fromkeys(results[0][0].stderr.splitlines()):
            if line.startswith(("specialized:", "not specialized:", "region:")):
                print(f"  {line}")
    # Only now: a module's dotted name is found from the `__init__.py`
    # files beside and above it.
    # The selected functions compiled, by module.
    selected: dict[str, list[str]] = {}
    for b, _ in results:
        for line in b.stderr.splitlines():
            if line.startswith("compiled: "):
                module, _, qualname = line[len("compiled: "):].partition(":")
                if qualname not in selected.setdefault(module, []):
                    selected[module].append(qualname)
    summed = sum(seconds for _, seconds in results if seconds == seconds)
    # The modules' bytecode runs: their sources stay.
    n = sum(len(q) for q in selected.values())
    for module, qualnames in sorted(selected.items()):
        print(f"  compiled: {module}: {', '.join(qualnames)}")
    print(f"  {len(pkg.modules)} modules built in {build_seconds:.2f}s "
          f"({summed:.2f}s of builds, {a.jobs} at a time); {n} selected functions compiled")
    if build_cpu is not None and a.reference_seconds:
        print(f"  build CPU time {build_cpu:.2f}s, {build_cpu / a.reference_seconds:.2f}x the "
              f"reference compile's {a.reference_seconds:.2f}s")
    if a.select == "profile":
        # The build compiles exactly the set pass 3 measured as the winner.
        r = subprocess.run([a.pycc, "profile", str(plain / pkg.layout[0][0]), "-o", str(profile),
                            "--check", *[arg for e in pkg.excludes for arg in ("--exclude", e)],
                            "--", sys.executable], cwd=cwd, capture_output=True, text=True)
        measured = {line[len("selected: "):].split(" (")[0] for line in r.stdout.splitlines()
                    if line.startswith("selected: ")}
        compiled_set = {f"{module}:{q}" for module, qualnames in selected.items() for q in qualnames}
        if r.returncode != 0 or measured != compiled_set:
            print(r.stdout + r.stderr)
            print(f"FAIL: the build compiled {sorted(compiled_set)}, but pass 3 measured "
                  f"{sorted(measured)} as the winner")
            return False
    if a.build_ratio and build_cpu / a.reference_seconds > pkg.max_build_ratio:
        print(f"FAIL: {pkg.name}'s build took {build_cpu / a.reference_seconds:.2f}x the "
              f"reference compile's CPU time, more than {pkg.max_build_ratio:g}x")
        return False

    # A module's dotted name: `a/b.py` is `<package>.a.b`, `a/__init__.py` is `<package>.a`.
    names = ",".join(
        pkg.import_name if single else
        ".".join([pkg.import_name, *pathlib.PurePosixPath(m).with_suffix("").parts])
        .removesuffix(".__init__")
        for m in pkg.modules
    )
    compiled = ("selected", ",".join(sorted(selected)))
    for label, tree, check_names in (("cpython", plain, names), ("compiled", built, compiled[1])):
        mode = compiled[0] if label == "compiled" else label
        r = python(CHECK_COMPILED + pkg.tests, str(tree), mode, check_names, cwd=cwd)
        tail = (r.stderr.strip().splitlines() or ["?"])[-1]
        print(f"  test suite ({label}): {tail}")
        if r.returncode != 0:
            print(r.stdout + r.stderr)
            print(f"FAIL: {pkg.name}'s own tests fail against the {label} modules")
            return False

    others = competitor_trees(pkg, a, plain, tmp, cwd, single, names) if a.competitors else {}
    if a.time:
        trees = {"cpython": plain, "compiled": built, **others}
        for speed_label, speed in ((pkg.speed_label, pkg.speed), *pkg.more_speed):
            samples: dict[str, list[float]] = {label: [] for label in trees}
            dropped: dict[str, str] = {}
            # One discarded warm-up process per tree (file cache, the sources'
            # `.pyc`s), then `--runs` rounds of one fresh process per tree each.
            warm = [list(trees)] if a.runs > 1 else []
            for i, order in enumerate(warm + stats.interleaved(a.runs, list(trees))):
                for label in order:
                    if label in dropped:
                        continue
                    r = python(speed, str(trees[label]), "1", cwd=cwd)
                    if r.returncode != 0:
                        print(r.stdout + r.stderr)
                        if label in others:
                            # A competitor's failure is its own, not pycc-next's.
                            dropped[label] = "the workload fails: " + \
                                (r.stderr.strip().splitlines() or ["?"])[-1][:200]
                            continue
                        return False
                    if i >= len(warm):
                        samples[label].append(float(r.stdout))
            times = {label: stats.summary(xs) for label, xs in samples.items()
                     if label not in dropped}
            ratio = times["cpython"]["median"] / times["compiled"]["median"]
            # A workload CPython runs in under 0.1 s (an import) prints in ms.
            ms = times["cpython"]["median"] < 0.1
            print(f"  {speed_label}: CPython {stats.short(times['cpython'], ms)}, "
                  f"compiled {stats.short(times['compiled'], ms)} "
                  f"({ratio:.2f}x, medians of {a.runs})")
            for label in others:
                t = times.get(label)
                print(f"  {speed_label}: {label} " + (dropped[label] if t is None else
                      stats.short(t, ms)))
            TIMINGS.append({"package": pkg.name, "version": pkg.version,
                            "workload": speed_label, "runs": a.runs,
                            "seconds": times, "ratio": ratio, "not_timed": dropped,
                            "modes": {b["tool"]: b["mode"] for b in BUILDS
                                      if b["package"] == pkg.name and b.get("timed")}})
    if a.instructions:
        per_call = instructions_per_call(pkg, built, cwd)
        # CPython running the sources, in the same run: the bound is absolute,
        # but whether the compiled code beats the interpreter is the ratio.
        interpreted = instructions_per_call(pkg, plain, cwd)
        if per_call is None or interpreted is None:
            return False
        print(f"  {pkg.instructions_label}: {per_call / 1e6:.2f}M instructions per call "
              f"(at most {pkg.max_instructions / 1e6:.2f}M; CPython on the sources "
              f"{interpreted / 1e6:.2f}M, {interpreted / per_call:.2f}x)")
        if per_call > pkg.max_instructions:
            print(f"FAIL: {pkg.name}'s hot path executes more instructions than its bound")
            return False
    return True


def instructions_per_call(pkg: Package, tree: pathlib.Path, cwd: pathlib.Path) -> float | None:
    """The instructions one call of `pkg`'s hot path executes against `tree`, under
    callgrind: the count with 1 + K calls less the count with 1, over K. A fixed hash
    seed keeps CPython's own dict and set probing, and so the count, the same per run;
    `-I` would ignore it, so the process is isolated as `-I` isolates (`-P -s`, and no
    other `PYTHON*` variable in its environment)."""
    totals = []
    env = {k: v for k, v in os.environ.items() if not k.startswith("PYTHON")}
    env["PYTHONHASHSEED"] = "0"
    for calls in (1, 1 + pkg.instruction_calls):
        r = subprocess.run(["valgrind", "--tool=callgrind", "--callgrind-out-file=/dev/null",
                            sys.executable, "-P", "-s", "-c", pkg.instructions, str(tree),
                            str(calls)],
                           cwd=cwd, env=env, capture_output=True, text=True, timeout=1800)
        m = re.search(r"Collected : (\d+)", r.stderr)
        if r.returncode != 0 or m is None:
            print(r.stdout + r.stderr)
            print(f"FAIL: could not count {pkg.name}'s instructions under callgrind")
            return None
        totals.append(int(m[1]))
    return (totals[1] - totals[0]) / pkg.instruction_calls


def main() -> int:
    ap = argparse.ArgumentParser()
    target = pathlib.Path(os.environ.get("CARGO_TARGET_DIR", ROOT / "target"))
    ap.add_argument("packages", nargs="*", metavar="PACKAGE")
    ap.add_argument("--pycc", default=str(target / "release" / PYCC_EXE))
    ap.add_argument("--cache", default=str(target / "package_check"))
    ap.add_argument("--keep", help="build in this (new) directory and keep it")
    ap.add_argument("--time", action="store_true")
    ap.add_argument("--runs", type=int, default=5)
    ap.add_argument("--build-ratio", action="store_true")
    ap.add_argument("--jobs", type=int, default=os.cpu_count() or 1)
    ap.add_argument("--instructions", action="store_true")
    ap.add_argument("--separate", action="store_true")
    ap.add_argument("--json")
    ap.add_argument("--competitors", action="store_true")
    ap.add_argument("--select", choices=("profile", "all"), default="profile")
    a = ap.parse_args()
    for name in a.packages:
        if name not in PACKAGES:
            ap.error(f"unknown package {name!r} (known: {', '.join(PACKAGES)})")
    a.reference_seconds = None
    if a.build_ratio:
        # The bounds are those of `--select all` whole-package builds.
        if a.select != "all" or a.separate:
            ap.error("--build-ratio needs --select all (and no --separate)")
        if build_clock.child_cpu_seconds() is None:
            ap.error("--build-ratio needs a POSIX system (CPU time of child processes)")
        if shutil.which(build_clock.REFERENCE_CC) is None:
            ap.error(f"--build-ratio needs `{build_clock.REFERENCE_CC}` (the reference compile)")
        # The shim's runtime object cached, as it is for every build after a
        # machine's first (`build_clock.warm_cache`).
        build_clock.warm_cache(a.pycc)
        a.reference_seconds = build_clock.reference_seconds()
        print(f"reference compile ({build_clock.REFERENCE_CC} -O2, least of 3): "
              f"{a.reference_seconds:.2f}s of CPU time")

    ok = True
    for name in a.packages or list(PACKAGES):
        tmp = pathlib.Path(a.keep) / name if a.keep else \
            pathlib.Path(tempfile.mkdtemp(prefix="pycc-pkg-"))
        tmp.mkdir(parents=True, exist_ok=True)
        try:
            ok = check(PACKAGES[name], a, tmp) and ok
        finally:
            if not a.keep:
                shutil.rmtree(tmp, ignore_errors=True)
    print("OK" if ok else "FAIL")
    if a.json:
        doc: dict = {"packages": TIMINGS}
        if a.competitors:
            doc["competitors"] = {"versions": competitors.versions(), "builds": BUILDS}
        pathlib.Path(a.json).write_text(json.dumps(doc, indent=1))
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main())
