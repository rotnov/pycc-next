"""Cython and mypyc: the other compilers the benchmarks time pycc-next against.

CPython stays the correctness oracle; these are competitors only. `bench/run.py`,
`bench/package_check.py --time` and `bench/pydantic_check.py --time` build the same
unmodified `.py` sources with each of them, in place in a copy of the tree, and time the
result with the same interleaved fresh-process method and workloads as pycc-next. Both
run in the interpreter running the benchmark (`sys.executable`), which must have them
installed (`.github/competitor-requirements.txt` pins them by hash); a tool that is not
installed is reported as such, not timed.

  Cython   `cythonize -3 -i -k` (language_level=3, Cython's default directives, keep
           going past a module that fails), the C compiled with CFLAGS=-O2 (after
           CPython's own `sysconfig` flags, so the last `-O` wins). Default directives
           include `annotation_typing=True`: Cython's "pure Python mode" honoring the
           sources' annotations (an `int` stays a Python int, a `float` becomes a C
           double). `annotation_typing=False` (annotations ignored) is the other mode
           (`MODES`); a result names its mode.
  mypyc    `python -m mypyc FILES` (mypyc's defaults: its own `-O3`, MYPYC_OPT_LEVEL 3),
           all modules of a tree in one build. mypyc builds only code that type-checks
           under mypy; a build that fails is reported with mypy's first error, never
           worked around by patching the sources.

A module that does not build stays Python, and the result says which.
"""
import dataclasses
import importlib.metadata
import importlib.machinery
import os
import pathlib
import re
import shutil
import subprocess
import sys
import time

# Cython's modes: the directive value of `annotation_typing`, and its name in results.
MODES = {True: "annotation_typing=True (default directives)",
         False: "annotation_typing=False (annotations ignored)"}
CYTHON_ARGS = ["-3", "-i", "-k", "-q"]
CYTHON_CFLAGS = "-O2"


@dataclasses.dataclass
class Build:
    tool: str
    mode: str
    # Paths of the modules built, relative to the root, and of those that did not
    # build, with why.
    compiled: list[str]
    failed: dict[str, str]
    seconds: float
    # Why nothing was built (the tool missing, mypy's errors), or None.
    error: str | None = None

    def summary(self) -> str:
        if self.error:
            return f"does not compile: {self.error}"
        n = len(self.compiled) + len(self.failed)
        s = f"{len(self.compiled)}/{n} modules compiled"
        if self.failed:
            first = next(iter(self.failed.items()))
            s += f" ({first[0]}: {first[1]})"
        return s

    def as_json(self) -> dict:
        return dataclasses.asdict(self) | {"summary": self.summary()}


def version(dist: str) -> str | None:
    try:
        return importlib.metadata.version(dist)
    except importlib.metadata.PackageNotFoundError:
        return None


def versions() -> dict[str, str | None]:
    """The installed versions, and the flags every build uses (for the published result)."""
    return {"cython": version("cython"), "mypy": version("mypy"),
            "setuptools": version("setuptools"),
            "cython_flags": " ".join(["cythonize", *CYTHON_ARGS]) + f", CFLAGS={CYTHON_CFLAGS}",
            "mypyc_flags": "python -m mypyc FILES (defaults: -O3, MYPYC_OPT_LEVEL=3)"}


def built(py: pathlib.Path) -> bool:
    """Whether an extension module for `py` sits beside it."""
    stem = py.with_suffix("")
    return any(stem.with_name(stem.name + s).exists()
               for s in importlib.machinery.EXTENSION_SUFFIXES)


def first_error(text: str, pattern: str) -> str:
    m = re.search(pattern, text, re.M)
    if m:
        return m.group(0).strip()[:200]
    lines = text.strip().splitlines()
    return (lines[-1] if lines else "no output")[:200]


def cython(root: pathlib.Path, files: list[pathlib.Path], annotation_typing: bool = True,
           jobs: int | None = None) -> Build:
    """Compiles `files` (under `root`) in place with Cython."""
    rel = [f.relative_to(root).as_posix() for f in files]
    mode = MODES[annotation_typing]
    if version("cython") is None:
        return Build("cython", mode, [], {}, 0.0, "Cython is not installed")
    env = dict(os.environ, CFLAGS=CYTHON_CFLAGS)
    env.pop("CYTHON_FORCE_REGEN", None)
    started = time.perf_counter()
    r = subprocess.run([sys.executable, "-m", "Cython.Build.Cythonize", *CYTHON_ARGS,
                        "-j", str(jobs or os.cpu_count() or 1),
                        "-X", f"annotation_typing={annotation_typing}", *rel],
                       cwd=root, env=env, capture_output=True, text=True, timeout=3600)
    seconds = time.perf_counter() - started
    log = r.stdout + r.stderr
    compiled, failed = [], {}
    for f, name in zip(files, rel):
        if built(f):
            compiled.append(name)
        else:
            failed[name] = first_error(
                log, rf"^\S*{re.escape(pathlib.PurePosixPath(name).name)}:\d+:\d+: .*$")
    for f in files:
        f.with_suffix(".c").unlink(missing_ok=True)
    return Build("cython", mode, compiled, failed, seconds,
                 None if compiled else "no module built: " + first_error(
                     log, r"^\S+\.py:\d+:\d+: .*$"))


def mypyc(root: pathlib.Path, files: list[pathlib.Path]) -> Build:
    """Compiles `files` (under `root`) in place with mypyc, in one build."""
    rel = [f.relative_to(root).as_posix() for f in files]
    mode = "mypyc defaults"
    if version("mypy") is None:
        return Build("mypyc", mode, [], {}, 0.0, "mypy (mypyc) is not installed")
    started = time.perf_counter()
    r = subprocess.run([sys.executable, "-m", "mypyc", *rel], cwd=root,
                       capture_output=True, text=True, timeout=3600)
    seconds = time.perf_counter() - started
    for d in ("build", ".mypy_cache"):
        shutil.rmtree(root / d, ignore_errors=True)
    log = r.stdout + r.stderr
    if r.returncode != 0:
        errors = len(re.findall(r"^\S+:\d+: error: ", log, re.M))
        what = f"{errors} mypy error{'s' if errors != 1 else ''}, first: " if errors else ""
        return Build("mypyc", mode, [], {}, seconds,
                     what + first_error(log, r"^\S+:\d+: error: .*$"))
    compiled = [n for f, n in zip(files, rel) if built(f)]
    failed = {n: "no extension built" for f, n in zip(files, rel) if not built(f)}
    return Build("mypyc", mode, compiled, failed, seconds,
                 None if compiled else "no module built")


def remove_compiled_sources(root: pathlib.Path, b: Build) -> None:
    """Removes the `.py` of every module `b` built, so only its extension can be imported."""
    for name in b.compiled:
        (root / name).unlink()
