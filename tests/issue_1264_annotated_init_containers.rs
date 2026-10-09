//! End-to-end proof for annotated empty `list[int]`/`dict[str, int]`
//! instance-attribute initialisers ([#1264], Part 3 of [#1218]).
//!
//! `docs/TYPE_SYSTEM.md`'s class "Current state" paragraph is the contract.
//! The byte-exact oracle fixture is
//! `tests/fixtures/instance_annotated_container_slots.py` (registered in
//! `tests/conformance/classes.rs`, pinned-oracle only); this file runs the
//! same fixture against its recorded CPython 3.14.7 output on every CI job,
//! adds a module-scope program checked against whatever `python3` is
//! available (annotated attribute targets are Python 3.6 syntax), and owns
//! the refusals.
//!
//! [#1264]: https://github.com/rotnov/pycc/issues/1264
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

/// The oracle fixture, against CPython 3.14.7's recorded output: two
/// instances with independent containers, `.append`/`.get` and an alias
/// store, a subclass reading the inherited slot, a reset in a method, a
/// store from a free function, and a `this` receiver.
#[test]
fn the_fixture_matches_its_recorded_cpython_output() {
    let dir = ScratchDir::new("e2e_1264_fixture").expect("scratch");
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/instance_annotated_container_slots.py");
    assert_eq!(
        build_and_run(&dir, &fixture),
        "1 0\n3 -1\n9 5\n0 0\n1 7 7 0\n2 2\n"
    );
}

/// The same slots with module-scope instances, compared with CPython.
#[test]
fn module_scope_instances_match_cpython() {
    let dir = ScratchDir::new("e2e_1264_module").expect("scratch");
    let source = dir.join("a.py");
    std::fs::write(
        &source,
        "class Box:\n    def __init__(self) -> None:\n        self.xs: list[int] = []\n        \
         self.d: dict[str, int] = {}\n\n\nb = Box()\nc = Box()\nb.xs.append(4)\nys = c.xs\n\
         ys.append(5)\nys.append(6)\nd = b.d\nd[\"k\"] = 9\n\
         print(len(b.xs), len(c.xs), b.d.get(\"k\", 0), c.d.get(\"k\", 0))\n",
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
    assert_eq!(pycc_out, "1 2 9 0\n");
}

/// The element type is validated exactly as a local annotation's is.
#[test]
fn a_list_of_str_annotation_is_refused() {
    let text = check_fails(
        "e2e_1264_list_str",
        "class C:\n    def __init__(self) -> None:\n        self.xs: list[str] = []\n",
    );
    assert!(text.contains("error[T0034]"), "{text}");
}

/// A later annotated store must match the slot's container type.
#[test]
fn a_dict_annotation_on_a_list_slot_is_refused() {
    let text = check_fails(
        "e2e_1264_mismatch",
        "class C:\n    def __init__(self) -> None:\n        self.xs: list[int] = []\n\n\
         \x20   def bad(self) -> None:\n        self.xs: dict[str, int] = {}\n",
    );
    assert!(
        text.contains(
            "error[T0021]: cannot assign `dict[str, int]` to attribute `xs` of type `list[int]`"
        ),
        "{text}"
    );
}

/// An annotated store outside `__init__` does not declare a slot.
#[test]
fn an_annotated_store_to_an_undeclared_attribute_is_refused() {
    let text = check_fails(
        "e2e_1264_undeclared",
        "class C:\n    def __init__(self) -> None:\n        self.n = 0\n\n\n\
         def f(c: C) -> None:\n    c.ys: list[int] = []\n",
    );
    assert!(text.contains("error[T0044]"), "{text}");
}

/// Only `__init__`'s top-level statements declare a slot, so an annotated
/// store nested in a block declares none.
#[test]
fn an_annotated_store_nested_in_an_init_block_declares_no_slot() {
    let text = check_fails(
        "e2e_1264_nested",
        "class C:\n    def __init__(self, c: bool) -> None:\n        if c:\n            \
         self.xs: list[int] = []\n",
    );
    assert!(text.contains("error[T0044]"), "{text}");
}

/// Every other annotated attribute target names the admitted form.
#[test]
fn a_scalar_annotated_attribute_target_names_the_admitted_form() {
    let text = check_fails(
        "e2e_1264_scalar",
        "class C:\n    def __init__(self) -> None:\n        self.n: int = 0\n",
    );
    assert!(
        text.contains(
            "error[C0001]: an annotated attribute target annotated `int` with this value is not \
             supported yet -- only `<obj>.<attr>: list[int] = []` or `<obj>.<attr>: dict[str, \
             int] = {}` inside a function body is supported so far (the general annotated \
             attribute target is issue #891)"
        ),
        "{text}"
    );
}
