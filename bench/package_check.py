"""Build real PyPI packages with pycc-next and run their own test suites against them.

Each package in PACKAGES is pinned by version and by the sha256 of what is downloaded. Its
modules are built unchanged, each as its own `--ext --non-strict` extension module; the
`.py` files are removed from the tree, and the package's own unittest suite runs in
CPython against the compiled modules (and, first, against the sources). The downloaded
archive is untrusted data: it is extracted with tarfile's `data` filter into a fresh
directory, and every Python process that imports from it runs isolated (`-I`), from a
directory of its own.

Packages:
  diff-match-patch 20241021 (Apache-2.0): Google's diff/match/patch text algorithms. The
    PyPI sdist ships the module and its unittest suite.
  tomli 2.5.0 (MIT): a TOML parser (the source of the standard library's `tomllib`). Its
    sdist has no tests, so the tagged GitHub source archive is used: package, unittest
    suite with the toml-test data, and the benchmark document. GitHub regenerates its
    archives, whose compression is not byte-stable, so the files used are pinned by a
    sha256 over their paths and contents instead.

Usage: python3 bench/package_check.py [PACKAGE ...] [--pycc PATH] [--cache DIR]
                                      [--keep DIR] [--time] [--runs N]
  PACKAGE  names from PACKAGES (default: all of them).
  --time   also time each package's hot path compiled and in CPython.
Exit status 0 only when every module of every package builds and every suite passes.
"""
import argparse
import dataclasses
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


@dataclasses.dataclass(frozen=True)
class Package:
    name: str
    version: str
    url: str
    # The cached archive's file name.
    archive: str
    # Either the archive's sha256, or (`tree_sha256`) the sha256 of the files under
    # `layout`'s sources (see `tree_digest`).
    sha256: str | None
    tree_sha256: str | None
    # Tree path -> path inside the extracted archive. The first entry is the package.
    layout: tuple[tuple[str, str], ...]
    # The import package, and its modules (the tests stay Python: they are the oracle).
    import_name: str
    modules: tuple[str, ...]
    # Code run in the isolated interpreter, with the tree as argv[1]: runs the suite.
    tests: str
    # Code timing the hot path, with the tree as argv[1] and the run count as argv[2];
    # prints the best time in seconds.
    speed: str
    speed_label: str


# Prefixed to `Package.tests`: puts the tree on the path, imports every module and checks
# that each comes from a compiled extension when argv[2] is "compiled".
CHECK_COMPILED = """
import importlib, sys, unittest
sys.path.insert(0, sys.argv[1])
for name in sys.argv[3].split(","):
    m = importlib.import_module(name)
    if sys.argv[2] == "compiled" and not m.__file__.endswith((".so", ".pyd")):
        sys.exit(f"{m.__name__} was imported from {m.__file__}, not a compiled module")
"""

PACKAGES = {
    p.name: p
    for p in [
        Package(
            name="diff-match-patch",
            version="20241021",
            url="https://files.pythonhosted.org/packages/source/d/diff-match-patch/"
            "diff_match_patch-20241021.tar.gz",
            archive="diff_match_patch-20241021.tar.gz",
            sha256="beae57a99fa48084532935ee2968b8661db861862ec82c6f21f4acdd6d835073",
            tree_sha256=None,
            layout=(("diff_match_patch", "diff_match_patch-20241021/diff_match_patch"),),
            import_name="diff_match_patch",
            modules=("__init__.py", "__version__.py", "diff_match_patch.py"),
            tests="""
from diff_match_patch.tests import diff_match_patch_test as t
r = unittest.main(module=t, argv=["package_check"], exit=False, verbosity=1).result
sys.exit(0 if r.wasSuccessful() and r.testsRun > 0 else 1)
""",
            speed="""
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
""",
            speed_label="diff_main speed test (two ~30 KB texts)",
        ),
        Package(
            name="tomli",
            version="2.5.0",
            url="https://github.com/hukkin/tomli/archive/refs/tags/2.5.0.tar.gz",
            archive="tomli-2.5.0-source.tar.gz",
            sha256=None,
            tree_sha256="8bbb6589d151200e8aef88b32e66b46ffcfbd0606a25964bdac68299d987a506",
            layout=(
                ("tomli", "tomli-2.5.0/src/tomli"),
                ("tests", "tomli-2.5.0/tests"),
                ("benchmark/data.toml", "tomli-2.5.0/benchmark/data.toml"),
            ),
            import_name="tomli",
            modules=("__init__.py", "_parser.py", "_re.py", "_types.py"),
            tests="""
import os
r = unittest.main(module=None, exit=False, verbosity=1, argv=[
    "package_check", "discover", "-s", os.path.join(sys.argv[1], "tests"),
    "-t", sys.argv[1]]).result
sys.exit(0 if r.wasSuccessful() and r.testsRun > 0 else 1)
""",
            speed="""
import gc, os, sys, time
sys.path.insert(0, sys.argv[1])
import tomli
with open(os.path.join(sys.argv[1], "benchmark", "data.toml"), "rb") as f:
    doc = f.read().decode()
tomli.loads(doc)
best = float("inf")
for _ in range(int(sys.argv[2])):
    gc.collect()
    t = time.perf_counter()
    for _ in range(100):
        tomli.loads(doc)
    best = min(best, time.perf_counter() - t)
print(f"{best:.4f}")
""",
            speed_label="loads() of the benchmark document x100",
        ),
    ]
}


def tree_digest(base: pathlib.Path, paths: list[str]) -> str:
    """sha256 over the relative path and contents of every file under `paths`."""
    h = hashlib.sha256()
    files = []
    for rel in paths:
        p = base / rel
        files.extend([p] if p.is_file() else sorted(q for q in p.rglob("*") if q.is_file()))
    for f in sorted(files, key=lambda q: q.relative_to(base).as_posix()):
        h.update(f.relative_to(base).as_posix().encode() + b"\0")
        h.update(hashlib.sha256(f.read_bytes()).digest())
    return h.hexdigest()


def fetch(pkg: Package, cache: pathlib.Path) -> pathlib.Path:
    cache.mkdir(parents=True, exist_ok=True)
    path = cache / pkg.archive
    if pkg.sha256 is not None and path.exists() \
            and hashlib.sha256(path.read_bytes()).hexdigest() != pkg.sha256:
        path.unlink()
    if not path.exists():
        with urllib.request.urlopen(pkg.url, timeout=60) as r:
            data = r.read()
        if pkg.sha256 is not None:
            got = hashlib.sha256(data).hexdigest()
            if got != pkg.sha256:
                sys.exit(f"sha256 mismatch for {pkg.url}: got {got}, want {pkg.sha256}")
        path.write_bytes(data)
    return path


def unpack(pkg: Package, archive: pathlib.Path, dest: pathlib.Path) -> pathlib.Path:
    """Extracts `archive` and checks its tree digest; returns the extracted root."""
    with tarfile.open(archive) as tar:
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


def count_defs(src: str) -> int:
    return len(re.findall(r"^\s*def ", src, re.M))


def check(pkg: Package, a: argparse.Namespace, tmp: pathlib.Path) -> bool:
    print(f"{pkg.name} {pkg.version}")
    archive = fetch(pkg, pathlib.Path(a.cache))
    src = unpack(pkg, archive, tmp / "archive")
    plain = tmp / "cpython"
    built = tmp / "compiled"
    for tree in (plain, built):
        for dst, rel in pkg.layout:
            (tree / dst).parent.mkdir(parents=True, exist_ok=True)
            copy = shutil.copytree if (src / rel).is_dir() else shutil.copyfile
            copy(src / rel, tree / dst)
    cwd = tmp / "cwd"
    cwd.mkdir()
    pkg_dir = built / pkg.layout[0][0]

    defs = fell = 0
    for mod in pkg.modules:
        py = pkg_dir / mod
        out = py.with_suffix("") if mod != "__init__.py" else py.parent / "__init__"
        b = subprocess.run([a.pycc, "build", str(py), "-o", str(out), "--ext", "--non-strict",
                            "--release", "--foreign-relative-imports", "--report-fallbacks"],
                           capture_output=True, text=True, timeout=900)
        if b.returncode != 0:
            print(b.stdout + b.stderr)
            print(f"FAIL: pycc could not build {pkg.import_name}/{mod}")
            return False
        source = py.read_text()
        defs += count_defs(source)
        lines = source.splitlines()
        for m in re.finditer(r"^fallback: .*? at .*?:(\d+)(?:-(\d+))? runs in CPython: (.*)$",
                             b.stderr, re.M):
            lo = int(m[1])
            hi = int(m[2] or m[1])
            n = count_defs("\n".join(lines[lo - 1:hi]))
            fell += n
            print(f"  {mod}:{lo}-{hi} in CPython ({n} functions): {m[3]}")
        for line in b.stderr.splitlines():
            if line.startswith(("specialized:", "not specialized:")):
                print(f"  {mod}: {line}")
    # Only now: a module's dotted name is found from the `__init__.py`
    # files beside and above it.
    for mod in pkg.modules:
        (pkg_dir / mod).unlink()
    native = defs - fell
    print(f"  {len(pkg.modules)} modules built; {native}/{defs} functions native")

    names = ",".join(
        pkg.import_name if m == "__init__.py" else f"{pkg.import_name}.{m[:-3]}"
        for m in pkg.modules
    )
    for label, tree in (("cpython", plain), ("compiled", built)):
        r = python(CHECK_COMPILED + pkg.tests, str(tree), label, names, cwd=cwd)
        tail = (r.stderr.strip().splitlines() or ["?"])[-1]
        print(f"  test suite ({label}): {tail}")
        if r.returncode != 0:
            print(r.stdout + r.stderr)
            print(f"FAIL: {pkg.name}'s own tests fail against the {label} modules")
            return False

    if a.time:
        times = {}
        for label, tree in (("cpython", plain), ("compiled", built)):
            r = python(pkg.speed, str(tree), str(a.runs), cwd=cwd)
            if r.returncode != 0:
                print(r.stdout + r.stderr)
                return False
            times[label] = float(r.stdout)
        print(f"  {pkg.speed_label}: CPython {times['cpython']:.3f}s, "
              f"compiled {times['compiled']:.3f}s "
              f"({times['cpython'] / times['compiled']:.2f}x)")
    return True


def main() -> int:
    ap = argparse.ArgumentParser()
    target = pathlib.Path(os.environ.get("CARGO_TARGET_DIR", ROOT / "target"))
    ap.add_argument("packages", nargs="*", metavar="PACKAGE")
    ap.add_argument("--pycc", default=str(target / "release/pycc"))
    ap.add_argument("--cache", default=str(target / "package_check"))
    ap.add_argument("--keep", help="build in this (new) directory and keep it")
    ap.add_argument("--time", action="store_true")
    ap.add_argument("--runs", type=int, default=5)
    a = ap.parse_args()
    for name in a.packages:
        if name not in PACKAGES:
            ap.error(f"unknown package {name!r} (known: {', '.join(PACKAGES)})")

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
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main())
