//! #1465: a class body that binds `__getattribute__` or `__getattr__` is
//! refused at compile time with a located `C0001`, through the public CLI.
//!
//! Python calls `type(obj).__getattribute__` on every attribute read on an
//! instance and `type(obj).__getattr__` whenever that normal lookup raises
//! `AttributeError`; this compiler reads a compiled field or property
//! without consulting either name, and an `--ext` descriptor or a host read
//! of a missing name bypasses both too. Before this change the issue's
//! module compiled and printed `1` where CPython prints `42`. The refusal is
//! the frontend's (`crates/pycc_hir/src/class/reserved_names.rs`), so
//! `pycc check`, a native `pycc build` and a `pycc build --ext` all answer
//! it before any code is generated or any interpreter is probed; none of
//! these tests needs a CPython. The changed lines are covered by
//! `crates/pycc_hir/src/tests/read_protocol_names.rs`.

use pycc_scratch::ScratchDir;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn pycc() -> Command {
    Command::new(env!("CARGO_BIN_EXE_pycc"))
}

fn rendered(output: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
    .replace("\r\n", "\n")
}

fn write(dir: &Path, body: &str) -> PathBuf {
    let path = dir.join("m.py");
    std::fs::write(&path, body).expect("write the fixture source");
    path
}

/// The issue's own module.
const ISSUE_MODULE: &str = "class C:\n    def __init__(self, n: int) -> None:\n        \
                            self.n = n\n    def __getattribute__(self, name: str) -> int:\n        \
                            return 42\n\n\nc = C(1)\nprint(c.n)\n";

const METHOD_MESSAGE: &str = "error[C0001]: a `def __getattribute__` in a class body is not \
                              supported yet -- Python calls `__getattribute__` implicitly on \
                              every attribute read on an instance";

/// Asserts `output` is exactly one `C0001` refusal containing `message`,
/// located at `location`.
fn assert_refused(output: &Output, message: &str, location: &str) {
    let text = rendered(output);
    assert_eq!(output.status.code(), Some(1), "{text}");
    assert_eq!(text.matches("error[").count(), 1, "{text}");
    assert!(text.contains(message), "{text}");
    assert!(text.contains(location), "{text}");
    assert!(text.contains("(#1465)"), "{text}");
}

#[test]
fn the_issue_module_is_refused_by_check() {
    let dir = ScratchDir::new("getattribute_refused_check").expect("scratch");
    let output = pycc()
        .arg("check")
        .arg(write(&dir, ISSUE_MODULE))
        .output()
        .expect("pycc should spawn");
    assert_refused(&output, METHOD_MESSAGE, "m.py:4:5");
}

#[test]
fn the_issue_module_is_refused_by_a_native_and_an_ext_build() {
    for ext in [false, true] {
        let dir = ScratchDir::new("getattribute_refused_build").expect("scratch");
        let mut command = pycc();
        command
            .arg("build")
            .arg(write(&dir, ISSUE_MODULE))
            .arg("-o")
            .arg(dir.join("pycc_getattribute_refused"));
        if ext {
            command.arg("--ext");
        }
        let output = command.output().expect("pycc should spawn");
        assert_refused(&output, METHOD_MESSAGE, "m.py:4:5");
        assert!(
            !dir.join("pycc_getattribute_refused").exists(),
            "a refused build leaves no artifact (ext: {ext})"
        );
    }
}

/// A `def __getattr__` behind a property whose getter raises
/// `AttributeError` -- the program that printed a traceback here where
/// CPython prints `42` -- and a class-attribute binding of either name (a
/// literal, and a foreign callable CPython would call on the read) are
/// refused at their own line.
#[test]
fn every_binding_route_is_refused_at_its_line() {
    let cases = [
        (
            "class C:\n    @property\n    def p(self) -> int:\n        raise AttributeError(\"x\")\n\n    \
             def __getattr__(self, name: str) -> int:\n        return 42\n\n\nprint(C().p)\n",
            "error[C0001]: a `def __getattr__` in a class body is not supported yet -- Python \
             calls `__getattr__` implicitly whenever normal lookup",
            "m.py:6:5",
        ),
        (
            "class C:\n    __getattribute__ = 1\n\n    def __init__(self) -> None:\n        \
             self.n = 1\n",
            "error[C0001]: a class attribute named `__getattribute__` is not supported yet",
            "m.py:2:5",
        ),
        (
            "import os\n\n\nclass C:\n    __getattr__ = staticmethod(os.getcwd)\n\n    \
             def __init__(self) -> None:\n        self.n = 1\n",
            "error[C0001]: a class attribute named `__getattr__` is not supported yet",
            "m.py:5:5",
        ),
    ];
    for (source, message, location) in cases {
        let dir = ScratchDir::new("getattribute_refused_route").expect("scratch");
        let output = pycc()
            .arg("check")
            .arg(write(&dir, source))
            .output()
            .expect("pycc should spawn");
        assert_refused(&output, message, location);
    }
}
