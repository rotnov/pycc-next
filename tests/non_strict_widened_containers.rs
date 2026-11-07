//! Non-strict mode keeps code compiled where native containers meet CPython
//! objects: a name whose every binding is a fresh container becomes an
//! object (aliasing kept), including the result of a function that only
//! returns fresh lists; a nested list display in an object list, a list
//! holding a big `int` literal, a `collections.defaultdict` the native dict
//! cannot hold and `sys.stdin.buffer` are CPython's own; and a chained
//! comparison over objects is the conjunction of its links. Each fixture
//! builds with nothing falling back to CPython and prints what CPython
//! prints.

use pycc_scratch::ScratchDir;
use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).replace("\r\n", "\n")
}

/// Runs `command` with `stdin` written to its standard input.
fn run(command: &mut Command, stdin: &str) -> std::process::Output {
    let mut child = command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("the program runs");
    child
        .stdin
        .take()
        .expect("piped stdin")
        .write_all(stdin.as_bytes())
        .expect("stdin is written");
    child.wait_with_output().expect("the program finishes")
}

fn assert_compiled_like_cpython(stem: &str, stdin: &str) {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join(format!("tests/fixtures/{stem}.py"));
    let dir = ScratchDir::new(stem).expect("scratch");
    let binary = dir.join("app");
    let build = Command::new(env!("CARGO_BIN_EXE_pycc"))
        .arg("build")
        .arg(&fixture)
        .arg("-o")
        .arg(&binary)
        .args(["--release", "--non-strict", "--report-fallbacks"])
        .output()
        .expect("pycc should spawn");
    assert!(build.status.success(), "{}", text(&pycc_scratch::lf(&build.stderr)));
    assert!(!text(&pycc_scratch::lf(&build.stderr)).contains("fallback:"), "{}", text(&pycc_scratch::lf(&build.stderr)));
    let oracle = std::env::var_os("PYCC_PYTHON").unwrap_or_else(|| "python3".into());
    let expected = run(Command::new(oracle).arg(&fixture), stdin);
    assert!(expected.status.success(), "{}", text(&pycc_scratch::lf(&expected.stderr)));
    let actual = run(&mut Command::new(&binary), stdin);
    assert!(actual.status.success(), "{}", text(&pycc_scratch::lf(&actual.stderr)));
    assert_eq!(text(&pycc_scratch::lf(&actual.stdout)), text(&pycc_scratch::lf(&expected.stdout)));
}

#[test]
fn widened_containers_stay_compiled_and_match_cpython() {
    assert_compiled_like_cpython("non_strict_widened_containers", "ab 1\n21\n");
}

#[test]
fn chained_comparisons_over_objects_match_cpython() {
    assert_compiled_like_cpython("object_chained_compare", "");
}
