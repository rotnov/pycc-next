//! Definite assignment around `break`, `continue` and a loop's `else:`.
//!
//! The statement after a loop is reached by the loop's normal exit, through
//! its `else:` clause, and by each `break` with the bindings in effect
//! there; a branch or handler that ends in `continue` (or `return`/`raise`)
//! does not reach the statement after its `if`/`try`. A name bound on every
//! such edge is definitely bound; a read of one missing on any edge is
//! checked at run time (`UnboundLocalError`, as in CPython), except an
//! `except ... as` name, whose read after its handler stays `T0041`.
//!
//! `tests/fixtures/loop_definite_assignment.py` is compared byte-for-byte
//! with the live CPython oracle (`PYCC_PYTHON`, default `python3`), built
//! natively and as an `--ext` module.

use pycc_scratch::ScratchDir;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const FIXTURE: &str = "tests/fixtures/loop_definite_assignment.py";

fn pycc() -> Command {
    Command::new(env!("CARGO_BIN_EXE_pycc"))
}

fn oracle() -> std::ffi::OsString {
    std::env::var_os("PYCC_PYTHON").unwrap_or_else(|| "python3".into())
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).replace("\r\n", "\n")
}

fn fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(FIXTURE)
}

fn assert_success(output: &Output, what: &str) {
    assert!(
        output.status.success(),
        "{what} failed:\n{}{}",
        text(&output.stdout),
        text(&output.stderr)
    );
}

fn cpython_stdout() -> String {
    let run = Command::new(oracle())
        .arg(fixture())
        .output()
        .expect("python3 should spawn");
    assert_success(&run, "CPython");
    text(&run.stdout)
}

#[test]
fn loop_exits_bind_like_cpython_native() {
    let dir = ScratchDir::new("loop_definite_native").expect("scratch");
    let binary = dir.join("app");
    let build = pycc()
        .arg("build")
        .arg(fixture())
        .arg("-o")
        .arg(&binary)
        .arg("--release")
        .output()
        .expect("pycc should spawn");
    assert_success(&build, "pycc build");
    let run = Command::new(&binary).output().expect("the binary runs");
    assert_success(&run, "the native binary");
    assert_eq!(text(&run.stdout), cpython_stdout());
}

#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn loop_exits_bind_like_cpython_as_an_extension() {
    let source = std::fs::read_to_string(fixture()).expect("read the fixture");
    let script =
        "import lda_mod\nprint(lda_mod.first_div(91), lda_mod.nested(3, 3), lda_mod.pick(True))";

    let hosted = ScratchDir::new("loop_definite_ext").expect("scratch");
    let src = hosted.join("m.py");
    std::fs::write(&src, &source).expect("write the source");
    let build = pycc()
        .arg("build")
        .arg(&src)
        .arg("-o")
        .arg(hosted.join("lda_mod"))
        .arg("--ext")
        .output()
        .expect("pycc should spawn");
    assert_success(&build, "pycc build --ext");
    let run = Command::new(oracle())
        .arg("-c")
        .arg(script)
        .current_dir(&*hosted)
        .output()
        .expect("python3 should spawn");
    assert_success(&run, "importing the extension");

    let reference = ScratchDir::new("loop_definite_cpython").expect("scratch");
    std::fs::write(reference.join("lda_mod.py"), &source).expect("write the oracle source");
    let cpython = Command::new(oracle())
        .arg("-c")
        .arg(script)
        .current_dir(&*reference)
        .output()
        .expect("python3 should spawn");
    assert_success(&cpython, "CPython importing the source");
    assert_eq!(text(&run.stdout), text(&cpython.stdout));
}

/// `source` compiles, and running it matches CPython: the same stdout, exit
/// status `1` and final `UnboundLocalError`/`NameError` line naming `name`
/// -- a read of a name bound on some paths only is checked at run time.
fn assert_unbound_like_cpython(tag: &str, source: &str, name: &str) {
    let dir = ScratchDir::new(tag).expect("scratch");
    let src = dir.join("m.py");
    std::fs::write(&src, source).expect("write the source");
    let binary = dir.join("app");
    let build = pycc()
        .arg("build")
        .arg(&src)
        .arg("-o")
        .arg(&binary)
        .arg("--release")
        .output()
        .expect("pycc should spawn");
    assert_success(&build, "pycc build");
    let run = Command::new(&binary).output().expect("the binary runs");
    let cpython = Command::new(oracle())
        .arg(&src)
        .output()
        .expect("python3 should spawn");
    let last = |bytes: &[u8]| text(bytes).trim_end().lines().last().unwrap_or("").to_string();
    assert_eq!(text(&run.stdout), text(&cpython.stdout), "{tag}");
    assert_eq!(run.status.code(), Some(1), "{tag}");
    assert_eq!(cpython.status.code(), Some(1), "{tag}");
    assert_eq!(last(&run.stderr), last(&cpython.stderr), "{tag}");
    assert!(last(&run.stderr).contains(&format!("'{name}'")), "{tag}");
}

/// `pycc check` on `source` fails with `T0041` naming `name`.
fn assert_maybe_unbound(tag: &str, source: &str, name: &str) {
    let dir = ScratchDir::new(tag).expect("scratch");
    let src = dir.join("m.py");
    std::fs::write(&src, source).expect("write the source");
    let out = pycc()
        .arg("check")
        .arg(&src)
        .output()
        .expect("pycc should spawn");
    // Diagnostics may go to either stream.
    let stderr = format!("{}{}", text(&out.stdout), text(&out.stderr));
    assert!(!out.status.success(), "{tag}: expected T0041, check passed");
    assert!(
        stderr.contains(&format!(
            "error[T0041]: local name `{name}` may not be bound on every path reaching this use"
        )),
        "{tag}: {stderr}"
    );
}

#[test]
fn a_loop_target_after_a_loop_without_else_is_maybe_unbound() {
    // The normal exit is reached when `range(2, n)` is empty.
    assert_unbound_like_cpython(
        "lda_no_else",
        "def f(n: int) -> int:\n    for d in range(2, n):\n        if n % d == 0:\n            break\n    return d\n\n\nprint(f(2))\n",
        "d",
    );
}

#[test]
fn a_name_bound_only_before_break_is_maybe_unbound_when_else_skips_it() {
    assert_unbound_like_cpython(
        "lda_else_skips",
        "def f(n: int) -> int:\n    i: int = 0\n    while i < n:\n        if i == 3:\n            x = i\n            break\n        i += 1\n    else:\n        i = -1\n    return x\n\n\nprint(f(1))\n",
        "x",
    );
}

#[test]
fn a_break_before_the_binding_is_maybe_unbound() {
    assert_unbound_like_cpython(
        "lda_break_first",
        "def f(n: int) -> int:\n    for i in range(n):\n        if i == 0:\n            break\n        x = i\n    else:\n        return 0\n    return x\n\n\nprint(f(5))\n",
        "x",
    );
}

#[test]
fn a_handler_that_falls_through_leaves_the_try_binding_maybe_unbound() {
    assert_unbound_like_cpython(
        "lda_handler_pass",
        "def g(v: int) -> int:\n    if v < 0:\n        raise ValueError(\"negative\")\n    return v\n\n\ndef f(n: int) -> int:\n    total: int = 0\n    for v in range(-1, n):\n        try:\n            r = g(v)\n        except ValueError:\n            pass\n        total += r\n    return total\n\n\nprint(f(3))\n",
        "r",
    );
}

#[test]
fn an_inner_break_does_not_bind_for_the_outer_loop_exit() {
    assert_unbound_like_cpython(
        "lda_inner_break",
        "def f(n: int) -> int:\n    while True:\n        for c in range(n):\n            if c == 2:\n                break\n        break\n    return c\n\n\nprint(f(0))\n",
        "c",
    );
}

const RAISER: &str = "def g(i: int) -> int:\n    if i == 0:\n        raise ValueError(\"first\")\n    return i\n\n\n";

#[test]
fn a_handler_as_name_is_unbound_after_a_break_out_of_the_handler() {
    // CPython deletes `e` when the `break` leaves the handler:
    // `UnboundLocalError` at `print(e)`.
    assert_maybe_unbound(
        "lda_handler_as_break",
        &format!(
            "{RAISER}def f() -> str:\n    while True:\n        try:\n            g(0)\n        except ValueError as e:\n            break\n    print(e)\n    return \"x\"\n\n\nprint(f())\n"
        ),
        "e",
    );
}

#[test]
fn a_handler_as_name_is_unbound_after_a_break_skipping_a_returning_else() {
    assert_maybe_unbound(
        "lda_handler_as_for_else",
        &format!(
            "{RAISER}def f() -> str:\n    for i in range(3):\n        try:\n            g(0)\n        except ValueError as e:\n            break\n    else:\n        return \"none\"\n    print(e)\n    return \"x\"\n\n\nprint(f())\n"
        ),
        "e",
    );
}

#[test]
fn a_module_scope_handler_as_name_is_unbound_after_a_break() {
    let dir = ScratchDir::new("lda_handler_as_module").expect("scratch");
    let src = dir.join("m.py");
    std::fs::write(
        &src,
        format!(
            "{RAISER}while True:\n    try:\n        g(0)\n    except ValueError as e:\n        break\nprint(e)\n"
        ),
    )
    .expect("write the source");
    let out = pycc()
        .arg("check")
        .arg(&src)
        .output()
        .expect("pycc should spawn");
    let stderr = format!("{}{}", text(&out.stdout), text(&out.stderr));
    assert!(!out.status.success(), "expected T0041, check passed");
    assert!(
        stderr.contains("error[T0041]") && stderr.contains("`e`"),
        "{stderr}"
    );
}

#[test]
fn a_continue_branch_does_not_bind_after_the_loop() {
    assert_unbound_like_cpython(
        "lda_continue_after",
        "def f(n: int) -> int:\n    for i in range(n):\n        if i % 2 == 0:\n            y = i\n        else:\n            continue\n    return y\n\n\nprint(f(0))\n",
        "y",
    );
}
