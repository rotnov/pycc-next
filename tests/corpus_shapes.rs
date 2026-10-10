//! Shapes common in real-world scripts (the competitive-programming corpus,
//! `bench/corpus.py`) compiled in strict mode.
//!
//! The oracle is CPython itself: each fixture is built with `--release`, run
//! with the same stdin as `python3` running the file, and compared byte for
//! byte -- stdout, the uncaught exception's last line and the exit status.

use pycc_scratch::ScratchDir;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

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

fn run_with_stdin(mut command: Command, stdin: &str) -> Output {
    let mut child = command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("the program should spawn");
    child
        .stdin
        .take()
        .unwrap()
        .write_all(stdin.as_bytes())
        .unwrap();
    child.wait_with_output().unwrap()
}

/// Builds fixture `name` in strict mode (plus `flags`), runs it with
/// `stdin`, and compares with CPython.
fn matches_cpython(name: &str, flags: &[&str], stdin: &str) {
    let dir = ScratchDir::new("corpus_shapes").expect("scratch");
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
    let ours = run_with_stdin(Command::new(&out), stdin);
    let mut theirs = python();
    theirs.arg(fixture(name));
    let theirs = run_with_stdin(theirs, stdin);
    assert_eq!(text(&ours.stdout), text(&theirs.stdout), "stdout differs");
    assert_eq!(last_line(&ours.stderr), last_line(&theirs.stderr));
    assert_eq!(ours.status.code(), theirs.status.code());
}

/// List and dict comprehensions in a module whose solver runs (an inferred
/// helper is present) are typed from their elements, not as `list[int]`.
#[test]
fn comprehension_containers_are_typed_from_their_elements() {
    matches_cpython("corpus_comprehension_types.py", &[], "2\n1 2 3\n4 5\n");
}

/// `input = sys.stdin.readline` (and the other stream aliases) are typed by
/// the operation they alias, so the values they return type empty
/// containers and builtin calls downstream.
#[test]
fn stdin_and_stdout_aliases_are_typed() {
    matches_cpython(
        "corpus_stdin_alias.py",
        &[],
        "2\nabcdefghijklmnopqrstuvwxyz\nhello\nqwertyuiopasdfghjklzxcvbnm\nabc\nleft over\n",
    );
}

/// `math`'s float functions, logarithms of exact ints, `factorial`/`comb`/
/// `perm`, its constants, and CPython's domain and range errors.
#[test]
fn math_functions_match_cpython() {
    matches_cpython("corpus_math.py", &[], "");
}

/// `from M import x as y` of a stdlib function or constant, and
/// `from M import *` of a module whose exports are known.
#[test]
fn aliased_and_wildcard_imports_match_cpython() {
    matches_cpython("corpus_imports.py", &[], "16\n");
}

/// A name rebound to an unrelated value (`n = int(n)`, a loop variable
/// reused over another type) is one variable per live range.
#[test]
fn a_name_rebound_to_another_type_is_split_per_live_range() {
    matches_cpython("corpus_rebinding.py", &[], "21\n1 2 3\n2\nhello\n");
}

/// `P = print`-style builtin aliases, `a() if c else b()` as a statement,
/// and an explicit `object` base.
#[test]
fn script_idioms_match_cpython() {
    matches_cpython("corpus_idioms.py", &[], "4\n");
}

/// Inserting into or deleting from a dict (or set) while a `for` loop
/// iterates it raises CPython's `RuntimeError` at the next step; a value
/// update or a `break` right after the change does not.
#[test]
fn dict_and_set_size_changes_during_iteration_raise() {
    matches_cpython("corpus_dict_mutation.py", &[], "");
}

/// `heapq` and `bisect` on native lists, through every import spelling,
/// with CPython's tie order and `IndexError`.
#[test]
fn heapq_and_bisect_match_cpython() {
    matches_cpython("corpus_heap_bisect.py", &[], "");
}

/// A name whose two types meet at one read stays one variable, refused.
#[test]
fn a_name_holding_two_types_at_one_read_is_still_refused() {
    let dir = ScratchDir::new("corpus_shapes_union").expect("scratch");
    let source = dir.join("m.py");
    std::fs::write(
        &source,
        "n = int(input())\ny = \"keep\"\nif n > 100:\n    y = 5\nprint(y)\n",
    )
    .unwrap();
    let output = pycc().arg("check").arg(&source).output().unwrap();
    assert_eq!(output.status.code(), Some(1));
    let text = text(&output.stdout);
    assert!(text.contains("T0023"), "{text}");
}

/// A local named like a stdlib module still shadows it for a function the
/// builtin table types, as for every other stdlib symbol.
#[test]
fn a_local_shadowing_math_refuses_a_table_typed_math_call() {
    let dir = ScratchDir::new("corpus_shapes_shadow").expect("scratch");
    let source = dir.join("m.py");
    std::fs::write(
        &source,
        "import math\n\n\ndef f(math: float) -> float:\n    return math.log(2.0)\n\n\nprint(f(1.0))\n",
    )
    .unwrap();
    let output = pycc().arg("check").arg(&source).output().unwrap();
    assert_eq!(output.status.code(), Some(1));
    let text = text(&output.stdout);
    assert!(text.contains("C0001"), "{text}");
    assert!(text.contains("not the stdlib `math` module"), "{text}");
}
