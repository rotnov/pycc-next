"""Build real PyPI packages with pycc-next and run their own test suites against them.

Each package in PACKAGES is pinned by version and by the sha256 of what is downloaded. Its
modules are built unchanged, each as its own `--ext --non-strict` extension module; the
`.py` files are removed from the tree, and the package's own unittest suite runs in
CPython against the compiled modules (and, first, against the sources). The downloaded
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

Usage: python3 bench/package_check.py [PACKAGE ...] [--pycc PATH] [--cache DIR]
                                      [--keep DIR] [--time] [--runs N]
                                      [--max-build-seconds S] [--instructions]
  PACKAGE  names from PACKAGES (default: all of them).
  --time   also time each package's hot path compiled and in CPython.
  --instructions
           also count, under valgrind's callgrind, the instructions one call
           of each package's hot path executes against the compiled modules
           (with N = 1 and N = 1 + K calls; per call = the difference / K),
           and fail a package over its bound (`max_instructions`). Unlike
           wall time, the count is the same on every run of a shared CI
           runner, so it gates the code the compiler generates for the hot
           path; it cannot see what only changes speed (code placement).
           The bounds are for CI's Linux x86_64 runner and its CPython
           (actions/setup-python 3.13), which itself executes more
           instructions than Ubuntu's own python3.13: the counts there are
           1-10% lower (for tomli, half of whose `loads()` is CPython's).
  --max-build-seconds S
           fail a package whose modules take more than S seconds to build in
           all (v0.5 accept 4: 10 on the CI Linux runner). Each module's build
           time and the package's total are always printed.
Exit status 0 only when every module of every package builds (within the bound,
when one is given) and every suite passes.
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
import time
import urllib.request

ROOT = pathlib.Path(__file__).resolve().parent.parent


@dataclasses.dataclass(frozen=True)
class Package:
    name: str
    version: str
    url: str
    # The cached archive's file name.
    archive: str
    # The archive's sha256, and optionally (`tree_sha256`) the sha256 of the files
    # under `layout`'s sources (see `tree_digest`).
    sha256: str | None
    tree_sha256: str | None
    # Tree path -> path inside the extracted archive. The first entry is the package
    # (a directory), or a single top-level module (a `.py` file).
    layout: tuple[tuple[str, str], ...]
    # The import package (or module), and its modules (the tests stay Python: they are
    # the oracle).
    import_name: str
    modules: tuple[str, ...]
    # Code run in the isolated interpreter, with the tree as argv[1]: runs the suite.
    tests: str
    # Code timing the hot path, with the tree as argv[1] and the run count as argv[2];
    # prints the best time in seconds.
    speed: str
    speed_label: str
    # Code calling the hot path argv[2] times, with the tree as argv[1], for
    # `--instructions`; the bound on the instructions per call, and K.
    instructions: str
    instructions_label: str
    max_instructions: int
    instruction_calls: int


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

# pypng's hot path: a 256x256 RGB image whose rows cycle through the five PNG
# filter types, so that decoding undoes every filter (pypng itself writes
# unfiltered rows), and `decode()`, which reads it back.
PYPNG_IMAGE = """
import gc, io, struct, sys, time, zlib
sys.path.insert(0, sys.argv[1])
import png
w = h = 256
rows = [bytes((x * 7 + y * 3 + (x * y >> 5)) & 0xFF for x in range(w * 3))
        for y in range(h)]
def paeth(a, b, c):
    p = a + b - c
    pa, pb, pc = abs(p - a), abs(p - b), abs(p - c)
    return a if pa <= pb and pa <= pc else (b if pb <= pc else c)
raw = bytearray()
prev = bytes(w * 3)
for y, row in enumerate(rows):
    kind = y % 5
    raw.append(kind)
    for i, x in enumerate(row):
        a = row[i - 3] if i >= 3 else 0
        c = prev[i - 3] if i >= 3 else 0
        p = (0, a, prev[i], (a + prev[i]) >> 1, paeth(a, prev[i], c))[kind]
        raw.append((x - p) & 0xFF)
    prev = row
out = io.BytesIO()
png.write_chunks(out, [(b"IHDR", struct.pack("!2I5B", w, h, 8, 2, 0, 0, 0)),
                       (b"IDAT", zlib.compress(bytes(raw))), (b"IEND", b"")])
data = out.getvalue()
def decode():
    return [bytes(r) for r in png.Reader(bytes=data).read()[2]]
assert decode() == rows
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
            # The first 2000 characters of the speed test's texts: almost all
            # of a call is `diff_bisect`'s native loop (with the runtime's
            # `str` indexing). 126.9M instructions per call at 0.5.0 on CI
            # (125.6M with Ubuntu's python3.13); the bound is 5% above.
            instructions="""
import sys, pkgutil
sys.path.insert(0, sys.argv[1])
import diff_match_patch as d
a = pkgutil.get_data("diff_match_patch.tests", "speedtest1.txt").decode()[:2000]
b = pkgutil.get_data("diff_match_patch.tests", "speedtest2.txt").decode()[:2000]
dmp = d.diff_match_patch()
dmp.Diff_Timeout = 0.0
for _ in range(int(sys.argv[2])):
    dmp.diff_main(a, b, False)
""",
            instructions_label="diff_main of the speed test's first 2000 characters",
            max_instructions=133_300_000,
            instruction_calls=2,
        ),
        Package(
            name="tomli",
            version="2.5.0",
            url="https://github.com/hukkin/tomli/archive/refs/tags/2.5.0.tar.gz",
            archive="tomli-2.5.0-source.tar.gz",
            # GitHub's archive as CI downloaded it; the tree digest also pins the
            # files used, should GitHub ever regenerate the archive.
            sha256="8d4b735554696bf2b7e3d4505e069f6750b80e4c035fe5f1566f2a241d34dcc3",
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
            # 2.79M instructions per call at 0.5.0 on CI; the bound is 5%
            # above. With Ubuntu's python3.13 (ROADMAP v0.5 accept 1's
            # method, at most 2.60M): 2.53M; 2.95M when its frozenset
            # constants had become CPython objects.
            instructions="""
import os, sys
sys.path.insert(0, sys.argv[1])
import tomli
with open(os.path.join(sys.argv[1], "benchmark", "data.toml"), "rb") as f:
    doc = f.read().decode()
for _ in range(int(sys.argv[2])):
    tomli.loads(doc)
""",
            instructions_label="loads() of the benchmark document",
            max_instructions=2_930_000,
            instruction_calls=20,
        ),
        Package(
            name="pypng",
            version="0.20220715.0",
            url="https://files.pythonhosted.org/packages/source/p/pypng/"
            "pypng-0.20220715.0.tar.gz",
            archive="pypng-0.20220715.0.tar.gz",
            sha256="739c433ba96f078315de54c0db975aee537cbc3e1d0ae4ed9aab0ca1e427e2c1",
            tree_sha256=None,
            layout=(
                ("png.py", "pypng-0.20220715.0/code/png.py"),
                ("test_png.py", "pypng-0.20220715.0/code/test_png.py"),
                ("pngsuite.py", "pypng-0.20220715.0/code/pngsuite.py"),
            ),
            import_name="png",
            modules=("png.py",),
            # `test_test_dir` runs the sdist's shell scripts, which are not copied:
            # it skips itself.
            tests="""
import test_png
r = unittest.main(module=test_png, argv=["package_check"], exit=False, verbosity=1).result
sys.exit(0 if r.wasSuccessful() and r.testsRun > 0 else 1)
""",
            speed=PYPNG_IMAGE + """
best = float("inf")
for _ in range(int(sys.argv[2])):
    gc.collect()
    t = time.perf_counter()
    for _ in range(5):
        decode()
    best = min(best, time.perf_counter() - t)
print(f"{best:.4f}")
""",
            speed_label="read() of a 256x256 RGB image using all five filters x5",
            instructions=PYPNG_IMAGE + """
for _ in range(int(sys.argv[2])):
    decode()
""",
            instructions_label="read() of the 256x256 RGB image",
            # 21.2M instructions per call at 0.5.0 on CI (20.7M with
            # Ubuntu's python3.13); the bound is 5% above.
            max_instructions=22_200_000,
            instruction_calls=2,
        ),
    ]
}


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


def fetch(pkg: Package, cache: pathlib.Path) -> pathlib.Path:
    cache.mkdir(parents=True, exist_ok=True)
    path = cache / pkg.archive
    if pkg.sha256 is not None and path.exists() \
            and hashlib.sha256(path.read_bytes()).hexdigest() != pkg.sha256:
        path.unlink()
    if not path.exists():
        with urllib.request.urlopen(pkg.url, timeout=60) as r:
            data = r.read()
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


def count_defs(src: str) -> int:
    return len(re.findall(r"^\s*def ", src, re.M))


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

    defs = fell = 0
    build_seconds = 0.0
    for mod in pkg.modules:
        py = pkg_dir / mod
        out = py.with_suffix("") if mod != "__init__.py" else py.parent / "__init__"
        started = time.perf_counter()
        b = subprocess.run([a.pycc, "build", str(py), "-o", str(out), "--ext", "--non-strict",
                            "--release", "--foreign-relative-imports", "--report-fallbacks"],
                           capture_output=True, text=True, timeout=900)
        seconds = time.perf_counter() - started
        build_seconds += seconds
        print(f"  {mod}: built in {seconds:.2f}s")
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
    print(f"  {len(pkg.modules)} modules built in {build_seconds:.2f}s; "
          f"{native}/{defs} functions native")
    if a.max_build_seconds is not None and build_seconds > a.max_build_seconds:
        print(f"FAIL: {pkg.name} took {build_seconds:.2f}s to build, more than "
              f"{a.max_build_seconds:g}s")
        return False

    names = ",".join(
        pkg.import_name if single or m == "__init__.py" else f"{pkg.import_name}.{m[:-3]}"
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
    if a.instructions:
        per_call = instructions_per_call(pkg, built, cwd)
        if per_call is None:
            return False
        print(f"  {pkg.instructions_label}: {per_call / 1e6:.2f}M instructions per call "
              f"(at most {pkg.max_instructions / 1e6:.2f}M)")
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
    ap.add_argument("--pycc", default=str(target / "release/pycc"))
    ap.add_argument("--cache", default=str(target / "package_check"))
    ap.add_argument("--keep", help="build in this (new) directory and keep it")
    ap.add_argument("--time", action="store_true")
    ap.add_argument("--runs", type=int, default=5)
    ap.add_argument("--max-build-seconds", type=float)
    ap.add_argument("--instructions", action="store_true")
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
