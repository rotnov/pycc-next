"""Finding the `pycc` compiler and running it on each module of a project."""

from __future__ import annotations

import concurrent.futures
import dataclasses
import os
import pathlib
import shutil
import subprocess
import sys
import time

from ._project import ModuleOptions, ProjectError


class CompileError(Exception):
    """pycc refused a module that may not fall back to pure Python."""


# The compiler's file name (`pycc.exe` on Windows).
EXE_NAME = "pycc.exe" if os.name == "nt" else "pycc"


def find_pycc(env: dict | None = None) -> pathlib.Path:
    """The `pycc` executable: `$PYCC` (the executable, or an unpacked release archive
    holding `bin/pycc`, `bin\\pycc.exe` on Windows), else `pycc` on `PATH`. The
    compiler finds its runtime (`lib/pycc/`) next to itself, so the release layout
    must stay whole."""
    env = os.environ if env is None else env
    value = env.get("PYCC")
    if value:
        path = pathlib.Path(value)
        if path.is_dir():
            path = path / "bin" / EXE_NAME
        if not (path.is_file() and os.access(path, os.X_OK)):
            raise ProjectError(f"PYCC={value!r} is not the pycc executable or a pycc release "
                               "directory holding bin/pycc")
        return path.resolve()
    found = shutil.which("pycc", path=env.get("PATH"))
    if found is None:
        raise ProjectError("pycc.build cannot find the pycc compiler: set PYCC to its path "
                           "(or to an unpacked release archive), or put its bin/ on PATH")
    return pathlib.Path(found).resolve()


@dataclasses.dataclass
class Outcome:
    module: str
    artifact: pathlib.Path | None  # None: ships as Python (it fell back)
    seconds: float
    log: str


def command(pycc: pathlib.Path, source: pathlib.Path, out: pathlib.Path,
            options: ModuleOptions, release: bool) -> list[str]:
    """The `pycc build` command line for one module. Each module is its own
    extension, so the imports between a package's modules bind through CPython
    (`--foreign-relative-imports`), one shared module object per name."""
    cmd = [str(pycc), "build", str(source), "-o", str(out), "--ext",
           "--foreign-relative-imports"]
    if release:
        cmd.append("--release")
    if not options.strict:
        cmd += ["--non-strict", "--report-fallbacks"]
    return cmd + list(options.extra_args)


def compile_modules(pycc: pathlib.Path, staging: pathlib.Path,
                    modules: list[tuple[str, str, ModuleOptions]], release: bool,
                    env: dict, display: pathlib.Path) -> list[Outcome]:
    """Builds every `(dotted name, path in staging, options)` module in `staging`, in
    parallel. Every `.py` file stays in place until all are built, since a module's
    dotted name is found from the `__init__.py` files above it. pycc's messages name
    the project's own files (`display`, the source directory), not the copies."""

    def one(item):
        name, rel, options = item
        source = staging / rel
        out = source.with_suffix("")
        cmd = command(pycc, source, out, options, release)
        started = time.perf_counter()
        try:
            r = subprocess.run(cmd, cwd=staging, env=env, capture_output=True, text=True,
                               stdin=subprocess.DEVNULL)
        except OSError as e:
            raise ProjectError(f"could not run {pycc}: {e}") from None
        seconds = time.perf_counter() - started
        log = (r.stdout + r.stderr).strip()
        for copy in {str(staging), str(staging.resolve())}:
            log = log.replace(copy + os.sep, str(display) + os.sep)
        if r.returncode != 0:
            if options.fallback:
                return Outcome(name, None, seconds, log)
            raise CompileError(f"pycc could not compile {name} ({rel}):\n{log}\n\n"
                               f"command: {' '.join(cmd)}\n"
                               f"Fix the diagnostics above, or keep the module in Python with "
                               f'[tool.pycc] pure-python = ["{name}"] (or allow it to fall back '
                               f"with fallback-to-python).")
        artifacts = [p for p in out.parent.iterdir()
                     if p.name.startswith(out.name + ".") and p.suffix in (".so", ".pyd")]
        if len(artifacts) != 1:
            raise ProjectError(f"pycc built {name} but left {len(artifacts)} extension files "
                               f"for {out}")
        return Outcome(name, artifacts[0], seconds, log)

    jobs = max(1, min(len(modules), os.cpu_count() or 1))
    with concurrent.futures.ThreadPoolExecutor(jobs) as pool:
        futures = [pool.submit(one, m) for m in modules]
        outcomes, errors = [], []
        for f in futures:
            try:
                outcomes.append(f.result())
            except (CompileError, ProjectError) as e:
                errors.append(str(e))
    if errors:
        raise CompileError("\n\n".join(errors))
    for o in outcomes:
        if o.artifact is None:
            print(f"pycc.build: {o.module}: pycc could not compile it; it ships as Python "
                  f"(fallback-to-python):\n{o.log}", file=sys.stderr)
        else:
            print(f"pycc.build: {o.module}: compiled in {o.seconds:.2f}s", file=sys.stderr)
            if o.log:
                print("  " + o.log.replace("\n", "\n  "), file=sys.stderr)
    return outcomes
