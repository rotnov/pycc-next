//! v0.11: a fallback chunk of only `def` statements whose defaults, keyword
//! defaults and annotations are constants (no decorator, no closure) is
//! tabled as the functions' own code objects, and importing the `--ext`
//! module makes each function directly (`src/ext_build/precompiled.rs`)
//! instead of running a wrapper code object: no `exec`. Everything Python
//! sees is the source's: name, qualified name, module, docstring,
//! defaults, keyword defaults, annotations (deferred or not), first line,
//! the namespace's order, tracebacks, and the forwarder a compiled body
//! gets. A chunk with a decorator, a default computed at run time, or
//! anything else still runs as `exec` of its code.
//!
//! The oracle is `python3` (or `PYCC_PYTHON`) importing the source itself.

use pycc_scratch::ScratchDir;
use std::path::Path;
use std::process::{Command, Output};

fn pycc() -> Command {
    Command::new(env!("CARGO_BIN_EXE_pycc"))
}

fn python() -> Command {
    Command::new(std::env::var_os("PYCC_PYTHON").unwrap_or_else(|| "python3".into()))
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(&pycc_scratch::lf(bytes)).replace("\r\n", "\n")
}

/// Functions CPython defines: generators, lambdas inside, constant and
/// computed defaults, a decorator, a docstring, and one that raises.
const DEFERRED: &str = r#"from __future__ import annotations
import functools

LIMIT = 3


def gen(n: int, *, step: int = 1, label: str = "x") -> list[int]:
    """Yield every step-th number below n."""
    for i in range(0, n, step):
        yield i


def lam(k=(1, 2), *args, z=None, **kw):
    return (lambda: (k, args, z, kw))()
def bad():
    return (lambda: 1 / 0)()


def computed(k=LIMIT, *, j=[LIMIT]):
    return (lambda: (k, j))()


@functools.lru_cache
def decorated(x: int) -> int:
    return (lambda: x * 2)()


def plain(x: int) -> int:
    return x + 1


def parse_version(version: str) -> tuple[int, int, int]:
    """Its body is compiled; its `def` runs in CPython."""
    return tuple(map(int, version.partition('+')[0].split('.')))


def outer(n: int) -> int:
    def inner(k):
        yield k
        yield n + LIMIT
    print(inner.__qualname__, inner.__name__, inner.__code__.co_freevars)
    return sum(inner(n))
"#;

/// The same without deferred annotations: a constant annotation is part of
/// the `def`'s constants, a name is not.
const EAGER: &str = r#"def none_only(x: "int" = 1) -> None:
    return (lambda: print(x))()


def named(x: int) -> int:
    return (lambda: x)()
"#;

const DRIVER: &str = r#"
import sys, traceback, types
seen = []
sys.addaudithook(lambda e, a: seen.append(a[0].co_name) if e == "exec" and isinstance(a[0], types.CodeType) and a[0].co_filename.endswith("m.py") else None)
import m
print("exec at import:", len(seen), file=sys.stderr)
for name in ("gen", "lam", "bad", "computed", "decorated", "parse_version", "none_only", "named"):
    if hasattr(m, name):
        f = getattr(m, name)
        print(name, f.__name__, f.__qualname__, f.__module__, f.__doc__, getattr(f, "__defaults__", None),
              getattr(f, "__kwdefaults__", None), f.__annotations__, getattr(f, "__wrapped__", f).__code__.co_firstlineno,
              type(f).__name__, sorted(vars(f)))
# The module's own names in order (an `--ext` module binds no `__cached__`,
# and `from __future__ import annotations` binds nothing in it).
print([k for k in vars(m) if not k.startswith("__") and k != "annotations"])
for name in ("gen", "lam", "computed", "decorated", "plain", "parse_version", "outer", "none_only", "named"):
    if hasattr(m, name):
        f = getattr(m, name)
        print(name, list(f(5, step=2)) if name == "gen" else f(1) if name in ("decorated", "plain", "named", "none_only", "outer") else f("1.2.3+x") if name == "parse_version" else f())
if hasattr(m, "lam"):
    print(m.lam(7, 8, z=9, w=10), m.lam.__defaults__ is m.lam.__defaults__)
    m.lam.__kwdefaults__["z"] = "changed"
    print(m.lam(), m.computed.__kwdefaults__["j"] is m.computed.__kwdefaults__["j"])
if hasattr(m, "bad"):
    try:
        m.bad()
    except ZeroDivisionError:
        for frame in traceback.extract_tb(sys.exc_info()[2]):
            print(frame.name, frame.lineno, frame.filename.endswith("m.py"))
"#;

fn import(dir: &Path, optimize: bool) -> Output {
    python()
        .args(if optimize { &["-O", "-B", "-c", DRIVER][..] } else { &["-B", "-c", DRIVER][..] })
        .current_dir(dir)
        .output()
        .expect("python should spawn")
}

/// Builds `source` as an `--ext` module, imports it and the source, and
/// checks the same stdout, with and without `-O`; returns the build report
/// and the `exec` count without.
fn check(label: &str, source: &str) -> (String, String) {
    let compiled = ScratchDir::new(label).expect("scratch");
    let path = compiled.join("m.py");
    std::fs::write(&path, source).expect("write the module");
    let build = pycc()
        .args(["build", "--ext", "--non-strict", "--report-fallbacks"])
        .arg(&path)
        .arg("-o")
        .arg(compiled.join("m"))
        .output()
        .expect("pycc should spawn");
    let report = text(&build.stderr);
    assert!(build.status.success(), "{report}");
    std::fs::remove_file(&path).expect("import the artifact, not the source");
    let plain = ScratchDir::new(&format!("{label}_src")).expect("scratch");
    std::fs::write(plain.join("m.py"), source).expect("write the module");
    // Under `-O` every chunk is compiled from its source at import and
    // `exec`'d, as before.
    let mut count = String::new();
    for optimize in [true, false] {
        let (ours, oracle) = (import(&compiled, optimize), import(&plain, optimize));
        assert_eq!(text(&ours.stdout), text(&oracle.stdout), "{}", text(&ours.stderr));
        assert_eq!(ours.status.code(), Some(0), "{}", text(&ours.stderr));
        count = text(&ours.stderr).lines().next().unwrap_or_default().to_string();
    }
    (report, count)
}

/// Whether the oracle interpreter is CPython 3.13: the build reads 3.13's
/// bytecode for constant `def`s, and on another version every chunk
/// `exec`s.
fn python_is_3_13() -> bool {
    python()
        .args(["-c", "import sys; print(sys.version_info[:2] == (3, 13))"])
        .output()
        .is_ok_and(|out| text(&out.stdout).trim() == "True")
}

#[test]
fn fallback_defs_are_made_without_running_their_statements() {
    let (report, count) = check("direct_defs", DEFERRED);
    for name in ["gen", "lam", "bad", "computed", "decorated", "parse_version"] {
        assert!(report.contains(&format!("function `{name}`")), "{name} runs in CPython: {report}");
    }
    assert!(report.contains("`outer.<locals>.inner`"), "the nested `def` runs in CPython: {report}");
    // `gen`, `lam` and `bad` are one chunk of three `def`s, made directly,
    // as are `outer`'s scaffold and `parse_version`'s `def`; `computed`
    // (its defaults are read at run time) and `decorated`, one chunk too,
    // run their statements, as does the nested statements' helper chunk
    // (an `import`).
    if python_is_3_13() {
        assert_eq!(count, "exec at import: 2", "{report}");
    }
}

#[test]
fn an_eager_constant_annotation_is_made_directly_and_a_name_is_not() {
    let (report, count) = check("direct_defs_eager", EAGER);
    assert!(report.contains("function `named`"), "{report}");
    if python_is_3_13() {
        assert_eq!(count, "exec at import: 1", "{report}");
    }
}
