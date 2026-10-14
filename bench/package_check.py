"""Build a real PyPI package with pycc-next and run its own test suite against it.

Package: diff-match-patch 20241021 (Apache-2.0, pure Python, Google's diff/match/patch
text algorithms), pinned by version and sdist sha256. Its modules are built unchanged,
each as its own `--ext --non-strict` extension module; the `.py` files are removed from
the tree, and the package's own unittest suite (shipped in the sdist) runs in CPython
against the compiled modules. The downloaded archive is untrusted data: it is extracted
with tarfile's `data` filter into a fresh directory, and every Python process that
imports from it runs isolated (`-I`), from a directory of its own.

Usage: python3 bench/package_check.py [--pycc PATH] [--cache DIR] [--keep DIR] [--time]
  --time   also time the package's speed test (`diff_main` on two ~30 KB texts) compiled
           and in CPython.
Exit status 0 only when every module builds and the suite passes.
"""
import argparse
import hashlib
import os
import pathlib
import re
import shutil
import subprocess
import sys
import tarfile
import tempfile
import urllib.request

ROOT = pathlib.Path(__file__).resolve().parent.parent
NAME = "diff_match_patch"
VERSION = "20241021"
SDIST = f"{NAME}-{VERSION}.tar.gz"
URL = f"https://files.pythonhosted.org/packages/source/d/diff-match-patch/{SDIST}"
SHA256 = "beae57a99fa48084532935ee2968b8661db861862ec82c6f21f4acdd6d835073"
# Every module of the package (the tests stay Python: they are the oracle).
MODULES = ["__init__.py", "__version__.py", "diff_match_patch.py"]

# Run inside the isolated interpreter: the work tree comes in as argv[1].
RUN_TESTS = """
import sys, unittest
sys.path.insert(0, sys.argv[1])
import diff_match_patch
for name in ("diff_match_patch", "diff_match_patch.__version__", "diff_match_patch.diff_match_patch"):
    m = sys.modules[name]
    if sys.argv[2] == "compiled" and not m.__file__.endswith((".so", ".pyd")):
        sys.exit(f"{m.__name__} was imported from {m.__file__}, not a compiled module")
from diff_match_patch.tests import diff_match_patch_test as t
r = unittest.main(module=t, argv=["package_check"], exit=False, verbosity=1).result
sys.exit(0 if r.wasSuccessful() and r.testsRun > 0 else 1)
"""

SPEED = """
import gc, sys, time, pkgutil
sys.path.insert(0, sys.argv[1])
import diff_match_patch as d
a = pkgutil.get_data("diff_match_patch.tests", "speedtest1.txt").decode()
b = pkgutil.get_data("diff_match_patch.tests", "speedtest2.txt").decode()
dmp = d.diff_match_patch()
dmp.Diff_Timeout = 0.0
dmp.diff_main(b, a, False)
best = float("inf")
for _ in range(int(sys.argv[2])):
    gc.collect()
    t = time.perf_counter()
    dmp.diff_main(a, b, False)
    best = min(best, time.perf_counter() - t)
print(f"{best:.4f}")
"""


def fetch(cache: pathlib.Path) -> pathlib.Path:
    cache.mkdir(parents=True, exist_ok=True)
    path = cache / SDIST
    if not path.exists() or hashlib.sha256(path.read_bytes()).hexdigest() != SHA256:
        with urllib.request.urlopen(URL, timeout=60) as r:
            data = r.read()
        got = hashlib.sha256(data).hexdigest()
        if got != SHA256:
            sys.exit(f"sha256 mismatch for {URL}: got {got}, want {SHA256}")
        path.write_bytes(data)
    return path


def unpack(sdist: pathlib.Path, dest: pathlib.Path) -> pathlib.Path:
    with tarfile.open(sdist) as tar:
        tar.extractall(dest, filter="data")
    return dest / f"{NAME}-{VERSION}" / NAME


def python(code: str, *args: str, cwd: pathlib.Path) -> subprocess.CompletedProcess:
    return subprocess.run([sys.executable, "-I", "-c", code, *args], cwd=cwd,
                          capture_output=True, text=True, timeout=600)


def count_defs(src: str) -> int:
    return len(re.findall(r"^\s*def ", src, re.M))


def main() -> int:
    ap = argparse.ArgumentParser()
    target = pathlib.Path(os.environ.get("CARGO_TARGET_DIR", ROOT / "target"))
    ap.add_argument("--pycc", default=str(target / "release/pycc"))
    ap.add_argument("--cache", default=str(target / "package_check"))
    ap.add_argument("--keep", help="build in this (new) directory and keep it")
    ap.add_argument("--time", action="store_true")
    ap.add_argument("--runs", type=int, default=5)
    a = ap.parse_args()

    sdist = fetch(pathlib.Path(a.cache))
    tmp = pathlib.Path(a.keep) if a.keep else pathlib.Path(tempfile.mkdtemp(prefix="pycc-pkg-"))
    tmp.mkdir(parents=True, exist_ok=True)
    try:
        src = unpack(sdist, tmp / "sdist")
        plain = tmp / "cpython"
        built = tmp / "compiled"
        shutil.copytree(src, plain / NAME)
        shutil.copytree(src, built / NAME)
        cwd = tmp / "cwd"
        cwd.mkdir()

        defs = fell = 0
        for mod in MODULES:
            py = built / NAME / mod
            out = py.with_suffix("") if mod != "__init__.py" else py.parent / "__init__"
            b = subprocess.run([a.pycc, "build", str(py), "-o", str(out), "--ext", "--non-strict",
                                "--release", "--foreign-relative-imports", "--report-fallbacks"],
                               capture_output=True, text=True, timeout=900)
            if b.returncode != 0:
                print(b.stdout + b.stderr)
                print(f"FAIL: pycc could not build {NAME}/{mod}")
                return 1
            source = py.read_text()
            defs += count_defs(source)
            lines = source.splitlines()
            for m in re.finditer(r"^fallback: .*? at .*?:(\d+)-(\d+) runs in CPython: (.*)$", b.stderr, re.M):
                lo, hi = int(m[1]), int(m[2])
                n = count_defs("\n".join(lines[lo - 1:hi]))
                fell += n
                print(f"  {mod}:{lo}-{hi} in CPython ({n} functions): {m[3]}")
        # Only now: a module's dotted name is found from the `__init__.py`
        # files beside and above it.
        for mod in MODULES:
            (built / NAME / mod).unlink()
        native = defs - fell
        print(f"{NAME} {VERSION}: {len(MODULES)} modules built; {native}/{defs} functions native")

        for label, tree in (("cpython", plain), ("compiled", built)):
            r = python(RUN_TESTS, str(tree), label, cwd=cwd)
            tail = (r.stderr.strip().splitlines() or ["?"])[-1]
            print(f"  test suite ({label}): {tail}")
            if r.returncode != 0:
                print(r.stdout + r.stderr)
                print(f"FAIL: the package's own tests fail against the {label} modules")
                return 1

        if a.time:
            times = {}
            for label, tree in (("cpython", plain), ("compiled", built)):
                r = python(SPEED, str(tree), str(a.runs), cwd=cwd)
                if r.returncode != 0:
                    print(r.stdout + r.stderr)
                    return 1
                times[label] = float(r.stdout)
            print(f"  diff_main speed test: CPython {times['cpython']:.3f}s, "
                  f"compiled {times['compiled']:.3f}s "
                  f"({times['cpython'] / times['compiled']:.2f}x)")
        print("OK")
        return 0
    finally:
        if not a.keep:
            shutil.rmtree(tmp, ignore_errors=True)


if __name__ == "__main__":
    sys.exit(main())
