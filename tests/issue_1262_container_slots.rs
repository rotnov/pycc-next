//! End-to-end proof for `list[int]`/`dict[str, int]` instance attributes
//! ([#1262], Part 1 of [#1218]).
//!
//! `docs/TYPE_SYSTEM.md`'s class "Current state" paragraph is the contract.
//! The byte-exact oracle fixture is `tests/fixtures/instance_container_slots.py`
//! (registered in `tests/conformance/classes.rs`, pinned-oracle only); this
//! file runs the same fixture against its recorded CPython 3.14.7 output on
//! every CI job, adds a module-scope program checked against whatever
//! `python3` is available, and owns the refusals.
//!
//! [#1262]: https://github.com/rotnov/pycc/issues/1262
//! [#1218]: https://github.com/rotnov/pycc/issues/1218

use pycc_scratch::ScratchDir;
use std::path::Path;
use std::process::{Command, Output};

fn pycc() -> Command {
    Command::new(env!("CARGO_BIN_EXE_pycc"))
}

fn python() -> Command {
    Command::new(std::env::var_os("PYCC_PYTHON").unwrap_or_else(|| "python3".into()))
}

fn rendered(output: &Output) -> String {
    let mut text = String::from_utf8_lossy(&output.stdout).into_owned();
    text.push_str(&String::from_utf8_lossy(&output.stderr));
    text.replace("\r\n", "\n")
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).replace("\r\n", "\n")
}

/// Builds `source` into `dir/app`, runs it, and returns its standard output.
fn build_and_run(dir: &Path, source: &Path) -> String {
    let build = pycc()
        .arg("build")
        .arg(source)
        .arg("-o")
        .arg(dir.join("app"))
        .output()
        .expect("pycc should spawn");
    assert!(build.status.success(), "{}", rendered(&build));
    let run = Command::new(dir.join("app"))
        .output()
        .expect("the program should spawn");
    assert!(run.status.success(), "{}", rendered(&run));
    stdout(&run)
}

/// Runs `pycc check` on `source` and returns the rendered diagnostics,
/// asserting that the check failed.
fn check_fails(category: &str, source: &str) -> String {
    let dir = ScratchDir::new(category).expect("scratch");
    std::fs::write(dir.join("a.py"), source).expect("write the subject");
    let output = pycc()
        .arg("check")
        .arg(dir.join("a.py"))
        .output()
        .expect("pycc should spawn");
    assert!(!output.status.success(), "{source:?} was accepted");
    rendered(&output)
}

/// The oracle fixture, against CPython 3.14.7's recorded output: reads,
/// alias mutation of the list and the dict, a method reading the slot, a
/// reassignment in a method, and a subclass inheriting the base's slots.
#[test]
fn the_fixture_matches_its_recorded_cpython_output() {
    let dir = ScratchDir::new("e2e_1262_fixture").expect("scratch");
    let fixture =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/instance_container_slots.py");
    assert_eq!(
        build_and_run(&dir, &fixture),
        "3\n1 3\n2\n6\n4 4\n10\n3 7\n2 30\n4\n5 11 26 bag\n"
    );
}

/// The same slots at module scope, where the instance and the alias are
/// module globals, compared with CPython.
#[test]
fn module_scope_container_slots_match_cpython() {
    let dir = ScratchDir::new("e2e_1262_module").expect("scratch");
    let source = dir.join("a.py");
    std::fs::write(
        &source,
        "class Box:\n    def __init__(self, xs: list[int], d: dict[str, int]) -> None:\n        \
         self.xs = xs\n        self.d = d\n\n\nb = Box([4, 5], {\"k\": 9})\nys = b.xs\n\
         ys.append(6)\nprint(len(b.xs), b.xs[2], b.d[\"k\"])\nb.xs = [1]\n\
         print(len(b.xs), len(ys))\n",
    )
    .expect("write the subject");
    let pycc_out = build_and_run(&dir, &source);
    let oracle = python()
        .arg("a.py")
        .current_dir(&*dir)
        .output()
        .expect("python3 should spawn");
    assert!(oracle.status.success(), "{}", rendered(&oracle));
    assert_eq!(pycc_out, stdout(&oracle));
    assert_eq!(pycc_out, "3 6 9\n1 3\n");
}

/// A slice of a list attribute is a fresh list, as in CPython: appending to
/// it leaves the attribute unchanged.
#[test]
fn slicing_a_list_attribute_matches_cpython() {
    let dir = ScratchDir::new("e2e_1262_slice").expect("scratch");
    let source = dir.join("a.py");
    std::fs::write(
        &source,
        "class Box:\n    def __init__(self, xs: list[int]) -> None:\n        self.xs = xs\n\n\n\
         def main() -> None:\n    b = Box([1, 2, 3, 4])\n    ys = b.xs[1:3]\n    ys.append(9)\n    \
         print(len(ys), ys[0], ys[2], len(b.xs))\n\n\nmain()\n",
    )
    .expect("write the subject");
    let pycc_out = build_and_run(&dir, &source);
    let oracle = python()
        .arg("a.py")
        .current_dir(&*dir)
        .output()
        .expect("python3 should spawn");
    assert!(oracle.status.success(), "{}", rendered(&oracle));
    assert_eq!(pycc_out, stdout(&oracle));
    assert_eq!(pycc_out, "3 2 9 4\n");
}

/// A `set[int]` or `tuple[int, int]` parameter still cannot seed a slot, and
/// the message names what can.
#[test]
fn a_set_or_tuple_parameter_is_refused() {
    for (category, annotation) in [
        ("e2e_1262_set", "set[int]"),
        ("e2e_1262_tuple", "tuple[int, int]"),
    ] {
        let text = check_fails(
            category,
            &format!(
                "class C:\n    def __init__(self, s: {annotation}) -> None:\n        self.s = s\n"
            ),
        );
        assert!(text.contains("error[C0001]"), "{text}");
        assert!(
            text.contains(&format!(
                "cannot establish an attribute of type `{annotation}` yet -- only a scalar \
                 (int/float/bool/str), `list[int]`, `dict[str, int]`, class instance or \
                 foreign-imported class parameter is supported"
            )),
            "{text}"
        );
    }
}

/// A later store must match the slot's container type exactly.
#[test]
fn a_list_stored_into_a_dict_attribute_is_refused() {
    let text = check_fails(
        "e2e_1262_mismatch",
        "class C:\n    def __init__(self, d: dict[str, int]) -> None:\n        self.d = d\n\n\
         \x20   def bad(self, xs: list[int]) -> None:\n        self.d = xs\n",
    );
    assert!(
        text.contains(
            "error[T0021]: cannot assign `list[int]` to attribute `d` of type `dict[str, int]`"
        ),
        "{text}"
    );
}

/// Known limit: iterating an attribute directly is still refused; a local
/// alias (`items = self.xs`) is the supported spelling.
#[test]
fn iterating_a_container_attribute_directly_is_still_refused() {
    let text = check_fails(
        "e2e_1262_for",
        "class C:\n    def __init__(self, xs: list[int]) -> None:\n        self.xs = xs\n\n\
         \x20   def total(self) -> int:\n        acc = 0\n        for x in self.xs:\n            \
         acc = acc + x\n        return acc\n",
    );
    assert!(text.contains("error[I0404]"), "{text}");
}
