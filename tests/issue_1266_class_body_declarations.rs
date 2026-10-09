//! End-to-end proof for class-body instance attribute declarations
//! ([#1266], Part 5 of [#1218]).
//!
//! `docs/TYPE_SYSTEM.md`'s class "Current state" paragraph and D-245's
//! 2026-09-24 amendment for #1266 are the contract. The byte-exact oracle
//! fixture is `tests/fixtures/instance_declared_attrs.py` (registered in
//! `tests/conformance/classes.rs`, pinned-oracle only); this file runs the
//! same fixture against its recorded CPython 3.14.7 output on every CI job,
//! adds programs checked against whatever `python3` is available (none uses
//! PEP 695 syntax), proves a type-parameter declaration against hard-coded
//! output, and owns the refusals.
//!
//! [#1266]: https://github.com/rotnov/pycc/issues/1266
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

/// Writes `source` as `a.py` in a fresh scratch directory, builds and runs
/// it, runs the same file under `python3`, and asserts the two agree;
/// returns pycc's output.
fn matches_cpython(category: &str, source: &str) -> String {
    let dir = ScratchDir::new(category).expect("scratch");
    std::fs::write(dir.join("a.py"), source).expect("write the subject");
    let pycc_out = build_and_run(&dir, &dir.join("a.py"));
    let oracle = python()
        .arg("a.py")
        .current_dir(dir.join("."))
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

/// `class C:` declaring `x: <annotation>` with an `__init__` whose top level
/// assigns `self.x = <value>`.
fn declared(annotation: &str, value: &str) -> String {
    format!(
        "class C:\n    x: {annotation}\n\n    def __init__(self) -> None:\n        \
         self.x = {value}\n\n\nc = C()\n"
    )
}

/// The oracle fixture, against CPython 3.14.7's recorded output: scalar and
/// container declarations, `{}`/`[]` establishing the declared containers,
/// two instances with independent containers, a reset in a method, and a
/// subclass declaring its own attribute.
#[test]
fn the_fixture_matches_its_recorded_cpython_output() {
    let dir = ScratchDir::new("e2e_1266_fixture").expect("scratch");
    let fixture =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/instance_declared_attrs.py");
    assert_eq!(
        build_and_run(&dir, &fixture),
        "a 3 4.0 True 2 7 3\nb 1 2.5 1 8\n0 0 -1\nt! 1 2\n"
    );
}

/// A declared `dict[str, int]` established by `{}`, read through `len` and
/// `.get`, and a declared `list[int]` established by `[]` with no producer
/// in the class at all -- the declaration alone types it.
#[test]
fn declared_containers_established_by_empty_literals_match_cpython() {
    assert_eq!(
        matches_cpython(
            "e2e_1266_containers",
            "class Index:\n    d: dict[str, int]\n    xs: list[int]\n\n    \
             def __init__(self) -> None:\n        self.d = {}\n        self.xs = []\n\n\n\
             i = Index()\nm = i.d\nm[\"k\"] = 3\nys = i.xs\nys.append(9)\n\
             print(len(i.d), i.d.get(\"k\", 0), i.d.get(\"z\", -1), len(i.xs), i.xs[0])\n",
        ),
        "1 3 -1 1 9\n"
    );
}

/// A declaration after `__init__`, a renamed receiver, an `Annotated`
/// declaration, and an annotated establishing assignment that agrees.
#[test]
fn declaration_placement_and_spelling_variants_match_cpython() {
    assert_eq!(
        matches_cpython(
            "e2e_1266_variants",
            "from typing import Annotated\n\n\nclass P:\n    \
             def __init__(this, n: int) -> None:\n        this.n = n\n        \
             this.f = 1.5\n        this.xs: list[int] = []\n\n    \
             f: float\n    n: Annotated[int, \"meta\"]\n    xs: list[int]\n\n\n\
             p = P(4)\np.xs.append(p.n)\nprint(p.n, p.f, len(p.xs))\n",
        ),
        "4 1.5 1\n"
    );
}

/// An unannotated `self.xs = []` in a subclass resolves from a declared
/// base slot.
#[test]
fn a_subclass_reset_resolves_from_a_declared_base_slot() {
    assert_eq!(
        matches_cpython(
            "e2e_1266_subclass",
            "class Base:\n    xs: list[int]\n\n    def __init__(self) -> None:\n        \
             self.xs = []\n\n\nclass Child(Base):\n    def __init__(self) -> None:\n        \
             self.xs = []\n        self.xs.append(2)\n\n\nprint(len(Child().xs))\n",
        ),
        "1\n"
    );
}

/// A subclass that repeats the base's declarations with the same types and
/// establishes them again in its own `__init__` shares the base slots.
#[test]
fn a_subclass_redeclaring_a_base_declaration_matches_cpython() {
    assert_eq!(
        matches_cpython(
            "e2e_1266_redeclare",
            "class Base:\n    n: int\n    xs: list[int]\n\n    def __init__(self) -> None:\n        \
             self.n = 1\n        self.xs = []\n\n\nclass Child(Base):\n    n: int\n    \
             xs: list[int]\n\n    def __init__(self) -> None:\n        super().__init__()\n        \
             self.n = 5\n        self.xs = []\n        self.xs.append(self.n)\n\n\n\
             c = Child()\nprint(c.n, len(c.xs), c.xs[0], Base().n)\n",
        ),
        "5 1 5 1\n"
    );
}

/// A subclass redeclaration with a different type is D-210's `T0052`.
#[test]
fn a_subclass_redeclaration_with_another_type_is_t0052() {
    let dir = ScratchDir::new("e2e_1266_redeclare_conflict").expect("scratch");
    let subject = dir.join("a.py");
    std::fs::write(
        &subject,
        "class Base:\n    n: int\n\n    def __init__(self) -> None:\n        self.n = 1\n\n\n\
         class Child(Base):\n    n: str\n\n    def __init__(self) -> None:\n        \
         self.n = \"a\"\n\n\nprint(Child().n)\n",
    )
    .expect("write the subject");
    let check = pycc()
        .arg("check")
        .arg(&subject)
        .output()
        .expect("run pycc check");
    assert!(!check.status.success(), "{}", rendered(&check));
    assert!(
        rendered(&check).contains(
            "error[T0052]: attribute `n` is declared as `str` in class `Child` and as `int` in class `Base`"
        ),
        "{}",
        rendered(&check)
    );
}

/// A type-parameter declaration, against hard-coded CPython output (PEP 695
/// syntax needs Python 3.12, so this program is not run under an arbitrary
/// `python3`).
#[test]
fn a_type_parameter_declaration_builds_and_runs() {
    let dir = ScratchDir::new("e2e_1266_generic").expect("scratch");
    std::fs::write(
        dir.join("a.py"),
        "class Box[T]:\n    value: T\n\n    def __init__(self, value: T) -> None:\n        \
         self.value = value\n\n\nbi = Box[int](3)\nbs = Box[str](\"s\")\n\
         n: int = bi.value\nprint(n + 1, bs.value)\n",
    )
    .expect("write the subject");
    assert_eq!(build_and_run(&dir, &dir.join("a.py")), "4 s\n");
}

/// An empty literal of the wrong shape for its declared slot is the
/// checker's `T0003`, never an inferred placeholder type or #891's wording.
#[test]
fn a_wrong_shape_empty_literal_is_t0003() {
    for (index, (annotation, value)) in [
        ("list[int]", "{}"),
        ("dict[str, int]", "[]"),
        ("int", "[]"),
        ("int", "{}"),
        ("str", "{}"),
    ]
    .into_iter()
    .enumerate()
    {
        let text = fails(
            &format!("e2e_1266_shape_{index}"),
            "check",
            &declared(annotation, value),
        );
        assert!(text.contains("error[T0003]"), "{annotation}: {text}");
        assert!(!text.contains("inferred"), "{annotation}: {text}");
        assert!(!text.contains("#891"), "{annotation}: {text}");
    }
    for (index, value) in ["[]", "{}"].into_iter().enumerate() {
        let text = fails(
            &format!("e2e_1266_shape_param_{index}"),
            "check",
            &format!(
                "class Box[T]:\n    v: T\n\n    def __init__(self) -> None:\n        \
                 self.v = {value}\n"
            ),
        );
        assert!(text.contains("error[T0003]"), "{value}: {text}");
    }
}

/// `check` and `build` agree on a wrong-shape refusal.
#[test]
fn check_and_build_report_the_same_t0003() {
    let source = declared("int", "[]");
    let check = fails("e2e_1266_check", "check", &source);
    let build = fails("e2e_1266_build", "build", &source);
    assert_eq!(check, build);
}

/// A declared `float` does not widen an `int` value: the checker's ordinary
/// attribute-assignment rule applies.
#[test]
fn a_declared_float_refuses_an_int_value() {
    let text = fails("e2e_1266_float", "check", &declared("float", "2"));
    assert!(
        text.contains("error[T0021]: cannot assign `int` to attribute `x` of type `float`"),
        "{text}"
    );
}

/// Every new `C0001`, end to end.
#[test]
fn each_declaration_refusal_is_c0001() {
    for (category, source, message) in [
        (
            "e2e_1266_unestablished",
            "class C:\n    x: int\n    x = 1\n".to_string(),
            "instance attribute `x` declared in class `C` is never assigned at the top level of \
             `C.__init__` -- assign it there (`self.x = ...`), or give it a value (`x: int = \
             1`) to make it a class constant",
        ),
        (
            "e2e_1266_collision",
            "class C:\n    x: int\n\n    def __init__(self) -> None:\n        self.x = 1\n\n    \
             def x(self) -> int:\n        return 1\n"
                .to_string(),
            "instance attribute `x` declared in class `C` collides with the method `x` of the \
             same class",
        ),
        (
            "e2e_1266_duplicate",
            "class C:\n    x: int\n    x: int\n".to_string(),
            "instance attribute `x` is declared more than once in class `C` -- declare it once",
        ),
        (
            "e2e_1266_none",
            "class C:\n    x: None\n".to_string(),
            "instance attribute `x` declared in class `C` has type `None`, which has no \
             instance-slot representation",
        ),
        (
            "e2e_1266_disagree",
            "class C:\n    x: list[int]\n\n    def __init__(self) -> None:\n        \
             self.x: dict[str, int] = {}\n"
                .to_string(),
            "instance attribute `x` is declared in class `C` as `list[int]` but annotated \
             `dict[str, int]` in `__init__` -- the two annotations must agree",
        ),
        (
            "e2e_1266_param",
            "class Box[T]:\n    v: T\n\n    def __init__(self, v: T) -> None:\n        \
             self.v = 3\n"
                .to_string(),
            "instance attribute `v` declared in class `Box` as `T` is assigned a value of type \
             `int`",
        ),
        (
            "e2e_1266_bare_list",
            "class C:\n    x: list\n".to_string(),
            "a bare `list` type annotation is not supported yet -- write the parameterized \
             form, e.g. `list[int]`",
        ),
    ] {
        let text = fails(category, "check", &source);
        assert!(text.contains("error[C0001]"), "{category}: {text}");
        assert!(text.contains(message), "{category}: {text}");
    }
}

/// A declaration beside a class constant of the same name keeps #911's own
/// collision, and a container element type keeps its annotation code.
#[test]
fn existing_refusals_keep_their_own_diagnostics() {
    let text = fails(
        "e2e_1266_class_attr",
        "check",
        "class C:\n    x: int\n\n    def __init__(self) -> None:\n        self.x = 5\n\n    \
         x = 1\n",
    );
    assert!(
        text.contains("class attribute `C.x` collides with an instance attribute"),
        "{text}"
    );
    let text = fails("e2e_1266_list_str", "check", "class C:\n    x: list[str]\n");
    assert!(text.contains("error[T0034]"), "{text}");
}

/// An unannotated `self.d = {}` stays refused, now naming both spellings
/// that type it.
#[test]
fn an_unannotated_empty_dict_names_both_spellings() {
    let text = fails(
        "e2e_1266_dict",
        "check",
        "class C:\n    def __init__(self) -> None:\n        self.d = {}\n",
    );
    assert!(text.contains("error[C0001]"), "{text}");
    assert!(text.contains("`self.d: dict[str, int] = {}`"), "{text}");
    assert!(text.contains("(`d: dict[str, int]`)"), "{text}");
}
