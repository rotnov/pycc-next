//! A `return` out of a `for x in <object>:` loop releases the loop's
//! iterator, as CPython does when the return unwinds the loop (#42).
//!
//! Before the fix only the loop's normal exit, `break`, and exception edges
//! released the iterator, so every `return` from inside such a loop leaked
//! one: 100 `list_iterator` objects after 100 calls, against none under
//! CPython. The driver counts the `list_iterator` objects `gc` still tracks
//! after 100 calls of each shape: a plain and a `None` return, nested loops,
//! a `finally` inside the loop (the iterator outlives it), around it (the
//! return releases the iterator first), both at once, and a `finally` whose
//! own `return` from a second loop overrides the first. Every parameter is
//! a type the `--ext` boundary carries, so no shape runs in CPython instead.
//! The compiled module's output must equal the source module's under
//! CPython.

use pycc_scratch::ScratchDir;
use std::path::Path;
use std::process::{Command, Output};

const MODULE: &str = "return_releases_loop_iterator";

fn fixture(name: &str) -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join(name)
}

fn text(bytes: &[u8]) -> String {
    pycc_scratch::lf(&*String::from_utf8_lossy(bytes)).replace("\r\n", "\n")
}

fn drive(dir: &Path) -> Output {
    let run = Command::new(std::env::var_os("PYCC_PYTHON").unwrap_or_else(|| "python3".into()))
        .arg("-B")
        .arg(fixture(&format!("{MODULE}_driver.py")))
        .arg(dir)
        .output()
        .expect("python3 should spawn");
    assert!(
        run.status.success(),
        "stdout: {}\nstderr: {}",
        text(&run.stdout),
        text(&run.stderr)
    );
    run
}

#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn a_return_from_an_object_loop_releases_its_iterator_like_cpython() {
    let source = ScratchDir::new("return-loop-iter-src").expect("scratch");
    std::fs::copy(
        fixture(&format!("{MODULE}.py")),
        source.join(format!("{MODULE}.py")),
    )
    .expect("copy the fixture");
    let expected = text(&drive(&source).stdout);
    let measured: Vec<&str> = expected
        .lines()
        .filter(|line| line.contains(" leaked "))
        .collect();
    assert_eq!(measured.len(), 8, "{expected}");
    assert!(
        measured.iter().all(|line| line.contains(" leaked 0 ")),
        "CPython should leave no iterator behind:\n{expected}"
    );

    let compiled = ScratchDir::new("return-loop-iter-ext").expect("scratch");
    let build = Command::new(env!("CARGO_BIN_EXE_pycc"))
        .arg("build")
        .arg(fixture(&format!("{MODULE}.py")))
        .arg("-o")
        .arg(compiled.join(MODULE))
        .arg("--ext")
        .arg("--non-strict")
        .arg("--report-fallbacks")
        .output()
        .expect("pycc should spawn");
    assert!(build.status.success(), "{}", text(&build.stderr));
    assert_eq!(text(&build.stderr), "", "every shape must be compiled");
    assert_eq!(text(&drive(&compiled).stdout), expected);
}
