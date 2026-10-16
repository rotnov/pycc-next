"""Unit tests of the `pycc.build` backend: `python3 -m unittest discover -s python/tests`.

The compiler is replaced by a stand-in script (`fake pycc`) that records its arguments
and writes a minimal ELF shared object (or fails for a source holding `FAIL`), so the
backend's own behavior -- configuration, discovery, metadata, tags, RECORD, archives,
diagnostics -- is tested without building anything. `tests/pep517_backend.rs` runs the
real compiler through pip.
"""
import base64
import contextlib
import csv
import hashlib
import io
import json
import os
import pathlib
import re
import stat
import struct
import sys
import tarfile
import tempfile
import textwrap
import tomllib
import unittest
import zipfile

PYTHON_DIR = pathlib.Path(__file__).resolve().parent.parent
sys.path.insert(0, str(PYTHON_DIR))

import pycc  # noqa: E402
from pycc import _compile, _project, _wheel, build  # noqa: E402


def minimal_elf(needed: list[str], machine: int = 0x3E) -> bytes:
    """An ELF64 little-endian shared object with one PT_LOAD and a PT_DYNAMIC listing
    `needed`; nothing else."""
    strtab = b"\0"
    offsets = []
    for name in needed:
        offsets.append(len(strtab))
        strtab += name.encode() + b"\0"
    ph_off, ph_size, ph_num = 64, 56, 2
    dyn_off = ph_off + ph_size * ph_num
    entries = [(1, o) for o in offsets]
    str_off = dyn_off + 16 * (len(entries) + 2)
    entries += [(5, str_off), (0, 0)]
    total = str_off + len(strtab)
    header = b"\x7fELF" + bytes([2, 1, 1, 0]) + bytes(8)
    header += struct.pack("<HHIQQQIHHHHHH", 3, machine, 1, 0, ph_off, 0, 0, 64, ph_size,
                          ph_num, 64, 0, 0)
    load = struct.pack("<IIQQQQQQ", 1, 5, 0, 0, 0, total, total, 0x1000)
    dynamic = struct.pack("<IIQQQQQQ", 2, 6, dyn_off, dyn_off, dyn_off, 16 * len(entries),
                          16 * len(entries), 8)
    body = b"".join(struct.pack("<qQ", t, v) for t, v in entries)
    return header + load + dynamic + body + strtab


def minimal_macho(minos: tuple[int, int], cmd: int = 0x32) -> bytes:
    """A 64-bit Mach-O header with one LC_BUILD_VERSION (or LC_VERSION_MIN_MACOSX)."""
    version = (minos[0] << 16) | (minos[1] << 8)
    if cmd == 0x32:
        lc = struct.pack("<IIIIII", 0x32, 24, 1, version, version, 0)
    else:
        lc = struct.pack("<IIII", 0x24, 16, version, version)
    return struct.pack("<IiiIIIII", 0xFEEDFACF, 0x0100000C, 0, 6, 1, len(lc), 0, 0) + lc


FAKE_PYCC = """\
#!{python}
import json, os, pathlib, sys
args = sys.argv[1:]
log = pathlib.Path({log!r})
with open(log, "a") as f:
    f.write(json.dumps({{"args": args, "tmpdir": os.environ.get("TMPDIR"),
                        "python": os.environ.get("PYCC_PYTHON")}}) + "\\n")
src = pathlib.Path(args[1])
out = pathlib.Path(args[args.index("-o") + 1])
if "FAIL" in src.read_text():
    print(f"error[C0001]: not supported at {{src}}:1", file=sys.stderr)
    sys.exit(1)
pathlib.Path(str(out) + ".abi3.so").write_bytes(pathlib.Path({so!r}).read_bytes())
print(f"fallback: statement at {{src}}:1 runs in CPython: test", file=sys.stderr)
"""


@contextlib.contextmanager
def chdir(path):
    old = os.getcwd()
    os.chdir(path)
    try:
        yield
    finally:
        os.chdir(old)


@contextlib.contextmanager
def environ(**values):
    old = {k: os.environ.get(k) for k in values}
    for k, v in values.items():
        if v is None:
            os.environ.pop(k, None)
        else:
            os.environ[k] = v
    try:
        yield
    finally:
        for k, v in old.items():
            if v is None:
                os.environ.pop(k, None)
            else:
                os.environ[k] = v


def write(root: pathlib.Path, files: dict[str, str]) -> None:
    for rel, text in files.items():
        path = root / rel
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(textwrap.dedent(text))


PYPROJECT = """\
[build-system]
requires = ["pycc-build"]
build-backend = "pycc.build"

[project]
name = "Demo.Pkg"
version = "1.2.0"
description = "A demo"
readme = "README.md"
requires-python = ">=3.13"
license = {{ text = "MIT" }}
authors = [{{ name = "Ada", email = "ada@example.org" }}, {{ name = "Bob" }}]
keywords = ["a", "b"]
classifiers = ["Programming Language :: Python :: 3"]
dependencies = ["attrs>=23"]
urls = {{ Homepage = "https://example.org" }}

[project.optional-dependencies]
fast = ["numpy", "uvloop; sys_platform != 'win32'"]

[project.scripts]
demo = "demo_pkg.cli:main"

[project.entry-points."demo.plugins"]
one = "demo_pkg.core:fib"

[tool.pycc]
{tool}
"""


class Project(unittest.TestCase):
    """A scratch project directory per test."""

    def setUp(self):
        self._tmp = tempfile.TemporaryDirectory(prefix="pycc-backend-test-")
        self.tmp = pathlib.Path(self._tmp.name)
        self.root = self.tmp / "proj"
        self.out = self.tmp / "out"
        self.out.mkdir()

    def tearDown(self):
        self._tmp.cleanup()

    def project(self, tool: str = "", files: dict[str, str] | None = None) -> None:
        write(self.root, {"pyproject.toml": PYPROJECT.format(tool=tool), "README.md": "# Demo\n"})
        write(self.root, files if files is not None else {
            "src/demo_pkg/__init__.py": "from .core import fib\n",
            "src/demo_pkg/core.py": "def fib(n: int) -> int:\n    return n\n",
            "src/demo_pkg/cli.py": "def main() -> None:\n    pass\n",
            "src/demo_pkg/data/table.txt": "data\n",
            "src/demo_pkg/py.typed": "",
            "tests/test_core.py": "",
        })

    def fake_pycc(self) -> pathlib.Path:
        so = self.tmp / "fake.so"
        so.write_bytes(minimal_elf(["libc.so.6", "libm.so.6"]) if sys.platform != "darwin"
                       else minimal_macho((26, 0)))
        self.log = self.tmp / "pycc.log"
        exe = self.tmp / "bin" / "pycc"
        exe.parent.mkdir()
        exe.write_text(FAKE_PYCC.format(python=sys.executable, log=str(self.log), so=str(so)))
        exe.chmod(0o755)
        return exe

    def calls(self) -> list[dict]:
        return [json.loads(line) for line in self.log.read_text().splitlines()]

    def build_wheel(self, **env) -> zipfile.ZipFile:
        with chdir(self.root), environ(**env), contextlib.redirect_stderr(io.StringIO()) as err:
            name = build.build_wheel(str(self.out))
        self.stderr = err.getvalue()
        self.assertEqual(sorted(p.name for p in self.out.iterdir()), [name],
                         "the wheel directory holds only the wheel")
        self.wheel_path = self.out / name
        return zipfile.ZipFile(self.wheel_path)


class WheelFormat(unittest.TestCase):
    def test_record_hash_is_urlsafe_base64_sha256_without_padding(self):
        self.assertEqual(_wheel.record_hash(b""),
                         "sha256=47DEQpj8HBSa-_TImW-5JCeuQeRkm5NMpJWZG3hSuFU")
        data = b"pycc" * 100
        want = base64.urlsafe_b64encode(hashlib.sha256(data).digest()).decode().rstrip("=")
        self.assertEqual(_wheel.record_hash(data), "sha256=" + want)

    def test_writer_puts_dist_info_last_and_records_every_file(self):
        w = _wheel.WheelWriter("x-1.dist-info")
        w.add("x-1.dist-info/METADATA", b"m")
        w.add("x/__init__.abi3.so", b"\x7fELF", executable=True)
        w.add("x/data.txt", b"d")
        with self.assertRaises(ValueError):
            w.add("x/data.txt", b"again")
        with self.assertRaises(ValueError):
            w.add("../escape", b"")
        with tempfile.TemporaryDirectory() as d, environ(SOURCE_DATE_EPOCH="1700000000"):
            path = pathlib.Path(d) / "x-1-py3-none-any.whl"
            w.write(path)
            with zipfile.ZipFile(path) as z:
                names = z.namelist()
                self.assertEqual(names, ["x/__init__.abi3.so", "x/data.txt",
                                         "x-1.dist-info/METADATA", "x-1.dist-info/RECORD"])
                info = z.getinfo("x/__init__.abi3.so")
                self.assertEqual(stat.S_IMODE(info.external_attr >> 16), 0o755)
                self.assertEqual(info.date_time[0], 2023)
                rows = list(csv.reader(io.StringIO(z.read("x-1.dist-info/RECORD").decode())))
            self.assertEqual(rows[-1], ["x-1.dist-info/RECORD", "", ""])
            self.assertEqual(rows[0], ["x/__init__.abi3.so", _wheel.record_hash(b"\x7fELF"), "4"])

    def test_sdist_is_normalized(self):
        with tempfile.TemporaryDirectory() as d:
            path = pathlib.Path(d) / "x-1.tar.gz"
            _wheel.write_sdist(path, "x-1", [("b.py", b"b"), ("a/c.txt", b"c")])
            with tarfile.open(path) as t:
                members = t.getmembers()
            self.assertEqual([m.name for m in members], ["x-1/a/c.txt", "x-1/b.py"])
            self.assertTrue(all(m.uid == 0 and m.mode == 0o644 and m.isreg() for m in members))


class Tags(unittest.TestCase):
    def test_elf_needed_reads_the_dynamic_section(self):
        with tempfile.TemporaryDirectory() as d:
            so = pathlib.Path(d) / "m.so"
            so.write_bytes(minimal_elf(["libc.so.6", "libfoo.so.1"]))
            self.assertEqual(_wheel.elf_needed(so), ["libc.so.6", "libfoo.so.1"])
            so.write_bytes(b"not an elf")
            self.assertIsNone(_wheel.elf_needed(so))

    @unittest.skipUnless(sys.platform.startswith("linux"), "needs an ELF interpreter")
    def test_elf_needed_of_this_python(self):
        needed = _wheel.elf_needed(pathlib.Path(sys.executable).resolve())
        self.assertTrue(any(n.startswith("libc.so") for n in needed), needed)

    def test_linux_tags(self):
        with tempfile.TemporaryDirectory() as d:
            ok = pathlib.Path(d) / "ok.so"
            ok.write_bytes(minimal_elf(["libc.so.6", "libm.so.6", "libgcc_s.so.1",
                                        "ld-linux-x86-64.so.2"]))
            foreign = pathlib.Path(d) / "foreign.so"
            foreign.write_bytes(minimal_elf(["libc.so.6", "libffi.so.8"]))
            def tag(files, **kw):
                return _wheel.platform_tag(files, system="Linux", **kw)

            self.assertEqual(tag([ok], machine="x86_64", libc=("glibc", "2.39")),
                             "manylinux_2_39_x86_64")
            self.assertEqual(tag([ok], machine="aarch64", libc=("glibc", "2.41")),
                             "manylinux_2_41_aarch64")
            self.assertEqual(tag([ok], machine="AMD64", libc=("glibc", "2.39")),
                             "manylinux_2_39_x86_64")
            with contextlib.redirect_stderr(io.StringIO()) as err:
                self.assertEqual(tag([ok, foreign], machine="x86_64", libc=("glibc", "2.39")),
                                 "linux_x86_64")
            self.assertIn("libffi.so.8", err.getvalue())
            self.assertEqual(tag([ok], machine="aarch64", libc=("", "")), "linux_aarch64")
            with self.assertRaises(_wheel.TagError):
                bad = pathlib.Path(d) / "bad.so"
                bad.write_bytes(b"text")
                tag([bad], machine="x86_64", libc=("glibc", "2.39"))

    def test_macos_tags(self):
        with tempfile.TemporaryDirectory() as d:
            so = pathlib.Path(d) / "m.so"
            def tag(**kw):
                return _wheel.platform_tag([so], system="Darwin", **kw)

            so.write_bytes(minimal_macho((26, 1)))
            self.assertEqual(tag(machine="arm64", env={}), "macosx_26_0_arm64")
            so.write_bytes(minimal_macho((10, 15), cmd=0x24))
            self.assertEqual(tag(machine="x86_64", env={}), "macosx_10_15_x86_64")
            so.write_bytes(b"no load commands")
            self.assertEqual(tag(machine="arm64", env={}, mac_version="15.4"),
                             "macosx_15_0_arm64")
            self.assertEqual(tag(machine="arm64", env={"MACOSX_DEPLOYMENT_TARGET": "26.0"},
                                 mac_version="15.4"), "macosx_26_0_arm64")
            with self.assertRaises(_wheel.TagError):
                tag(machine="ppc", env={})

    def test_other_systems_are_refused(self):
        with self.assertRaises(_wheel.TagError):
            _wheel.platform_tag([], system="Windows", machine="AMD64")


class Configuration(Project):
    def test_unknown_and_malformed_keys_are_errors(self):
        for table, message in [
            ({"stricct": False}, "unknown [tool.pycc] key(s): stricct"),
            ({"strict": "no"}, "strict must be a boolean"),
            ({"pure-python": "pkg"}, "pure-python must be a array"),
            ({"pure-python": ["pkg-x"]}, "not a dotted module name"),
            ({"packages": ["a.b"]}, "not a top-level package"),
            ({"extra-args": ["-o", "x"]}, "may not set '-o'"),
            ({"overrides": {"m": {"strikt": True}}}, "unknown [tool.pycc.overrides.\"m\"]"),
            ({"fallback-to-python": 3}, "fallback-to-python"),
        ]:
            with self.subTest(table=table), self.assertRaisesRegex(_project.ProjectError,
                                                                   re.escape(message)):
                _project.parse_tool(table)

    def test_options_take_the_most_specific_override(self):
        cfg = _project.parse_tool({
            "strict": True, "pure-python": ["pkg.cli"], "fallback-to-python": ["pkg.slow"],
            "extra-args": ["--a"],
            "overrides": {"pkg": {"strict": False, "extra-args": ["--b"]},
                          "pkg.core": {"strict": True},
                          "pkg.__init__": {"pure-python": True}},
        })
        self.assertEqual(cfg.options("pkg.core"),
                         _project.ModuleOptions(True, False, False, ("--a", "--b")))
        self.assertEqual(cfg.options("pkg.util"),
                         _project.ModuleOptions(False, False, False, ("--a", "--b")))
        self.assertTrue(cfg.options("pkg.__init__").pure)
        self.assertTrue(cfg.options("pkg.cli").pure)
        self.assertTrue(cfg.options("pkg.slow").fallback)
        self.assertFalse(cfg.options("pkgx").strict is False)
        self.assertTrue(_project.parse_tool({"fallback-to-python": True}).options("m").fallback)

    def test_project_metadata_errors(self):
        for change, message in [
            (('version = "1.2.0"', 'version = "one"'), "not a PEP 440 version"),
            (('name = "Demo.Pkg"', 'name = "-bad"'), "not a valid project name"),
            (('version = "1.2.0"', 'dynamic = ["version"]'), "reads only static metadata"),
        ]:
            with self.subTest(change=change):
                self.project()
                path = self.root / "pyproject.toml"
                path.write_text(path.read_text().replace(*change))
                with self.assertRaisesRegex(_project.ProjectError, re.escape(message)):
                    _project.load(self.root)
        (self.root / "pyproject.toml").unlink()
        with self.assertRaisesRegex(_project.ProjectError, "does not exist"):
            _project.load(self.root)

    def test_metadata(self):
        self.project()
        text = _project.metadata(_project.load(self.root))
        self.assertTrue(text.startswith("Metadata-Version: 2.1\nName: Demo.Pkg\nVersion: 1.2.0\n"))
        for line in ["Summary: A demo", "Author: Bob", 'Author-email: "Ada" <ada@example.org>',
                     "License: MIT", "Keywords: a,b",
                     "Classifier: Programming Language :: Python :: 3",
                     "Project-URL: Homepage, https://example.org", "Requires-Python: >=3.13",
                     "Requires-Dist: attrs>=23", "Provides-Extra: fast",
                     'Requires-Dist: numpy; extra == "fast"',
                     "Requires-Dist: uvloop; (sys_platform != 'win32') and extra == \"fast\"",
                     "Description-Content-Type: text/markdown"]:
            self.assertIn(line + "\n", text)
        self.assertTrue(text.endswith("\n\n# Demo\n"))
        eps = _project.entry_points(_project.load(self.root))
        self.assertEqual(eps, "[console_scripts]\ndemo = demo_pkg.cli:main\n\n"
                              "[demo.plugins]\none = demo_pkg.core:fib\n\n")


class Discovery(Project):
    def rels(self) -> dict[str, str | None]:
        return {f.rel: f.module for f in _project.sources(_project.load(self.root))}

    def test_src_layout(self):
        self.project()
        self.assertEqual(self.rels(), {
            "demo_pkg/__init__.py": "demo_pkg.__init__", "demo_pkg/cli.py": "demo_pkg.cli",
            "demo_pkg/core.py": "demo_pkg.core", "demo_pkg/data/table.txt": None,
            "demo_pkg/py.typed": None,
        })

    def test_flat_layout_skips_tests_docs_and_setup(self):
        self.project(files={"alpha/__init__.py": "", "alpha/sub/__init__.py": "",
                            "alpha/sub/x.py": "", "alpha/notpkg/y.py": "", "beta.py": "",
                            "setup.py": "", "tests/__init__.py": "", "docs/conf.py": "",
                            "test_beta.py": "", "_private.py": ""})
        self.assertEqual(self.rels(), {
            "alpha/__init__.py": "alpha.__init__", "alpha/notpkg/y.py": None,
            "alpha/sub/__init__.py": "alpha.sub.__init__", "alpha/sub/x.py": "alpha.sub.x",
            "beta.py": "beta",
        })

    def test_the_package_named_after_the_project_wins(self):
        self.project(files={"demo_pkg/__init__.py": "", "other/__init__.py": ""})
        self.assertEqual(self.rels(), {"demo_pkg/__init__.py": "demo_pkg.__init__"})

    def test_explicit_source_dir_and_modules(self):
        self.project('source-dir = "code"\npy-modules = ["png"]',
                     {"code/png.py": "", "code/test_png.py": ""})
        self.assertEqual(self.rels(), {"png.py": "png"})

    def test_nothing_to_ship_is_an_error(self):
        self.project(files={"tests/test_a.py": ""})
        with self.assertRaisesRegex(_project.ProjectError, "no package or module"):
            self.rels()

    def test_links_and_escapes_are_refused(self):
        self.project()
        os.symlink(self.root / "README.md", self.root / "src/demo_pkg/link.txt")
        with self.assertRaisesRegex(_project.ProjectError, "symbolic link"):
            self.rels()
        self.project('source-dir = "../elsewhere"')
        with self.assertRaisesRegex(_project.ProjectError, "inside the project"):
            self.rels()


class Wheel(Project):
    def test_compiles_modules_and_writes_a_valid_wheel(self):
        self.project('pure-python = ["demo_pkg.cli"]\n'
                     '[tool.pycc.overrides."demo_pkg.__init__"]\nstrict = false\n')
        pycc_exe = self.fake_pycc()
        with self.build_wheel(PYCC=str(pycc_exe), PYCC_PYTHON=None) as z:
            names = z.namelist()
            record = list(csv.reader(io.StringIO(z.read("demo_pkg-1.2.0.dist-info/RECORD")
                                                 .decode())))
            wheel = z.read("demo_pkg-1.2.0.dist-info/WHEEL").decode()
            entry_points = z.read("demo_pkg-1.2.0.dist-info/entry_points.txt").decode()
            for name, digest, size in record[:-1]:
                data = z.read(name)
                self.assertEqual((digest, int(size)), (_wheel.record_hash(data), len(data)))
            meta = z.read("demo_pkg-1.2.0.dist-info/METADATA").decode()
        tag = self.wheel_path.name[len("demo_pkg-1.2.0-"):-len(".whl")]
        self.assertTrue(tag.startswith("cp313-abi3-"), tag)
        self.assertIn(f"Root-Is-Purelib: false\nTag: {tag}\n", wheel)
        self.assertIn("demo = demo_pkg.cli:main", entry_points)
        self.assertEqual(names, [
            "demo_pkg/__init__.abi3.so", "demo_pkg/cli.py", "demo_pkg/core.abi3.so",
            "demo_pkg/data/table.txt", "demo_pkg/py.typed",
            "demo_pkg-1.2.0.dist-info/METADATA", "demo_pkg-1.2.0.dist-info/WHEEL",
            "demo_pkg-1.2.0.dist-info/entry_points.txt", "demo_pkg-1.2.0.dist-info/RECORD",
        ])
        self.assertEqual({r[0] for r in record}, set(names))
        calls = {pathlib.Path(c["args"][1]).name: c for c in self.calls()}
        self.assertEqual(set(calls), {"__init__.py", "core.py"})
        init, core = calls["__init__.py"]["args"], calls["core.py"]["args"]
        for args in (init, core):
            self.assertEqual(args[0], "build")
            for flag in ("--ext", "--foreign-relative-imports", "--release"):
                self.assertIn(flag, args)
        self.assertIn("--non-strict", init)
        self.assertNotIn("--non-strict", core)
        # Staged inside the wheel directory, TMPDIR too; nothing left behind.
        for c in calls.values():
            self.assertTrue(c["tmpdir"].startswith(str(self.out)), c["tmpdir"])
            self.assertEqual(c["python"], sys.executable)
        self.assertIn("demo_pkg.core: compiled", self.stderr)
        # pycc's messages name the project's files, not the staged copies.
        src = self.root.resolve() / "src"
        self.assertIn(f"{src}{os.sep}demo_pkg{os.sep}core.py:1", self.stderr)
        self.assertNotIn(".pycc-build-", self.stderr)
        # prepare_metadata_for_build_wheel writes the same METADATA.
        meta_dir = self.tmp / "meta"
        with chdir(self.root):
            dist_info = build.prepare_metadata_for_build_wheel(str(meta_dir))
        self.assertEqual((meta_dir / dist_info / "METADATA").read_text(), meta)

    def test_a_compile_error_fails_the_build_with_pycc_diagnostics(self):
        self.project(files={"m.py": "FAIL\n", "ok.py": ""})
        exe = self.fake_pycc()
        with chdir(self.root), environ(PYCC=str(exe)), \
                contextlib.redirect_stderr(io.StringIO()):
            with self.assertRaises(build.BuildError) as e:
                build.build_wheel(str(self.out))
        message = str(e.exception)
        self.assertIn("pycc could not compile m (m.py)", message)
        self.assertIn("error[C0001]: not supported at", message)
        self.assertIn('pure-python = ["m"]', message)
        self.assertEqual(list(self.out.iterdir()), [], "no wheel and no staging left behind")

    def test_fallback_to_python_ships_the_source(self):
        self.project('fallback-to-python = ["m"]', {"m.py": "FAIL\n", "ok.py": ""})
        exe = self.fake_pycc()
        with self.build_wheel(PYCC=str(exe)) as z:
            self.assertEqual([n for n in z.namelist() if "dist-info" not in n],
                             ["m.py", "ok.abi3.so"])
        self.assertIn("m: pycc could not compile it; it ships as Python", self.stderr)

    def test_all_python_is_a_pure_wheel_without_pycc(self):
        self.project('pure-python = ["demo_pkg"]')
        with self.build_wheel(PYCC=None, PATH=str(self.tmp / "empty")) as z:
            self.assertIn("Root-Is-Purelib: true\nTag: py3-none-any\n",
                          z.read("demo_pkg-1.2.0.dist-info/WHEEL").decode())
            self.assertIn("demo_pkg/core.py", z.namelist())
        self.assertTrue(self.wheel_path.name.endswith("-py3-none-any.whl"))

    def test_a_missing_compiler_is_named(self):
        self.project()
        with chdir(self.root), environ(PYCC=None, PATH=str(self.tmp / "empty")):
            with self.assertRaisesRegex(build.BuildError, "cannot find the pycc compiler"):
                build.build_wheel(str(self.out))
        with chdir(self.root), environ(PYCC=str(self.tmp / "nope")):
            with self.assertRaisesRegex(build.BuildError, "is not the pycc executable"):
                build.build_wheel(str(self.out))

    def test_find_pycc_accepts_a_release_directory(self):
        exe = self.fake_pycc()
        self.assertEqual(_compile.find_pycc({"PYCC": str(exe.parent.parent)}), exe.resolve())
        self.assertEqual(_compile.find_pycc({"PATH": str(exe.parent)}), exe.resolve())

    def test_sdist_and_editable(self):
        self.project('sdist-include = ["tests"]')
        (self.root / "LICENSE").write_text("MIT\n")
        path = self.root / "pyproject.toml"
        path.write_text(path.read_text().replace('license = { text = "MIT" }',
                                                 'license = { file = "LICENSE" }'))
        with chdir(self.root):
            name = build.build_sdist(str(self.out))
            editable = build.build_editable(str(self.out))
            self.assertEqual(build.get_requires_for_build_wheel(), [])
            self.assertEqual(build.get_requires_for_build_sdist(), [])
            self.assertEqual(build.get_requires_for_build_editable(), [])
        self.assertEqual(name, "demo_pkg-1.2.0.tar.gz")
        with tarfile.open(self.out / name) as t:
            self.assertEqual(t.getnames(), [f"demo_pkg-1.2.0/{n}" for n in [
                "LICENSE", "PKG-INFO", "README.md", "pyproject.toml", "src/demo_pkg/__init__.py",
                "src/demo_pkg/cli.py", "src/demo_pkg/core.py", "src/demo_pkg/data/table.txt",
                "src/demo_pkg/py.typed", "tests/test_core.py"]])
        with zipfile.ZipFile(self.out / editable) as z:
            pth = z.read("_pycc_editable_demo_pkg.pth").decode()
            self.assertIn("demo_pkg-1.2.0.dist-info/licenses/LICENSE", z.namelist())
        self.assertEqual(pth, str((self.root / "src").resolve()) + "\n")


class Versions(unittest.TestCase):
    def test_backend_version_matches_the_compiler_release(self):
        cargo = (PYTHON_DIR.parent / "Cargo.toml").read_text()
        version = re.search(r'^version = "([^"]+)"', cargo, re.M)[1]
        with open(PYTHON_DIR / "pyproject.toml", "rb") as f:
            self.assertEqual(tomllib.load(f)["project"]["version"], version)
        self.assertEqual(pycc.__version__, version)


if __name__ == "__main__":
    unittest.main()
