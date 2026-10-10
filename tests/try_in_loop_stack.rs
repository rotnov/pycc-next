//! A `try` inside a loop must not grow the stack per iteration.
//!
//! Every slot a `try` statement needs (the `finally` return flag and return
//! value, a handler's saved exception and its `as` binding, the `except*`
//! partition slots) is allocated in the function's entry block. Allocated
//! where the statement is emitted, each iteration of an enclosing loop added
//! a fresh stack slot that lives until the function returns; an optimized
//! build folded them away, but a debug build segfaulted after a few million
//! iterations.
//!
//! `tests/fixtures/try_in_loop_stack.py` runs five million iterations of a
//! `try`/`finally`, an `except ... as e:` and a `return` through `finally`,
//! in functions and at module scope. It is built *without* `--release` and
//! its stdout compared byte-for-byte with the live CPython oracle
//! (`PYCC_PYTHON`, default `python3`).

use pycc_scratch::ScratchDir;
use std::path::Path;
use std::process::Command;

const FIXTURE: &str = "tests/fixtures/try_in_loop_stack.py";

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).replace("\r\n", "\n")
}

#[test]
fn a_try_in_a_long_loop_runs_in_a_debug_build() {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join(FIXTURE);
    let dir = ScratchDir::new("try_in_loop_stack").expect("scratch");
    // Built from a copy, so no `pycc.toml` near the fixture can turn on
    // `release`.
    let src = dir.join("m.py");
    std::fs::copy(&fixture, &src).expect("copy the fixture");
    let binary = dir.join("app");
    let build = Command::new(env!("CARGO_BIN_EXE_pycc"))
        .arg("build")
        .arg(&src)
        .arg("-o")
        .arg(&binary)
        .output()
        .expect("pycc should spawn");
    assert!(
        build.status.success(),
        "pycc build failed:\n{}{}",
        text(&build.stdout),
        text(&build.stderr)
    );
    let run = Command::new(&binary).output().expect("the binary runs");
    assert!(
        run.status.success(),
        "the debug binary failed ({:?}):\n{}{}",
        run.status,
        text(&run.stdout),
        text(&run.stderr)
    );
    let oracle = std::env::var_os("PYCC_PYTHON").unwrap_or_else(|| "python3".into());
    let cpython = Command::new(oracle)
        .arg(&fixture)
        .output()
        .expect("python3 should spawn");
    assert!(cpython.status.success(), "{}", text(&cpython.stderr));
    assert_eq!(text(&run.stdout), text(&cpython.stdout));
}
