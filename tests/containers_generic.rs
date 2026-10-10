//! Generic dicts, sets and tuples: keys, values and elements of every type
//! the runtime containers store (`pycc_rt::{dict, set, tuple}`), CPython's
//! set iteration order, and the container builtins and methods over them;
//! and the source rewrites that bring common code to them: lifted nested
//! functions, `key=`/`reverse=`, comprehension tuple targets, generator
//! arguments, item deletion, set operators and empty containers typed by
//! later use.
//!
//! The oracle is CPython itself: each fixture is built in strict mode with
//! `--release`, run, and compared byte for byte with `python3` running the
//! same file -- stdout, the uncaught exception's last line and the exit
//! status.

use pycc_scratch::ScratchDir;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn pycc() -> Command {
    Command::new(env!("CARGO_BIN_EXE_pycc"))
}

fn python() -> Command {
    Command::new(std::env::var_os("PYCC_PYTHON").unwrap_or_else(|| "python3".into()))
}

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).replace("\r\n", "\n")
}

fn last_line(bytes: &[u8]) -> String {
    text(bytes).lines().last().unwrap_or_default().to_string()
}

fn assert_same_as_cpython(ours: &Output, theirs: &Output) {
    assert_eq!(text(&ours.stdout), text(&theirs.stdout), "stdout differs");
    assert_eq!(last_line(&ours.stderr), last_line(&theirs.stderr));
    assert_eq!(ours.status.code(), theirs.status.code());
}

/// Builds `name` (strict mode unless `flags` says otherwise), runs it, and
/// compares with CPython.
fn matches_cpython(name: &str, flags: &[&str]) {
    let dir = ScratchDir::new("containers_generic").expect("scratch");
    let out = dir.join("prog");
    let build = pycc()
        .arg("build")
        .arg(fixture(name))
        .arg("-o")
        .arg(&out)
        .arg("--release")
        .args(flags)
        .output()
        .expect("pycc should spawn");
    assert!(
        build.status.success(),
        "{}{}",
        text(&build.stdout),
        text(&build.stderr)
    );
    let ours = Command::new(&out).output().expect("the program should run");
    let theirs = python()
        .arg(fixture(name))
        .output()
        .expect("python3 should spawn");
    assert_same_as_cpython(&ours, &theirs);
}

#[test]
fn generic_dicts_sets_and_tuples_match_cpython() {
    matches_cpython("containers_generic.py", &[]);
}

#[test]
fn closures_keys_and_inferred_containers_match_cpython() {
    matches_cpython("closures_keys.py", &[]);
}

/// A missing key of a dict of struct tuples or lists raises `KeyError`
/// before the absent value is read, caught and uncaught alike.
#[test]
fn a_missing_key_of_a_tuple_or_list_valued_dict_raises_key_error() {
    matches_cpython("containers_keyerror.py", &[]);
}
