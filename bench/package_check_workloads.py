"""The packages `bench/package_check.py` checks: their pinned archives, layouts,
test suites and timed workloads, and the training workload `--select profile`
profiles (`training`). Moved out of `package_check.py` unchanged."""

import dataclasses
import os
import sys


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
for name in filter(None, sys.argv[3].split(",")):
    m = importlib.import_module(name)
    loader = type(m.__spec__.loader).__name__
    # `selected`: its selected functions compiled, its bytecode run from `.py`.
    if sys.argv[2] == "selected" and loader not in ("ExtensionFileLoader", "_PyccFinder"):
        sys.exit(f"{m.__name__} was imported from {m.__file__} ({loader}), "
                 "not a compiled module")
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
            # `--select all`: 351.6M instructions per call on CI (CPython
            # running the sources: 873.8M locally); the bound is 5% above.
            max_instructions=369_100_000,
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
            # test_misc's lazy-import test (CPython 3.15) runs `import tomli`
            # in a child interpreter, which finds the tree by PYTHONPATH.
            tests="""
import os
os.environ["PYTHONPATH"] = sys.argv[1]
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
            # `--select all`: 6.35M instructions per call on CI; the bound is
            # 5% above. The whole-module build's typed bodies ran 2.79M (the
            # comment above); the generic bodies execute more than CPython on
            # the sources (62.2M against 44.7M off CI, `loads()` of
            # `Cargo.lock`): every `src[pos]`, `pos += 1` and `char == "x"`
            # is an object operation.
            max_instructions=6_670_000,
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
            # `--select all`: 355.9M instructions per call on CI; the bound is
            # 5% above. CPython running the sources executes fewer (248.7M
            # locally, against 333.9M compiled): the generic bodies of the
            # byte loops cost more than the interpreter. (The whole-module
            # build's native `bytearray` specializations ran 21.2M.)
            max_instructions=373_700_000,
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
            # `--select all`: 76.1M instructions per call on CI; the bound is
            # 5% above.
            max_instructions=79_900_000,
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
            # `--select all`: 0.56M instructions per call on CI (0.50M locally,
            # where CPython running the sources executes 0.37M); the bound is
            # 5% above, rounded up.
            max_instructions=590_000,
            instruction_calls=100,
            more_speed=(("import pystache (once per fresh process)", import_timed("pystache")),),
        ),
    ]
}

def training(pkg: Package) -> str:
    """The training workload `--select profile` profiles: the package's own test suite,
    then each of its timed workloads (`speed`, `more_speed`, but an import, which the
    suite ran) run once, with the tree as argv[1] (and 1 as argv[2]), in one process.
    The tree is the one `pycc profile` runs (`PYCC_PROFILE_ROOT`, the driver's): in
    pass 3 a built copy, under which a suite that discovers its tests by path finds
    them. One profile of all of them, so that a function hot only
    in the timed paths (diff-match-patch's `diff_bisect`) is measured there too; pass 3
    times this same workload. A workload's output goes to devnull, and a failing suite
    still fails the run, after the timed workloads."""
    # An import workload (`import_timed`) times the first import of a fresh process:
    # in this one the suite has imported the package already, and its import ran there.
    timed = [pkg.speed, *(code for _, code in pkg.more_speed)]
    parts = [pkg.tests, *(code for code in timed if code != import_timed(pkg.import_name))]
    return (
        "import contextlib, io, os, sys, unittest\n"
        "TREE = os.environ.get('PYCC_PROFILE_ROOT', sys.argv[1])\n"
        "sys.path.insert(0, TREE)\n"
        f"PARTS = {parts!r}\n"
        "status = 0\n"
        "for i, code in enumerate(PARTS):\n"
        "    sys.argv = ['train', TREE] if i == 0 else ['train', TREE, '1']\n"
        "    try:\n"
        "        with open(os.devnull, 'w') as null, contextlib.redirect_stdout(null):\n"
        "            exec(compile(code, f'<training {i}>', 'exec'), {'__name__': '__main__', 'sys': sys, 'unittest': unittest})\n"
        "    except SystemExit as e:\n"
        "        if i == 0 and e.code not in (0, None):\n"
        "            status = 1\n"
        "sys.exit(status)\n"
    )
