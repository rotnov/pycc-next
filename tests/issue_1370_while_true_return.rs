//! #1370: a function that ends in `while True:` (or `while 1:`) and leaves
//! the loop only through `return` or a raise does not fall off its end, so
//! it is not reported as `T0022` "can exit without returning". Codegen
//! terminates the unreachable block after such a loop with `unreachable`.
//!
//! The native cases need no interpreter and run in the coverage job; each
//! expected output is CPython 3.14.7's own for the same source, recorded
//! here. The `--ext` case compares live against the host interpreter and is
//! `#[ignore]`d like every hosted test.

use pycc_scratch::ScratchDir;
use std::path::Path;
use std::process::{Command, Output};

fn pycc() -> Command {
    Command::new(env!("CARGO_BIN_EXE_pycc"))
}

fn stdout_of(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).replace("\r\n", "\n")
}

fn stderr_of(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).replace("\r\n", "\n")
}

/// The functions every case calls: the issue's `f`, `while 1:`, a raise as
/// the other exit, a loop inside one `if` branch, a loop inside a `try`
/// body, a method, and a `str` result.
const SOURCE: &str = "\
def f(x: int) -> int:
    while True:
        if x > 0:
            return x
        x += 1


def g(x: int) -> int:
    while 1:
        if x > 3:
            return x
        x += 1


def h(x: int) -> int:
    while True:
        x -= 1
        if x < 0:
            raise ValueError(\"negative\")
        if x == 3:
            return x


def k(c: bool) -> int:
    if c:
        while True:
            return 1
    else:
        return 2


def t(x: int) -> int:
    try:
        while True:
            if x < 0:
                raise ValueError(\"negative\")
            if x > 5:
                return x
            x += 2
    except ValueError:
        y = -1
    return y


class A:
    def m(self, n: int) -> int:
        while True:
            n -= 1
            if n == 3:
                return n


def s(x: int) -> str:
    while True:
        x += 1
        if x > 2:
            return \"done\"


";

const SCRIPT: &str = "\
print(f(-3))
print(g(0))
print(h(10))
try:
    h(2)
except ValueError as e:
    print(\"caught\", e)
print(k(True), k(False))
print(t(0), t(-1))
print(A().m(10))
print(s(0))
";

const EXPECTED: &str = "1\n4\n3\ncaught negative\n1 2\n6 -1\n3\ndone\n";

fn check(dir: &Path, source: &str) -> Output {
    let src = dir.join("m.py");
    std::fs::write(&src, source).expect("write the fixture source");
    pycc()
        .arg("check")
        .arg(&src)
        .output()
        .expect("pycc should spawn")
}

/// The loops build and run natively and print CPython's output.
#[test]
fn while_true_functions_run_like_cpython() {
    let dir = ScratchDir::new("1370_native").expect("scratch");
    let src = dir.join("m.py");
    std::fs::write(&src, format!("{SOURCE}{SCRIPT}")).expect("write the fixture source");
    let binary = dir.join("app");
    let build = pycc()
        .arg("build")
        .arg(&src)
        .arg("-o")
        .arg(&binary)
        .output()
        .expect("pycc should spawn");
    assert!(build.status.success(), "{}", stderr_of(&build));
    let run = Command::new(&binary).output().expect("the binary runs");
    assert!(run.status.success(), "{}", stderr_of(&run));
    assert_eq!(stdout_of(&run), EXPECTED);
}

/// A loop test that may be false keeps `T0022`.
#[test]
fn a_loop_that_may_not_run_still_reports_t0022() {
    let dir = ScratchDir::new("1370_t0022").expect("scratch");
    let out = check(
        &dir,
        "def f(x: int) -> int:\n    while x:\n        return 1\n",
    );
    assert!(!out.status.success());
    assert!(
        stdout_of(&out).contains("error[T0022]: function `f` can exit without returning `int`"),
        "{}",
        stdout_of(&out)
    );
}

/// The constant-true rule is sound only while `break` cannot be lowered:
/// today a `break` inside a loop is a `C0001` capability gap, so no loop
/// body can leave the loop normally. When `break` lowering lands, this test
/// fails; the loop arms of `pycc_hir::return_coverage` and
/// `pycc_codegen::fallthrough` must then require that the body holds no
/// `break` bound to that loop, and this test becomes the `T0022` case.
#[test]
fn a_while_true_with_break_is_still_rejected() {
    let dir = ScratchDir::new("1370_break").expect("scratch");
    let out = check(
        &dir,
        "def f(x: int) -> int:\n    while True:\n        if x > 0:\n            break\n        x += 1\n",
    );
    assert!(!out.status.success());
    assert!(
        stdout_of(&out)
            .contains("error[C0001]: statement kind not supported yet: `break` inside a loop"),
        "{}",
        stdout_of(&out)
    );
}

/// The same functions built as an extension module and called from the
/// host, compared with CPython importing the same source.
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn while_true_functions_run_like_cpython_as_an_extension() {
    let oracle = std::env::var_os("PYCC_PYTHON").unwrap_or_else(|| "python3".into());
    let script = format!("from pycc_while1370 import *\n{SCRIPT}");

    let hosted = ScratchDir::new("1370_ext").expect("scratch");
    let src = hosted.join("m.py");
    std::fs::write(&src, SOURCE).expect("write the fixture source");
    let build = pycc()
        .arg("build")
        .arg(&src)
        .arg("-o")
        .arg(hosted.join("pycc_while1370"))
        .arg("--ext")
        .output()
        .expect("pycc should spawn");
    assert!(
        build.status.success(),
        "{}{}",
        stdout_of(&build),
        stderr_of(&build)
    );
    let run = Command::new(&oracle)
        .arg("-c")
        .arg(&script)
        .current_dir(&*hosted)
        .output()
        .expect("python3 should spawn");
    assert!(run.status.success(), "{}", stderr_of(&run));
    assert_eq!(stdout_of(&run), EXPECTED);

    let reference = ScratchDir::new("1370_ext_cpython").expect("scratch");
    std::fs::write(reference.join("pycc_while1370.py"), SOURCE).expect("write the oracle source");
    let cpython = Command::new(&oracle)
        .arg("-c")
        .arg(&script)
        .current_dir(&*reference)
        .output()
        .expect("python3 should spawn");
    assert!(cpython.status.success(), "{}", stderr_of(&cpython));
    assert_eq!(stdout_of(&cpython), EXPECTED);
}
