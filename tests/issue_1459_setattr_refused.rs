//! #1459: a class body that binds `__setattr__` or `__delattr__` is refused
//! at compile time with a located `C0001`, through the public CLI.
//!
//! Python calls `type(obj).__setattr__` on every `obj.x = v` and
//! `type(obj).__delattr__` on every `del obj.x`; this compiler stores into a
//! compiled instance without consulting either name, and an `--ext` slot or
//! property descriptor would bypass the method too. Before this change the
//! issue's module compiled and printed `done` alone where CPython prints
//! `intercept n` twice first. The refusal is the frontend's
//! (`crates/pycc_hir/src/class/reserved_names.rs`), so `pycc check`, a
//! native `pycc build` and a `pycc build --ext` all answer it before any
//! code is generated or any interpreter is probed; none of these tests needs
//! a CPython. The changed lines are covered by
//! `crates/pycc_hir/src/tests/store_protocol_names.rs`.

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
                            self.n = n\n\n    def __setattr__(self, name: str, value: int) -> \
                            None:\n        print(\"intercept\", name)\n\n\nc = C(1)\nc.n = \
                            2\nprint(\"done\")\n";

const METHOD_MESSAGE: &str = "error[C0001]: a `def __setattr__` in a class body is not \
                              supported yet -- Python calls `__setattr__` implicitly on every \
                              attribute store on an instance";

/// Asserts `output` is exactly one `C0001` refusal containing `message`,
/// located at `location`.
fn assert_refused(output: &Output, message: &str, location: &str) {
    let text = rendered(output);
    assert_eq!(output.status.code(), Some(1), "{text}");
    assert_eq!(text.matches("error[").count(), 1, "{text}");
    assert!(text.contains(message), "{text}");
    assert!(text.contains(location), "{text}");
    assert!(text.contains("(#1459)"), "{text}");
}

#[test]
fn the_issue_module_is_refused_by_check() {
    let dir = ScratchDir::new("setattr_refused_check").expect("scratch");
    let output = pycc()
        .arg("check")
        .arg(write(&dir, ISSUE_MODULE))
        .output()
        .expect("pycc should spawn");
    assert_refused(&output, METHOD_MESSAGE, "m.py:5:5");
}

#[test]
fn the_issue_module_is_refused_by_a_native_and_an_ext_build() {
    for ext in [false, true] {
        let dir = ScratchDir::new("setattr_refused_build").expect("scratch");
        let mut command = pycc();
        command
            .arg("build")
            .arg(write(&dir, ISSUE_MODULE))
            .arg("-o")
            .arg(dir.join("pycc_setattr_refused"));
        if ext {
            command.arg("--ext");
        }
        let output = command.output().expect("pycc should spawn");
        assert_refused(&output, METHOD_MESSAGE, "m.py:5:5");
        assert!(
            !dir.join("pycc_setattr_refused").exists(),
            "a refused build leaves no artifact (ext: {ext})"
        );
    }
}

/// A `def __delattr__`, and a class-attribute binding of either name -- a
/// literal, and a foreign callable CPython would call on the store -- are
/// refused at their own line.
#[test]
fn every_binding_route_is_refused_at_its_line() {
    let cases = [
        (
            "class C:\n    def __init__(self) -> None:\n        self.n = 1\n\n    \
             def __delattr__(self, name: str) -> None:\n        pass\n",
            "error[C0001]: a `def __delattr__` in a class body is not supported yet",
            "m.py:5:5",
        ),
        (
            "class C:\n    __delattr__ = 1\n\n    def __init__(self) -> None:\n        \
             self.n = 1\n",
            "error[C0001]: a class attribute named `__delattr__` is not supported yet",
            "m.py:2:5",
        ),
        (
            "import os\n\n\nclass C:\n    __setattr__ = staticmethod(os.getcwd)\n\n    \
             def __init__(self) -> None:\n        self.n = 1\n",
            "error[C0001]: a class attribute named `__setattr__` is not supported yet",
            "m.py:5:5",
        ),
    ];
    for (source, message, location) in cases {
        let dir = ScratchDir::new("setattr_refused_route").expect("scratch");
        let output = pycc()
            .arg("check")
            .arg(write(&dir, source))
            .output()
            .expect("pycc should spawn");
        assert_refused(&output, message, location);
    }
}
