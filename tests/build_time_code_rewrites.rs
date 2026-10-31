//! v0.11: the code objects of a nested `def` CPython runs (its scaffold,
//! `src/fallback/nested.rs`) and of a published signature's wrapper
//! (`src/ext_build/signatures.rs`) are re-qualified and flagged when the
//! build compiles their chunks (`CodeRewrite`, `src/ext_build/precompiled.rs`),
//! so an import from the pre-compiled table makes no `code.replace` call,
//! while under `-O`, where the embedded source is compiled at import, the
//! source still makes them. Either way the names, signatures and tracebacks
//! are the uncompiled module's.
//!
//! The oracle is `python3` (or `PYCC_PYTHON`) importing the source itself.

use pycc_scratch::ScratchDir;
use std::path::Path;
use std::process::{Command, Output};

fn python() -> Command {
    Command::new(std::env::var_os("PYCC_PYTHON").unwrap_or_else(|| "python3".into()))
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(&pycc_scratch::lf(bytes)).into_owned()
}

const MODULE: &str = r#"import functools


def deco(fn):
    @functools.wraps(fn)
    def inner(*args, **kwargs):
        return fn(*args, **kwargs)
    return inner


def outer(n: int) -> object:
    @deco
    def helper(k):
        def leaf():
            return k * n
        return leaf() + 1
    return helper


def keyed(a: int, b: int = 2, *, c: int = 3) -> int:
    if a < 0:
        raise ValueError("negative")
    return a + b * c


class Box:
    def scaled(self, x: int, factor: int = 10, *, extra: int = 0) -> int:
        return x * factor + extra
"#;

/// Counts the `code.replace` calls the import makes (first stderr line),
/// then exercises the module.
const DRIVER: &str = r#"import sys, traceback
replaced = []
def prof(frame, ev, arg):
    if ev == "c_call" and getattr(arg, "__name__", "") == "replace" and type(getattr(arg, "__self__", None)).__name__ == "code":
        replaced.append(frame.f_code.co_name)
sys.setprofile(prof)
import m
sys.setprofile(None)
print("replaced at import:", len(replaced), file=sys.stderr)
h = m.outer(4)
print(h(3), h.__qualname__, h.__wrapped__.__qualname__, h.__wrapped__.__code__.co_qualname)
print(m.keyed(1), m.keyed(1, 3, c=4), m.keyed.__qualname__, m.keyed.__code__.co_qualname, m.keyed.__name__)
b = m.Box()
print(b.scaled(2), b.scaled(2, factor=3, extra=1), m.Box.scaled.__qualname__)
try:
    m.keyed(-1)
except ValueError:
    print([f.name for f in traceback.extract_tb(sys.exc_info()[2])])
try:
    m.keyed(1, 2, 3)
except TypeError as e:
    print("TypeError", e)
"#;

fn run(dir: &Path, optimize: bool) -> Output {
    let mut command = python();
    if optimize {
        command.arg("-O");
    }
    command
        .args(["-B", "-c", DRIVER])
        .current_dir(dir)
        .output()
        .expect("python should spawn")
}

#[test]
fn rewritten_code_objects_match_cpython_with_and_without_the_table() {
    let compiled = ScratchDir::new("code_rewrites").expect("scratch");
    let source = compiled.join("m.py");
    std::fs::write(&source, MODULE).expect("write the module");
    let build = Command::new(env!("CARGO_BIN_EXE_pycc"))
        .args(["build", "--ext", "--non-strict", "--report-fallbacks"])
        .arg(&source)
        .arg("-o")
        .arg(compiled.join("m"))
        .output()
        .expect("pycc should spawn");
    assert!(build.status.success(), "{}", text(&build.stderr));
    // Both nested `def`s run in CPython, so each has a scaffold.
    assert_eq!(
        text(&build.stderr)
            .matches("its `def` (decorators, defaults, signature) runs in CPython")
            .count(),
        2,
        "{}",
        text(&build.stderr)
    );
    std::fs::remove_file(&source).expect("import the artifact, not the source");
    let plain = ScratchDir::new("code_rewrites_src").expect("scratch");
    std::fs::write(plain.join("m.py"), MODULE).expect("write the module");
    for optimize in [false, true] {
        let ours = run(&compiled, optimize);
        let oracle = run(&plain, optimize);
        assert_eq!(ours.status.code(), Some(0), "{}", text(&ours.stderr));
        assert_eq!(
            text(&ours.stdout),
            text(&oracle.stdout),
            "{}",
            text(&ours.stderr)
        );
        let replaced = text(&ours.stderr)
            .lines()
            .next()
            .unwrap_or_default()
            .to_string();
        if optimize {
            assert_ne!(replaced, "replaced at import: 0", "{}", text(&ours.stderr));
        } else {
            assert_eq!(replaced, "replaced at import: 0", "{}", text(&ours.stderr));
        }
    }
}
