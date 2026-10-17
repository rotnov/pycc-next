"""Build real PyPI packages into wheels through pip with the `pycc.build` backend, install
each into a fresh venv and run the package's own test suite against the installed wheel
(ROADMAP v0.6 accept 1).

The packages, their pinned archives and the handling of those archives as untrusted
data are `bench/package_check.py`'s (sha256 pins; a symlink, hard link or special file
fails before extraction; Python runs isolated with `-I`). Each archive's project tree is
copied out and its `pyproject.toml` replaced by the one in HARNESS below, which selects
`pycc.build` and says where the package's modules are. That replacement is this test
harness, not a patch of the package: the packages use other backends (flit_core,
setuptools), and the injected file only states static metadata and layout -- no source
file of the package is changed. Then, for each package:

  1. `pip wheel --no-index --find-links BACKEND --no-deps` builds the wheel in pip's
     isolated build environment, which installs `pycc-build` from BACKEND (the release
     archive's `python/` directory, or a wheel built from this checkout's `python/`),
     with `PYCC` naming the compiler;
  2. the wheel is checked: its tag is `cp313-abi3-<platform>`, `Root-Is-Purelib: false`,
     every RECORD hash and size matches;
  3. `pip install --no-index --no-deps` puts it into a fresh venv;
  4. the venv's Python (`-I`, from an empty directory) imports every module the backend
     compiles and checks it is an extension module of the venv's site-packages, then runs
     the package's own tests, copied out of the archive without its sources.

Usage: python3 bench/wheel_check.py [PACKAGE ...] [--pycc PATH] [--backend DIR]
                                    [--cache DIR] [--keep DIR]
  --pycc     the compiler (default: target/release/pycc); passed to the backend as PYCC.
  --backend  a directory holding pycc_build-*.whl (default: build one from python/).
Exit status 0 only when every package's wheel builds, checks out, installs, and its
suite passes against it.
"""
import argparse
import base64
import csv
import dataclasses
import hashlib
import io
import os
import pathlib
import shutil
import subprocess
import sys
import tempfile
import zipfile

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))
import package_check  # noqa: E402  (the pinned packages and the untrusted-archive rules)

ROOT = pathlib.Path(__file__).resolve().parent.parent

BUILD_SYSTEM = """\
# Injected by bench/wheel_check.py (test harness): build with pycc.build.
[build-system]
requires = ["pycc-build"]
build-backend = "pycc.build"
"""


@dataclasses.dataclass(frozen=True)
class Harness:
    # The project's directory inside the extracted archive.
    root: str
    # The [project] and [tool.pycc] tables of the injected pyproject.toml.
    pyproject: str
    # (path in the test directory, path inside the archive): the test suite.
    tests: tuple[tuple[str, str], ...]
    # Code run by the venv's Python with the test directory as argv[1]: runs the suite.
    run: str
    # Modules the wheel must hold as extension modules.
    compiled: tuple[str, ...]


HARNESS = {
    "diff-match-patch": Harness(
        root="diff_match_patch-20241021",
        pyproject="""
[project]
name = "diff-match-patch"
version = "20241021"
requires-python = ">=3.13"
license = "Apache-2.0"

[tool.pycc]
strict = false
packages = ["diff_match_patch"]
# The package's unittest suite ships inside it; it stays Python (it is the oracle).
pure-python = ["diff_match_patch.tests"]
""",
        tests=(),
        run="""
from diff_match_patch.tests import diff_match_patch_test as t
r = unittest.main(module=t, argv=["wheel_check"], exit=False, verbosity=1).result
sys.exit(0 if r.wasSuccessful() and r.testsRun > 0 else 1)
""",
        compiled=("diff_match_patch", "diff_match_patch.__version__",
                  "diff_match_patch.diff_match_patch"),
    ),
    "tomli": Harness(
        root="tomli-2.5.0",
        pyproject="""
[project]
name = "tomli"
version = "2.5.0"
requires-python = ">=3.13"
license = "MIT"

[tool.pycc]
strict = false
# src/tomli is found by the default discovery (src layout).
""",
        tests=(("tests", "tomli-2.5.0/tests"),),
        run="""
import os
r = unittest.main(module=None, exit=False, verbosity=1, argv=[
    "wheel_check", "discover", "-s", os.path.join(sys.argv[1], "tests"),
    "-t", sys.argv[1]]).result
sys.exit(0 if r.wasSuccessful() and r.testsRun > 0 else 1)
""",
        compiled=("tomli", "tomli._parser", "tomli._re", "tomli._types"),
    ),
    "pypng": Harness(
        root="pypng-0.20220715.0",
        pyproject="""
[project]
name = "pypng"
version = "0.20220715.0"
requires-python = ">=3.13"
license = "MIT"

[tool.pycc]
strict = false
source-dir = "code"
py-modules = ["png"]
""",
        tests=(("test_png.py", "pypng-0.20220715.0/code/test_png.py"),
               ("pngsuite.py", "pypng-0.20220715.0/code/pngsuite.py")),
        # `test_test_dir` runs the sdist's shell scripts, which are not copied: it
        # skips itself.
        run="""
import test_png
r = unittest.main(module=test_png, argv=["wheel_check"], exit=False, verbosity=1).result
sys.exit(0 if r.wasSuccessful() and r.testsRun > 0 else 1)
""",
        compiled=("png",),
    ),
}

# Prefixed to `Harness.run`: argv[1] is the test directory (put on the path for the
# suite's own modules), argv[2] the modules that must be extension modules installed in
# this venv's site-packages.
CHECK_INSTALLED = """
import importlib, pathlib, sys, sysconfig, unittest
sys.path.insert(0, sys.argv[1])
site = {pathlib.Path(sysconfig.get_path(k)).resolve() for k in ("purelib", "platlib")}
for name in sys.argv[2].split(","):
    m = importlib.import_module(name)
    f = pathlib.Path(m.__file__).resolve()
    if not f.name.endswith((".abi3.so", ".pyd")) or not any(s in f.parents for s in site):
        sys.exit(f"{name} was imported from {f}, not an extension module of the wheel")
    print(f"  {name}: {f.name}")
"""


def check_wheel(path: pathlib.Path) -> str | None:
    """Checks the wheel's tag, WHEEL file and RECORD; returns an error or None."""
    tag = path.stem.split("-", 2)[2]
    if not tag.startswith("cp313-abi3-") or tag.endswith("-any"):
        return f"{path.name}: tag {tag!r} is not cp313-abi3-<platform>"
    with zipfile.ZipFile(path) as z:
        names = z.namelist()
        dist_info = {n.split("/")[0] for n in names if n.split("/")[0].endswith(".dist-info")}
        if len(dist_info) != 1:
            return f"{path.name}: {len(dist_info)} .dist-info directories"
        d = dist_info.pop()
        wheel = z.read(f"{d}/WHEEL").decode()
        if "Root-Is-Purelib: false" not in wheel or f"Tag: {tag}" not in wheel:
            return f"{path.name}: WHEEL does not say Root-Is-Purelib: false and Tag: {tag}"
        rows = list(csv.reader(io.StringIO(z.read(f"{d}/RECORD").decode())))
        recorded = {r[0] for r in rows}
        if recorded != set(names):
            return f"{path.name}: RECORD lists {sorted(recorded ^ set(names))} wrongly"
        for name, digest, size in rows:
            if name == f"{d}/RECORD":
                if digest or size:
                    return f"{path.name}: RECORD hashes itself"
                continue
            data = z.read(name)
            want = "sha256=" + base64.urlsafe_b64encode(
                hashlib.sha256(data).digest()).rstrip(b"=").decode()
            if digest != want or int(size) != len(data):
                return f"{path.name}: RECORD entry for {name} does not match"
    return None


def backend_wheel(dest: pathlib.Path) -> pathlib.Path:
    """Builds pycc-build's wheel from this checkout's python/ with its own backend."""
    dest.mkdir(parents=True)
    code = ("import sys; sys.path.insert(0, '.'); from pycc import build; "
            "print(build.build_wheel(sys.argv[1]))")
    subprocess.run([sys.executable, "-I", "-c", code, str(dest)], cwd=ROOT / "python",
                   check=True, capture_output=True, text=True)
    return dest


def run(cmd: list[str], **kw) -> subprocess.CompletedProcess:
    return subprocess.run(cmd, capture_output=True, text=True, timeout=1800, **kw)


def check(name: str, a: argparse.Namespace, backend: pathlib.Path, tmp: pathlib.Path) -> bool:
    pkg, h = package_check.PACKAGES[name], HARNESS[name]
    print(f"{pkg.name} {pkg.version}")
    archive = package_check.fetch(pkg, pathlib.Path(a.cache))
    try:
        src = package_check.unpack(pkg, archive, tmp / "archive")
        package_check.regular_files(src / h.root)  # refuses links and special files
    except package_check.UnsafeArchive as e:
        archive.unlink()
        print(f"FAIL: {pkg.url}: {e}")
        return False
    project = tmp / "project"
    shutil.copytree(src / h.root, project, symlinks=True)
    (project / "pyproject.toml").write_text(BUILD_SYSTEM + h.pyproject)
    tests = tmp / "tests"
    tests.mkdir()
    for dst, rel in h.tests:
        if (src / rel).is_dir():
            shutil.copytree(src / rel, tests / dst, symlinks=True)
        else:
            shutil.copyfile(src / rel, tests / dst, follow_symlinks=False)
        package_check.regular_files(tests / dst)

    venv = tmp / "venv"
    r = run([sys.executable, "-I", "-m", "venv", str(venv)])
    if r.returncode != 0:
        print(r.stdout + r.stderr)
        print("FAIL: could not create a venv")
        return False
    vpy = venv / "Scripts" / "python.exe" if os.name == "nt" else venv / "bin" / "python"
    env = {k: v for k, v in os.environ.items() if not k.startswith(("PYTHON", "PIP_"))}
    env["PYCC"] = str(pathlib.Path(a.pycc).resolve())
    wheels = tmp / "wheels"
    r = run([str(vpy), "-I", "-m", "pip", "wheel", "--no-index", "--find-links", str(backend),
             "--no-deps", "--no-cache-dir", "-v", "-w", str(wheels), str(project)],
            env=env, cwd=tmp)
    for line in (r.stdout + r.stderr).splitlines():
        if "pycc.build" in line or line.lstrip().startswith(("fallback:", "specialized:")):
            print("  " + line.strip())
    built = sorted(wheels.glob("*.whl")) if wheels.is_dir() else []
    if r.returncode != 0 or len(built) != 1:
        print(r.stdout[-4000:] + r.stderr[-4000:])
        print(f"FAIL: pip could not build {pkg.name}'s wheel with pycc.build")
        return False
    wheel = built[0]
    print(f"  wheel: {wheel.name} ({wheel.stat().st_size // 1024} KiB)")
    problem = check_wheel(wheel)
    if problem:
        print(f"FAIL: {problem}")
        return False
    r = run([str(vpy), "-I", "-m", "pip", "install", "--no-index", "--no-deps", str(wheel)],
            env=env, cwd=tmp)
    if r.returncode != 0:
        print(r.stdout + r.stderr)
        print(f"FAIL: pip could not install {wheel.name}")
        return False
    cwd = tmp / "cwd"
    cwd.mkdir()
    r = run([str(vpy), "-I", "-c", CHECK_INSTALLED + h.run, str(tests), ",".join(h.compiled)],
            env=env, cwd=cwd)
    print(r.stdout.rstrip())
    tail = (r.stderr.strip().splitlines() or ["?"])[-1]
    print(f"  test suite (installed wheel): {tail}")
    if r.returncode != 0:
        print(r.stderr)
        print(f"FAIL: {pkg.name}'s own tests fail against the installed wheel")
        return False
    return True


def main() -> int:
    ap = argparse.ArgumentParser()
    target = pathlib.Path(os.environ.get("CARGO_TARGET_DIR", ROOT / "target"))
    ap.add_argument("packages", nargs="*", metavar="PACKAGE")
    ap.add_argument("--pycc", default=str(target / "release" / package_check.PYCC_EXE))
    ap.add_argument("--backend", help="a directory holding pycc_build-*.whl")
    ap.add_argument("--cache", default=str(target / "package_check"))
    ap.add_argument("--keep", help="build in this (new) directory and keep it")
    a = ap.parse_args()
    for name in a.packages:
        if name not in HARNESS:
            ap.error(f"unknown package {name!r} (known: {', '.join(HARNESS)})")
    base = pathlib.Path(a.keep) if a.keep else pathlib.Path(tempfile.mkdtemp(prefix="pycc-whl-"))
    base = base.resolve()
    base.mkdir(parents=True, exist_ok=True)
    ok = True
    try:
        backend = pathlib.Path(a.backend).resolve() if a.backend \
            else backend_wheel(base / "backend")
        if not list(backend.glob("pycc_build-*.whl")):
            print(f"FAIL: no pycc_build-*.whl in {backend}")
            return 1
        for name in a.packages or list(HARNESS):
            tmp = base / name
            tmp.mkdir()
            ok = check(name, a, backend, tmp) and ok
    finally:
        if not a.keep:
            shutil.rmtree(base, ignore_errors=True)
    print("OK" if ok else "FAIL")
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main())
