//! v0.11 accept 3: a compiled import statement whose names fallback code
//! reads is mirrored into the module's namespace (`Plan::mirrored`) by
//! compiled code storing what its own import bound, one store per name the
//! statement binds (`Plan::mirror_text` in `src/fallback/render.rs`),
//! instead of exec'ing a chunk that imports the module a second time. So
//! each statement calls `__import__` once with CPython's arguments, and each
//! occurrence of a name binds the value its own `IMPORT_FROM` took: nothing
//! is read twice, duplicate names (`from p import x as a, x as b`, `import
//! p as a, p as b`) bind in CPython's order, and another module's import
//! run in the middle of the statement changes nothing. That holds for
//! `import a`, `import a.b`, `import a.b as c`, and absolute and relative
//! `from` imports, with and without `-O`. Compiled code's own call passes
//! the module's namespace as `globals` and `locals`, as `IMPORT_NAME` does,
//! so an `__import__` that answers by the importer's `__name__` binds the
//! same object. An import of a module compiled code models natively
//! (`math`) still runs again in CPython, and stays compiled.
//!
//! The oracle is CPython running the same sources.

use pycc_scratch::ScratchDir;
use std::path::Path;
use std::process::{Command, Output};

fn python() -> Command {
    Command::new(std::env::var_os("PYCC_PYTHON").unwrap_or_else(|| "python3".into()))
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(&pycc_scratch::lf(bytes)).into_owned()
}

const INIT: &str = "\"\"\"A package whose member mirrors its imports.\"\"\"\n";

const M: &str = r#"import json
import colorsys, pmi.nested
import os.path
import xml.dom.minidom as minidom
from collections import OrderedDict, ChainMap as cm
from os import path as p2, sep
from . import sib
from .sib import X as Y, twice
from probe import x, y as yy
import probe2.sub as ps
from probe import x as xa, x as xb
import probe3 as pa, probe3 as pb
import math


def names():
    # Runs in CPython (a called `lambda`), so every import above is
    # mirrored into the namespace.
    return (lambda: (json.dumps([1]), os.path.basename("/a/b"), minidom.__name__,
                     OrderedDict.__name__, cm.__name__, p2.__name__, sep, sib.X, Y,
                     twice(4), colorsys.__name__, x, yy, ps, pmi.nested.f(), xa, xb,
                     pa, pb, math.floor(2.5)))()


def native(n: int) -> int:
    return n + len(json.dumps([n])) + twice(n) + len(colorsys.__name__)


def native_rgb() -> object:
    return colorsys.rgb_to_hsv


def native_probe() -> object:
    return (x, yy, ps, xa, xb, pa, pb)


def native_floor(v: float) -> int:
    return math.floor(v)
"#;

const SIB: &str = "X = 7\n\n\ndef twice(n: int) -> int:\n    return n * 2\n";

/// Imported in the middle of `m`'s `import colorsys, pmi.nested`: its own
/// mirrored import runs then.
const NESTED: &str = "import json\n\n\ndef f():\n    return (lambda: json.dumps(2))()\n";

/// Prints the `__import__` calls `pmi.m`'s own statements make (to
/// stderr), then what the namespace holds.
const DRIVER: &str = r#"
import builtins, sys
sys.path.insert(0, sys.argv[1])
calls = []
real = builtins.__import__
import types
OWN = {"json", "colorsys", "pmi.nested", "os.path", "xml.dom.minidom", "collections", "os", "", "sib"}
# Answers by the importer's `__name__`, as CPython passes it.
FAKE = types.ModuleType("fake_colorsys")
FAKE.rgb_to_hsv = "hooked"
# Each read of an attribute answers the next number, so a second read of
# what the statement already read shows.
class Counting:
    def __init__(self, names):
        self.names = names
        self.reads = 0
    def __getattr__(self, attr):
        if attr not in self.names:
            raise AttributeError(attr)
        self.reads += 1
        return self.reads
PROBE = Counting(("x", "y"))
PROBE2 = Counting(("sub",))
# Each `__import__` of `probe3` answers the next of two objects.
ANSWERS = ["A", "B"]
def hook(name, globals=None, locals=None, fromlist=(), level=0):
    importer = (globals or {}).get("__name__")
    if importer == "pmi.m" and name in ("probe", "probe2.sub", "probe3"):
        calls.append((name, tuple(fromlist or ()), level, importer, locals is globals))
        if name == "probe3":
            return ANSWERS.pop(0)
        return PROBE if name == "probe" else PROBE2
    if name in OWN and importer in (None, "pmi.m"):
        calls.append((name, tuple(fromlist or ()), level, importer, locals is globals))
    if name == "colorsys" and importer == "pmi.m":
        return FAKE
    return real(name, globals, locals, fromlist, level)
builtins.__import__ = hook
from pmi import m
builtins.__import__ = real
print(calls, file=sys.stderr)
print(m.names(), m.native(3), m.native_rgb(), m.colorsys is FAKE)
print(m.native_probe(), PROBE.reads, PROBE2.reads, m.native_floor(3.5))
print(sorted(k for k in vars(m) if not k.startswith("__")))
print(m.minidom is sys.modules["xml.dom.minidom"], m.os is sys.modules["os"],
      m.sib is sys.modules["pmi.sib"], m.p2 is sys.modules["os.path"])
"#;

fn write_package(root: &Path) {
    let package = root.join("pmi");
    std::fs::create_dir_all(&package).expect("create the package");
    for (name, source) in [("__init__.py", INIT), ("m.py", M), ("sib.py", SIB), ("nested.py", NESTED)] {
        std::fs::write(package.join(name), source).expect("write a module");
    }
}

fn run(root: &Path, optimize: bool) -> Output {
    let mut command = python();
    command.arg("-I");
    if optimize {
        command.arg("-O");
    }
    command
        .args(["-c", DRIVER])
        .arg(root)
        .current_dir(root)
        .output()
        .expect("python3 should spawn")
}

#[test]
fn a_mirrored_import_binds_its_names_with_one_import_call_as_cpython() {
    let compiled = ScratchDir::new("mirrored_imports").expect("scratch");
    write_package(&compiled);
    let package = compiled.join("pmi");
    let build = Command::new(env!("CARGO_BIN_EXE_pycc"))
        .arg("build")
        .arg(&package)
        .arg("-o")
        .arg(package.join("__init__"))
        .args(["--ext", "--non-strict", "--report-fallbacks"])
        .output()
        .expect("pycc should spawn");
    let report = text(&build.stderr);
    assert!(build.status.success(), "{report}");
    assert!(report.contains("function `names`"), "{report}");
    // Every import but the two of `sib` (which fallback code reads as a
    // module compiled code does not expose) stays compiled: the mirror's
    // stores compile, `import math` included.
    assert_eq!(report.matches("statement at").count(), 2, "{report}");
    for name in ["__init__.py", "m.py", "sib.py", "nested.py"] {
        std::fs::remove_file(package.join(name)).expect("import the artifact, not the source");
    }
    let plain = ScratchDir::new("mirrored_imports_src").expect("scratch");
    write_package(&plain);
    for optimize in [false, true] {
        let ours = run(&compiled, optimize);
        let oracle = run(&plain, optimize);
        assert_eq!(ours.status.code(), Some(0), "{}", text(&ours.stderr));
        assert_eq!(text(&ours.stdout), text(&oracle.stdout), "{}", text(&ours.stderr));
        // The same `__import__` calls, one per statement.
        assert_eq!(text(&ours.stderr), text(&oracle.stderr));
    }
    let oracle = run(&plain, false);
    assert_eq!(
        text(&oracle.stderr),
        "[('json', (), 0, 'pmi.m', True), ('colorsys', (), 0, 'pmi.m', True), \
         ('pmi.nested', (), 0, 'pmi.m', True), \
         ('os.path', (), 0, 'pmi.m', True), ('xml.dom.minidom', (), 0, 'pmi.m', True), \
         ('collections', ('OrderedDict', 'ChainMap'), 0, 'pmi.m', True), \
         ('os', ('path', 'sep'), 0, 'pmi.m', True), ('', ('sib',), 1, 'pmi.m', True), \
         ('sib', ('X', 'twice'), 1, 'pmi.m', True), ('probe', ('x', 'y'), 0, 'pmi.m', True), \
         ('probe2.sub', (), 0, 'pmi.m', True), ('probe', ('x', 'x'), 0, 'pmi.m', True), \
         ('probe3', (), 0, 'pmi.m', True), ('probe3', (), 0, 'pmi.m', True)]\n",
        "the CPython reference itself"
    );
}
