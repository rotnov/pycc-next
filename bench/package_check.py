"""Build real PyPI packages with pycc-next and run their own test suites against them.

Each package in PACKAGES is pinned by version and by the sha256 of what is downloaded. Its
modules are built unchanged with `--ext --non-strict`: a package (a directory) whole, into
one extension `pkg/__init__.abi3.so` holding every module (`pycc build PKG --ext`, v0.8;
`--separate` builds each module as its own extension instead), a single-module package
as its own extension; the `.py` files are removed from the tree, and the package's own
unittest suite runs in CPython against the compiled modules (and, first, against the
sources). The downloaded
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
                                      [--max-build-seconds S] [--jobs N]
                                      [--instructions] [--separate] [--json PATH]
                                      [--competitors]
  PACKAGE  names from PACKAGES (default: all of them).
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
           all (v0.5 accept 4: 10 on the CI Linux runner). The modules build
           in parallel, as the `pycc.build` backend builds them (--jobs, by
           default one per CPU); each module's build time, the package's
           elapsed time (what the bound applies to) and the sum of the
           modules' times are always printed.
  --jobs N  build N modules at a time.
  --separate
           build each module of a package as its own extension, as before
           v0.8's whole-package build (for comparison).
Exit status 0 only when every module of every package builds (within the bound,
when one is given) and every suite passes.
"""
import argparse
import concurrent.futures
import dataclasses
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
import urllib.request

ROOT = pathlib.Path(__file__).resolve().parent.parent
sys.path.insert(0, str(ROOT / "bench"))
import competitors  # noqa: E402
import stats  # noqa: E402
# The compiler's file name in a Cargo profile directory.
PYCC_EXE = "pycc.exe" if os.name == "nt" else "pycc"


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
    # prints the best time in seconds (`--time` passes 1: one untimed call, one timed).
    speed: str
    speed_label: str
    # Code calling the hot path argv[2] times, with the tree as argv[1], for
    # `--instructions`; the bound on the instructions per call, and K.
    instructions: str
    instructions_label: str
    max_instructions: int
    instruction_calls: int
    # More hot paths `--time` times, as (label, code) pairs like `speed`.
    more_speed: tuple[tuple[str, str], ...] = ()
    # Modules and subpackages of the package that stay Python (`--exclude`).
    excludes: tuple[str, ...] = ()


# Prefixed to `Package.tests`: puts the tree on the path, imports every module and checks
# that each comes from a compiled extension when argv[2] is "compiled".
CHECK_COMPILED = """
import importlib, sys, unittest
sys.path.insert(0, sys.argv[1])
for name in sys.argv[3].split(","):
    m = importlib.import_module(name)
    loader = type(m.__spec__.loader).__name__
    if sys.argv[2] == "compiled" and (loader not in ("ExtensionFileLoader", "_PyccFinder")
                                      or not m.__file__.endswith((".so", ".pyd"))):
        sys.exit(f"{m.__name__} was imported from {m.__file__} ({loader}), "
                 "not a compiled module")
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

# pypng's other hot path: decoding every PngSuite image (`pngsuite`, the module
# its tests read them from; all 28 of them, since `test_png` iterates
# `pngsuite.png`) and iterating its rows.
PYPNG_SUITE = """
import gc, sys, time
sys.path.insert(0, sys.argv[1])
import png, pngsuite
images = list(pngsuite.png.values())
def decode():
    for data in images:
        for row in png.Reader(bytes=data).read()[2]:
            pass
decode()
best = float("inf")
for _ in range(int(sys.argv[2])):
    gc.collect()
    t = time.perf_counter()
    for _ in range(5):
        decode()
    best = min(best, time.perf_counter() - t)
print(f"{best:.4f}")
"""

# mistune's modules: all of the package (its command-line `__main__` too).
MISTUNE_MODULES = (
    "__init__.py", "__main__.py", "_inline/__init__.py", "_inline/emphasis.py",
    "_inline/links.py", "block_parser.py", "core.py", "directives/__init__.py",
    "directives/_base.py", "directives/_fenced.py", "directives/_rst.py",
    "directives/admonition.py", "directives/image.py", "directives/include.py",
    "directives/toc.py", "helpers.py", "inline_parser.py", "list_parser.py", "markdown.py",
    "plugins/__init__.py", "plugins/abbr.py", "plugins/def_list.py", "plugins/footnotes.py",
    "plugins/formatting.py", "plugins/math.py", "plugins/ruby.py", "plugins/speedup.py",
    "plugins/spoiler.py", "plugins/table.py", "plugins/task_lists.py", "plugins/url.py",
    "renderers/__init__.py", "renderers/_list.py", "renderers/html.py",
    "renderers/markdown.py", "renderers/rst.py", "toc.py", "util.py",
)

# mistune's own benchmark (`benchmark/bench.py`): its documents, each rendered
# by a parser `md` (the benchmark runs each case 100 times; here every case,
# 20 times over).
MISTUNE_CASES = """
import gc, os, sys, time
sys.path.insert(0, sys.argv[1])
import mistune
cases = os.path.join(sys.argv[1], "benchmark", "cases")
docs = []
for name in sorted(os.listdir(cases)):
    with open(os.path.join(cases, name), encoding="utf-8") as f:
        docs.append(f.read())
"""
# The benchmark's "mistune (full)" parser: every plugin and the directives.
MISTUNE_FULL = """
from mistune.directives import RSTDirective, Admonition, TableOfContents, Include
md = mistune.create_markdown(escape=False, plugins=[
    "url", "abbr", "ruby", "strikethrough", "mark", "insert", "subscript", "superscript",
    "footnotes", "def_list", "math", "table", "task_lists",
    RSTDirective([Admonition(), TableOfContents(), Include()])])
"""
MISTUNE_TIMED = """
for doc in docs:
    md(doc)
best = float("inf")
for _ in range(int(sys.argv[2])):
    gc.collect()
    t = time.perf_counter()
    for _ in range(20):
        for doc in docs:
            md(doc)
    best = min(best, time.perf_counter() - t)
print(f"{best:.4f}")
"""

# pystache's modules: the package but its tests (a subpackage of it, which stays
# Python).
PYSTACHE_MODULES = (
    "__init__.py", "_version.py", "commands/__init__.py", "commands/render.py",
    "commands/test.py", "common.py", "context.py", "defaults.py", "init.py", "loader.py",
    "locator.py", "parsed.py", "parser.py", "renderengine.py", "renderer.py",
    "specloader.py", "template_spec.py",
)

# pystache's own benchmark (`pystache/tests/benchmark.py`): its examples, each a
# `pystache.render()` of a template checked against the expected output (the script
# itself times each example with `timeit`; here all of them, 1000 times over).
PYSTACHE_BENCHMARK = """
import contextlib, gc, io, sys, time
sys.path.insert(0, sys.argv[1])
with contextlib.redirect_stdout(io.StringIO()):
    from pystache.tests import benchmark
tests = [benchmark.make_test_function(example) for example in benchmark.examples]
for test in tests:
    test()
"""


def import_timed(name: str) -> str:
    """The import workload (v0.11 accept 4): `import NAME` in a fresh `-I` process,
    timed once (an import runs once per process; argv[2] is ignored), with nothing
    imported before it but what the interpreter imports at startup and `sys` and
    `time`, so the stdlib modules the package imports count on every side alike. The
    discarded warm-up process per tree writes the CPython tree's `.pyc` files: CPython
    imports the package from its bytecode cache, as an installed package does. Prints
    the seconds."""
    return f"""
import sys, time
sys.path.insert(0, sys.argv[1])
t = time.perf_counter()
import {name}
t = time.perf_counter() - t
assert {name}.__file__.startswith(sys.argv[1]), {name}.__file__
print(f"{{t:.6f}}")
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
            excludes=("tests",),
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
            # (125.6M with Ubuntu's python3.13); the bound was 5% above.
            # Re-baselined in v0.8 to 135.2M, 2% above CI's 132.5M (run
            # 39109040009): about 1.3M of those are the nops that now pad
            # jumps off 32-byte boundaries (prefix padding doubled LLVM's
            # emission time), with wall time unchanged within 0.5% (best of
            # 15 interleaved runs, Skylake-class Xeon).
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
            max_instructions=135_200_000,
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
            more_speed=(("read() of every PngSuite image its tests use (28) x5", PYPNG_SUITE),),
        ),
        Package(
            name="mistune",
            version="3.3.4",
            url="https://files.pythonhosted.org/packages/source/m/mistune/mistune-3.3.4.tar.gz",
            archive="mistune-3.3.4.tar.gz",
            sha256="58b5c96d6fcb61190dfe5fae498d2b2065f99cf61e9649418fd54cf1ada86dfe",
            tree_sha256=None,
            layout=(
                ("mistune", "mistune-3.3.4/src/mistune"),
                ("tests", "mistune-3.3.4/tests"),
                ("benchmark", "mistune-3.3.4/benchmark"),
                ("pyproject.toml", "mistune-3.3.4/pyproject.toml"),
            ),
            import_name="mistune",
            modules=MISTUNE_MODULES,
            # From the tree, as from the project's root: a test reads its
            # `pyproject.toml`.
            tests="""
import os
os.chdir(sys.argv[1])
r = unittest.main(module=None, exit=False, verbosity=1, argv=[
    "package_check", "discover", "-s", os.path.join(sys.argv[1], "tests"),
    "-t", sys.argv[1]]).result
sys.exit(0 if r.wasSuccessful() and r.testsRun > 0 else 1)
""",
            speed=MISTUNE_CASES + """
md = mistune.html
""" + MISTUNE_TIMED,
            speed_label="mistune.html of every benchmark/cases document x20",
            instructions=MISTUNE_CASES + """
for _ in range(int(sys.argv[2])):
    for doc in docs:
        mistune.html(doc)
""",
            instructions_label="mistune.html of every benchmark/cases document",
            # 77.6M instructions per call at the first measurement (Ubuntu's
            # python3.13; CPython running the sources: see the README); the
            # bound leaves room for CI's CPython until it is measured there.
            max_instructions=85_400_000,
            instruction_calls=2,
            more_speed=(("the benchmark's \"mistune (full)\" parser (every plugin) x20",
                         MISTUNE_CASES + MISTUNE_FULL + MISTUNE_TIMED),
                        ("import mistune (once per fresh process)", import_timed("mistune"))),
        ),
        Package(
            name="pystache",
            version="0.6.8",
            url="https://files.pythonhosted.org/packages/source/p/pystache/pystache-0.6.8.tar.gz",
            archive="pystache-0.6.8.tar.gz",
            sha256="3707518e6a4d26dd189b07c10c669b1fc17df72684617c327bd3550e7075c72c",
            tree_sha256=None,
            layout=(("pystache", "pystache-0.6.8/pystache"),),
            import_name="pystache",
            modules=PYSTACHE_MODULES,
            # The package's own tests are a subpackage of it: they stay Python.
            excludes=("tests",),
            tests="""
import os
r = unittest.main(module=None, exit=False, verbosity=1, argv=[
    "package_check", "discover", "-s", os.path.join(sys.argv[1], "pystache", "tests"),
    "-t", sys.argv[1]]).result
sys.exit(0 if r.wasSuccessful() and r.testsRun > 0 else 1)
""",
            speed=PYSTACHE_BENCHMARK + """
best = float("inf")
for _ in range(int(sys.argv[2])):
    gc.collect()
    t = time.perf_counter()
    for _ in range(1000):
        for test in tests:
            test()
    best = min(best, time.perf_counter() - t)
print(f"{best:.4f}")
""",
            speed_label="the package's own benchmark (tests/benchmark.py) examples x1000",
            instructions=PYSTACHE_BENCHMARK + """
for _ in range(int(sys.argv[2])):
    for test in tests:
        test()
""",
            instructions_label="the package's own benchmark examples, each rendered once",
            # 0.83M instructions at the first measurement (Ubuntu's python3.13;
            # CPython running the sources: 0.50M); the bound leaves room for CI's
            # CPython until it is measured there.
            max_instructions=1_100_000,
            instruction_calls=100,
            more_speed=(("import pystache (once per fresh process)", import_timed("pystache")),),
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

    defs = fell = 0

    flags = ["--ext", "--non-strict", "--release", "--report-fallbacks"]

    def build(mod: str) -> tuple[subprocess.CompletedProcess, float]:
        py = pkg_dir / mod
        out = py.with_suffix("") if mod != "__init__.py" else py.parent / "__init__"
        started = time.perf_counter()
        b = subprocess.run([a.pycc, "build", str(py), "-o", str(out), *flags,
                            "--foreign-relative-imports"],
                           capture_output=True, text=True, timeout=900)
        return b, time.perf_counter() - started

    env = dict(os.environ, PYCC_JOBS=str(a.jobs))
    started = time.perf_counter()
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
    for mod, (b, seconds) in zip(pkg.modules, results):
        print(f"  {mod}: built in {seconds:.2f}s")
        if b.returncode != 0:
            print(b.stdout + b.stderr)
            print(f"FAIL: pycc could not build {pkg.import_name}/{mod}")
            return False
        source = (pkg_dir / mod).read_text(encoding="utf-8")
        defs += count_defs(source)
        lines = source.splitlines()
        # A whole-package build reports every module's fallbacks: this
        # module's name its file.
        for m in re.finditer(r"^fallback: .*? at (.*?):(\d+)(?:-(\d+))? runs in CPython: (.*)$",
                             b.stderr, re.M):
            if not pathlib.Path(m[1]).as_posix().endswith(f"/{pkg_dir.name}/{mod}") \
                    and not single:
                continue
            lo = int(m[2])
            hi = int(m[3] or m[2])
            n = count_defs("\n".join(lines[lo - 1:hi]))
            fell += n
            print(f"  {mod}:{lo}-{hi} in CPython ({n} functions): {m[4]}")
        if single or a.separate:
            for line in b.stderr.splitlines():
                if line.startswith(("specialized:", "not specialized:")):
                    print(f"  {mod}: {line}")
    if not (single or a.separate):
        for line in dict.fromkeys(results[0][0].stderr.splitlines()):
            if line.startswith(("specialized:", "not specialized:")):
                print(f"  {line}")
    # Only now: a module's dotted name is found from the `__init__.py`
    # files beside and above it.
    for mod in pkg.modules:
        (pkg_dir / mod).unlink()
    native = defs - fell
    cpu = sum(seconds for _, seconds in results)
    print(f"  {len(pkg.modules)} modules built in {build_seconds:.2f}s "
          f"({cpu:.2f}s of builds, {a.jobs} at a time); {native}/{defs} functions native")
    if a.max_build_seconds is not None and build_seconds > a.max_build_seconds:
        print(f"FAIL: {pkg.name} took {build_seconds:.2f}s to build, more than "
              f"{a.max_build_seconds:g}s")
        return False

    # A module's dotted name: `a/b.py` is `<package>.a.b`, `a/__init__.py` is `<package>.a`.
    names = ",".join(
        pkg.import_name if single else
        ".".join([pkg.import_name, *pathlib.PurePosixPath(m).with_suffix("").parts])
        .removesuffix(".__init__")
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
    ap.add_argument("--pycc", default=str(target / "release" / PYCC_EXE))
    ap.add_argument("--cache", default=str(target / "package_check"))
    ap.add_argument("--keep", help="build in this (new) directory and keep it")
    ap.add_argument("--time", action="store_true")
    ap.add_argument("--runs", type=int, default=5)
    ap.add_argument("--max-build-seconds", type=float)
    ap.add_argument("--jobs", type=int, default=os.cpu_count() or 1)
    ap.add_argument("--instructions", action="store_true")
    ap.add_argument("--separate", action="store_true")
    ap.add_argument("--json")
    ap.add_argument("--competitors", action="store_true")
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
    if a.json:
        doc: dict = {"packages": TIMINGS}
        if a.competitors:
            doc["competitors"] = {"versions": competitors.versions(), "builds": BUILDS}
        pathlib.Path(a.json).write_text(json.dumps(doc, indent=1))
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main())
