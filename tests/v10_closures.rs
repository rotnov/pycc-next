//! v0.10: nested functions used as values. A capture every binding of
//! which precedes the `def` (a parameter normalized first) is bound when
//! the value is made, and `list(map(...))`/`list(filter(...))`/
//! `sorted(key=...)` of such a value call its lifted function directly
//! (`pycc_ast::desugar::keys::apply`)
//! instead of handing a function object to CPython's builtin. The oracle is
//! `python3` running the fixture: strict and `--non-strict` native builds
//! and the `--non-strict --ext` module must print the same, with no
//! function, method or class running in CPython.

use pycc_scratch::ScratchDir;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn pycc() -> Command {
    Command::new(env!("CARGO_BIN_EXE_pycc"))
}

fn python() -> Command {
    Command::new(std::env::var_os("PYCC_PYTHON").unwrap_or_else(|| "python3".into()))
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).replace("\r\n", "\n")
}

fn last_line(bytes: &[u8]) -> String {
    text(bytes).lines().last().unwrap_or_default().to_string()
}

fn fixture(name: &str, dir: &Path, as_name: &str) -> PathBuf {
    let source = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(format!("{name}.py"));
    let path = dir.join(as_name);
    std::fs::copy(source, &path).expect("copy the fixture");
    path
}

fn assert_matches(ours: &Output, oracle: &Output) {
    assert_eq!(text(&pycc_scratch::lf(&ours.stdout)), text(&pycc_scratch::lf(&oracle.stdout)), "{}", text(&pycc_scratch::lf(&ours.stderr)));
    assert_eq!(ours.status.code(), oracle.status.code(), "{}", text(&pycc_scratch::lf(&ours.stderr)));
    assert_eq!(last_line(&pycc_scratch::lf(&ours.stderr)), last_line(&pycc_scratch::lf(&oracle.stderr)));
}

/// Builds `source` with `flags`, asserting it builds and that no function,
/// method or class runs in CPython.
fn build(source: &Path, out: &Path, flags: &[&str]) -> String {
    let build = pycc()
        .args(["build", "--report-fallbacks"])
        .args(flags)
        .arg(source)
        .arg("-o")
        .arg(out)
        .output()
        .expect("pycc should spawn");
    let report = text(&pycc_scratch::lf(&build.stderr));
    assert!(build.status.success(), "{report}");
    for unit in ["fallback: function", "fallback: method", "fallback: class"] {
        assert!(!report.contains(unit), "nothing should run in CPython:\n{report}");
    }
    report
}

fn native(name: &str, flags: &[&str]) {
    let dir = ScratchDir::new(&format!("{name}_native")).expect("scratch");
    let source = fixture(name, &dir, "m.py");
    let all = flags.to_vec();
    build(&source, &dir.join("app"), &all);
    let ours = Command::new(dir.join("app")).output().expect("run the build");
    let oracle = python().arg(&source).output().expect("python should spawn");
    assert_matches(&ours, &oracle);
}

fn ext(name: &str, flags: &[&str]) {
    let compiled = ScratchDir::new(&format!("{name}_ext")).expect("scratch");
    let source = fixture(name, &compiled, "m.py");
    let mut all = vec!["--ext", "--non-strict"];
    all.extend_from_slice(flags);
    build(&source, &compiled.join("m"), &all);
    std::fs::remove_file(&source).expect("import the artifact, not the source");
    let plain = ScratchDir::new(&format!("{name}_ext_src")).expect("scratch");
    fixture(name, &plain, "m.py");
    let run = |dir: &Path| {
        python()
            .args(["-c", "import m"])
            .current_dir(dir)
            .output()
            .expect("python should spawn")
    };
    assert_matches(&run(&compiled), &run(&plain));
}

#[test]
fn closure_values_strict() {
    native("v10_closure_values", &["--release"]);
}

#[test]
fn closure_values_non_strict() {
    native("v10_closure_values", &["--non-strict"]);
}

#[test]
fn closure_values_ext() {
    ext("v10_closure_values", &["--release"]);
}

#[test]
fn closure_rebound_after_def_non_strict() {
    native("v10_closure_late", &["--non-strict"]);
    ext("v10_closure_late", &[]);
}

/// `reverse=` is evaluated before any key call, and a raising one stops
/// them all (such a call is not rewritten, so part of the function may run
/// in CPython: only the output is compared, [`output_matches`]).
#[test]
fn key_calls_follow_every_argument_non_strict() {
    output_matches("v10_closure_key_order");
}

/// A `sorted` call with a nested function key keys a copy of its input
/// also where the program shadows `list`.
#[test]
fn a_shadowed_list_keeps_sorted_as_written() {
    output_matches("v10_closure_key_shadowed");
}

/// `name`'s `--non-strict` native build prints what CPython prints (part of
/// it may run in CPython).
fn output_matches(name: &str) {
    let dir = ScratchDir::new(name).expect("scratch");
    let source = fixture(name, &dir, "m.py");
    let build = pycc()
        .args(["build", "--non-strict"])
        .arg(&source)
        .arg("-o")
        .arg(dir.join("app"))
        .output()
        .expect("pycc should spawn");
    assert!(build.status.success(), "{}", text(&build.stderr));
    let ours = Command::new(dir.join("app")).output().expect("run the build");
    let oracle = python().arg(&source).output().expect("python should spawn");
    assert_matches(&ours, &oracle);
}

/// `sorted` keys a copy of its input: a nested function key that changes
/// the list does not change what is sorted.
#[test]
fn sorted_keys_a_copy_of_its_input() {
    native("v10_closure_key_snapshot", &[]);
    native("v10_closure_key_snapshot", &["--non-strict"]);
    ext("v10_closure_key_snapshot", &[]);
}

/// `min`/`max` compare each key as CPython does: a key that changes a key
/// it returned earlier, or one of CPython objects, keeps the call as
/// written.
#[test]
fn min_max_keys_run_in_cpythons_order() {
    output_matches("v10_closure_key_minmax");
}

/// `sorted` keys a copy of its whole input: a property getter the key runs
/// does not change what is sorted, and an input that raises part way raises
/// before the first key call.
#[test]
fn sorted_keys_a_copy_made_before_any_key_call() {
    output_matches("v10_closure_key_property");
    output_matches("v10_closure_key_input_raises");
    native("v10_closure_key_input_raises", &["--non-strict"]);
    ext("v10_closure_key_input_raises", &[]);
}

/// A `map`/`filter` not consumed at once stays lazy.
#[test]
fn a_lazy_map_reads_state_when_consumed() {
    output_matches("v10_closure_map_lazy");
}
