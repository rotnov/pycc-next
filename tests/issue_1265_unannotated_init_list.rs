//! End-to-end proof for unannotated empty `list` instance-attribute
//! initialisers ([#1265], Part 4 of [#1218]).
//!
//! `docs/TYPE_SYSTEM.md`'s class "Current state" paragraph and D-245's
//! 2026-09-24 amendment for #1265 are the contract. The byte-exact oracle
//! fixture is `tests/fixtures/instance_unannotated_list_slots.py`
//! (registered in `tests/conformance/classes.rs`, pinned-oracle only); this
//! file runs the same fixture against its recorded CPython 3.14.7 output on
//! every CI job, adds programs checked against whatever `python3` is
//! available (every one is Python 3.6 syntax), and owns the refusals.
//!
//! [#1265]: https://github.com/rotnov/pycc/issues/1265
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

/// Builds and runs `entry` in `dir`, runs the same file under `python3`, and
/// asserts the two agree; returns pycc's output.
fn matches_cpython(dir: &Path, entry: &str) -> String {
    let pycc_out = build_and_run(dir, &dir.join(entry));
    let oracle = python()
        .arg(entry)
        .current_dir(dir)
        .output()
        .expect("python3 should spawn");
    assert!(oracle.status.success(), "{}", rendered(&oracle));
    assert_eq!(pycc_out, stdout(&oracle));
    pycc_out
}

/// Runs `pycc <subcommand> a.py` from a scratch directory holding `source`
/// as `a.py`, asserting failure, and returns the rendered diagnostics. The
/// relative path keeps the rendering independent of the scratch directory.
fn fails(category: &str, subcommand: &str, source: &str) -> String {
    let dir = ScratchDir::new(category).expect("scratch");
    std::fs::write(dir.join("a.py"), source).expect("write the subject");
    let mut command = pycc();
    command
        .arg(subcommand)
        .arg("a.py")
        .current_dir(dir.join("."));
    if subcommand == "build" {
        command.arg("-o").arg("app");
    }
    let output = command.output().expect("pycc should spawn");
    assert!(!output.status.success(), "{source:?} was accepted");
    rendered(&output)
}

const NO_PRODUCER: &str = "class Buffer:\n    def __init__(self) -> None:\n        \
                           self.xs = []\n\n    def size(self) -> int:\n        \
                           return len(self.xs)\n\n\nprint(Buffer().size())\n";

/// The oracle fixture, against CPython 3.14.7's recorded output: two
/// instances with independent containers, producers in another method and
/// in `__init__`, `.pop()`, a reset, a subclass typed by its inherited
/// slots, and a `this` receiver.
#[test]
fn the_fixture_matches_its_recorded_cpython_output() {
    let dir = ScratchDir::new("e2e_1265_fixture").expect("scratch");
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/instance_unannotated_list_slots.py");
    assert_eq!(
        build_and_run(&dir, &fixture),
        "2 0 1 100\n4 4 1\n0\n11 0\n25 2\n"
    );
}

/// Module-scope instances and an alias sum, compared with CPython.
#[test]
fn module_scope_instances_match_cpython() {
    let dir = ScratchDir::new("e2e_1265_module").expect("scratch");
    std::fs::write(
        dir.join("a.py"),
        "class Bag:\n    def __init__(self) -> None:\n        self.xs = []\n\n    \
         def add(self, v: int) -> None:\n        self.xs.append(v)\n\n\n\
         b = Bag()\nc = Bag()\nb.add(4)\nys = c.xs\nys.append(5)\nys.append(6)\n\
         t = 0\nfor y in ys:\n    t = t + y\nprint(len(b.xs), len(c.xs), t)\n",
    )
    .expect("write the subject");
    assert_eq!(matches_cpython(&dir, "a.py"), "1 2 11\n");
}

/// A module that also routes through the private-helper solver (an
/// unannotated private helper) still sees the resolved slot.
#[test]
fn the_solver_path_sees_the_resolved_slot() {
    let dir = ScratchDir::new("e2e_1265_solver").expect("scratch");
    std::fs::write(
        dir.join("a.py"),
        "class Bag:\n    def __init__(self) -> None:\n        self.xs = []\n\n    \
         def add(self, v: int) -> None:\n        self.xs.append(v)\n\n\n\
         def _twice(n):\n    return n * 2\n\n\n\
         def main() -> None:\n    b = Bag()\n    b.add(_twice(3))\n    \
         print(b.xs[0], len(b.xs))\n\n\nmain()\n",
    )
    .expect("write the subject");
    assert_eq!(matches_cpython(&dir, "a.py"), "6 1\n");
}

/// A subclass in another module is typed by its base's resolved slot.
#[test]
fn a_subclass_in_another_module_is_typed_by_the_base_slot() {
    let dir = ScratchDir::new("e2e_1265_two_modules").expect("scratch");
    std::fs::write(
        dir.join("store.py"),
        "class Log:\n    def __init__(self) -> None:\n        self.items = []\n\n    \
         def add(self, v: int) -> None:\n        self.items.append(v)\n",
    )
    .expect("write the base module");
    std::fs::write(
        dir.join("main.py"),
        "from store import Log\n\n\nclass Big(Log):\n    def __init__(self) -> None:\n        \
         self.items = []\n\n\ndef main() -> None:\n    b = Big()\n    b.add(7)\n    \
         print(len(b.items), b.items[0])\n\n\nmain()\n",
    )
    .expect("write the entry module");
    assert_eq!(matches_cpython(&dir, "main.py"), "1 7\n");
}

/// An unresolved slot in an imported module is reported against that
/// module's file.
#[test]
fn an_unresolved_slot_in_another_module_points_at_that_module() {
    let dir = ScratchDir::new("e2e_1265_two_modules_refused").expect("scratch");
    std::fs::write(
        dir.join("store.py"),
        "class Bare:\n    def __init__(self) -> None:\n        self.items = []\n",
    )
    .expect("write the base module");
    std::fs::write(
        dir.join("main.py"),
        "from store import Bare\n\nprint(Bare())\n",
    )
    .expect("write the entry module");
    let output = pycc()
        .arg("check")
        .arg(dir.join("main.py"))
        .output()
        .expect("pycc should spawn");
    assert!(!output.status.success());
    let text = rendered(&output);
    assert!(text.contains("error[T0003]"), "{text}");
    assert!(text.contains("store.py:1:1"), "{text}");
    assert!(text.contains("`self.items` in class `Bare`"), "{text}");
}

/// `pycc check` and `pycc build` render byte-identical refusals for a slot
/// no source types.
#[test]
fn check_and_build_report_the_same_t0003() {
    let check = fails("e2e_1265_check", "check", NO_PRODUCER);
    let build = fails("e2e_1265_build", "build", NO_PRODUCER);
    assert_eq!(
        check,
        "error[T0003]: an empty list literal has no inferable element type for `self.xs` in \
         class `Buffer`\n --> a.py:1:1\n  |\n1 | class Buffer:\n  | ^ an empty list literal \
         has no inferable element type for `self.xs` in class `Buffer`\n"
    );
    assert_eq!(check, build);
}

/// A base's unresolved slot redeclared by an annotated subclass is the
/// `T0003`, not D-210's `T0052` naming a placeholder type.
#[test]
fn an_unresolved_base_slot_is_t0003_rather_than_t0052() {
    let text = fails(
        "e2e_1265_t0052",
        "check",
        "class A:\n    def __init__(self) -> None:\n        self.xs = []\n\n\n\
         class B(A):\n    def __init__(self) -> None:\n        self.xs: list[int] = []\n\n\n\
         print(len(B().xs))\n",
    );
    assert!(
        text.contains("error[T0003]") && text.contains("`self.xs` in class `A`"),
        "{text}"
    );
    assert!(!text.contains("T0052"), "{text}");
}

/// A static method whose first parameter is named `self` is not one of the
/// instance's own methods, so its `append` types nothing.
#[test]
fn a_static_method_named_self_parameter_is_not_a_producer() {
    let text = fails(
        "e2e_1265_static",
        "check",
        "class Box:\n    def __init__(self) -> None:\n        self.xs = []\n\n    \
         @staticmethod\n    def fill(self: Box, v: int) -> None:\n        \
         self.xs.append(v)\n\n\nprint(len(Box().xs))\n",
    );
    assert!(
        text.contains("error[T0003]") && text.contains("`self.xs` in class `Box`"),
        "{text}"
    );
}

/// The unannotated `{}` stays `C0001`, naming the annotated spelling.
#[test]
fn an_unannotated_empty_dict_stays_refused() {
    let text = fails(
        "e2e_1265_dict",
        "check",
        "class Index:\n    def __init__(self) -> None:\n        self.d = {}\n\n\nprint(Index())\n",
    );
    assert!(text.contains("error[C0001]"), "{text}");
    assert!(text.contains("`self.d: dict[str, int] = {}`"), "{text}");
    assert!(text.contains("#891"), "{text}");
}

const SLOT: &str = "class B:\n    def __init__(self) -> None:\n        self.xs = []\n\n";

/// A `str` producer resolves the slot to `list[str]`, which D-105's
/// element gate refuses as `T0034`, not the unresolved-slot `T0003`.
#[test]
fn a_str_producer_resolves_the_slot_and_meets_the_element_gate() {
    let text = fails(
        "e2e_1265_str",
        "check",
        &format!(
            "{SLOT}    def add(self, v: str) -> None:\n        self.xs.append(v)\n\n\n\
             print(len(B().xs))\n"
        ),
    );
    assert!(text.contains("error[T0034]: list[str]"), "{text}");
    assert!(!text.contains("T0003"), "{text}");
}

/// The first producer wins across methods; a later conflicting append is
/// the ordinary element mismatch.
#[test]
fn a_later_conflicting_producer_is_an_element_mismatch() {
    let text = fails(
        "e2e_1265_conflict",
        "check",
        &format!(
            "{SLOT}    def a(self) -> None:\n        self.xs.append(1)\n\n    \
             def b(self) -> None:\n        self.xs.append(\"a\")\n\n\nprint(len(B().xs))\n"
        ),
    );
    assert!(
        text.contains("error[T0021]: cannot append `str` to a list of `int`"),
        "{text}"
    );
}

/// The slot resolves, but a reset of another shape is not rewritten, so the
/// reset itself (not the slot gate) is the `T0003`.
#[test]
fn a_shape_mismatched_reset_is_refused() {
    let text = fails(
        "e2e_1265_mismatched_reset",
        "check",
        &format!(
            "{SLOT}    def add(self, v: int) -> None:\n        self.xs.append(v)\n\n    \
             def clear(self) -> None:\n        self.xs = {{}}\n\n\nprint(len(B().xs))\n"
        ),
    );
    assert!(
        text.contains("error[T0003]: an empty dict literal has no inferable key/value types here"),
        "{text}"
    );
    assert!(!text.contains("`self.xs` in class `B`"), "{text}");
}

/// A producer after `self` is rebound to another class does not yield a
/// program that builds.
#[test]
fn a_rebound_receiver_does_not_build() {
    fails(
        "e2e_1265_rebound",
        "build",
        &format!(
            "class O:\n    def __init__(self) -> None:\n        self.xs: list[int] = []\n\n\n\
             {SLOT}    def m(self) -> None:\n        self = O()\n        \
             self.xs.append(1.5)\n\n\nprint(len(B().xs))\n"
        ),
    );
}

/// A static method's `str` append ahead of the real `int` producer does not
/// type the slot: the slot is `list[int]` and the static append mismatches.
#[test]
fn a_static_method_append_ahead_of_the_real_producer_does_not_type_the_slot() {
    let text = fails(
        "e2e_1265_static_first",
        "check",
        &format!(
            "{SLOT}    @staticmethod\n    def tag(self: B, v: str) -> None:\n        \
             self.xs.append(v)\n\n    def add(self, v: int) -> None:\n        \
             self.xs.append(v)\n\n\nprint(len(B().xs))\n"
        ),
    );
    assert!(
        text.contains("error[T0021]: cannot append `str` to a list of `int`"),
        "{text}"
    );
    assert!(!text.contains("T0034"), "{text}");
}

/// On the private-helper solver path the gate's `T0003` still wins.
#[test]
fn the_solver_path_reports_the_gate_t0003() {
    for subcommand in ["check", "build"] {
        let text = fails(
            "e2e_1265_solver_refused",
            subcommand,
            &format!(
                "{SLOT}    def size(self) -> int:\n        return len(self.xs)\n\n\n\
                 def _twice(n):\n    return n * 2\n\n\nprint(B().size(), _twice(2))\n"
            ),
        );
        assert!(
            text.contains(
                "error[T0003]: an empty list literal has no inferable element type for \
                 `self.xs` in class `B`"
            ),
            "{text}"
        );
    }
}

/// A renamed receiver is named as written, so the suggested annotation is one
/// the receiver rule (#1181) accepts.
#[test]
fn a_renamed_receiver_is_named_as_written() {
    let text = fails(
        "e2e_1265_this",
        "check",
        "class Buffer:\n    def __init__(this) -> None:\n        this.xs = []\n\n\n\
         print(Buffer())\n",
    );
    assert!(
        text.contains("`this.xs` in class `Buffer`") && !text.contains("self.xs"),
        "{text}"
    );
}

/// Rebinding the whole attribute (`self.xs = other`) in another method is
/// not an element-type source; the slot stays unresolved.
#[test]
fn a_whole_attribute_rebinding_leaves_the_slot_refused() {
    let text = fails(
        "e2e_1265_rebind_attr",
        "check",
        "class B:\n    def __init__(self) -> None:\n        self.xs = []\n\n    \
         def load(self, other: list[int]) -> None:\n        self.xs = other\n\n\n\
         print(len(B().xs))\n",
    );
    assert!(
        text.contains("error[T0003]") && text.contains("`self.xs` in class `B`"),
        "{text}"
    );
}

/// A local alias of `self` in `__init__` is not the receiver: the refusal
/// names the spelling that established the slot.
#[test]
fn a_local_alias_of_self_is_not_named_as_the_receiver() {
    let text = fails(
        "e2e_1265_alias",
        "check",
        "class Buffer:\n    def __init__(self) -> None:\n        me = self\n        \
         self.xs = []\n\n\nprint(Buffer())\n",
    );
    assert!(
        text.contains("`self.xs` in class `Buffer`") && !text.contains("me.xs"),
        "{text}"
    );
}
