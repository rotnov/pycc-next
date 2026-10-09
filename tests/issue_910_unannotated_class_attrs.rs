//! Issue #910 (Part 2 of #885): un-annotated class-body assignments with a
//! literal right-hand side, end to end through the public `pycc` CLI.
//!
//! `X = 1` in a class body is the same compile-time constant #911 already
//! accepts for `X: int = 1`; the only difference is that its type is inferred
//! from the literal rather than read from an annotation. These tests pin the
//! accepted literal surface, the CPython-identical output it produces, and
//! the two rejections that are specific to this spelling (a bare assignment
//! in a `@dataclass` body, and `__slots__`).

use pycc_scratch::ScratchDir;
use std::io::Write;
use std::process::Command;

fn pycc_bin() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_BIN_EXE_pycc"))
}

fn write_fixture(dir: &std::path::Path, name: &str, source: &str) -> std::path::PathBuf {
    let path = dir.join(name);
    let mut f = std::fs::File::create(&path).unwrap();
    f.write_all(source.as_bytes()).unwrap();
    path
}

/// Builds and runs `source`, asserting the program's stdout.
fn assert_runs(tag: &str, source: &str, expected_stdout: &str) {
    let dir = ScratchDir::new(tag).expect("failed to create scratch dir");
    let src = write_fixture(&dir, "main.py", source);
    let out = dir.join("main.bin");
    let build = Command::new(pycc_bin())
        .args(["build", src.to_str().unwrap(), "-o", out.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(
        build.status.success(),
        "pycc build should succeed for {tag}:\n{}",
        String::from_utf8_lossy(&build.stdout)
    );
    let run = Command::new(&out).output().unwrap();
    assert!(run.status.success(), "compiled program {tag} should exit 0");
    assert_eq!(
        String::from_utf8_lossy(&run.stdout),
        expected_stdout,
        "stdout for {tag}"
    );
}

/// Asserts that `pycc check` rejects `source` with a diagnostic whose text
/// contains `needle`.
fn assert_rejected(tag: &str, source: &str, needle: &str) {
    let dir = ScratchDir::new(tag).expect("failed to create scratch dir");
    let src = write_fixture(&dir, "main.py", source);
    let out = Command::new(pycc_bin())
        .args(["check", src.to_str().unwrap()])
        .output()
        .unwrap();
    let rendered = String::from_utf8_lossy(&out.stdout).to_string();
    assert!(
        !out.status.success(),
        "pycc check should reject {tag}, but it succeeded"
    );
    assert!(
        rendered.contains(needle),
        "diagnostic for {tag} should contain {needle:?}, got:\n{rendered}"
    );
}

/// Every literal shape the inference accepts, read back through both the
/// class name and an instance. The `-1` / `+1` / `-1.5` entries are the
/// reason inference unwraps a unary `+`/`-` first: they parse as a `UnaryOp`
/// and are not literals at all.
const EVERY_LITERAL: &str = "\
class Config:
    COUNT = 1
    SCALE = 1.5
    DEBUG = True
    NAME = \"cfg\"
    FLOOR = -1024
    CEILING = +2048
    OFFSET = -1.5

    def __init__(self, n: int) -> None:
        self.n = n


def main() -> None:
    print(Config.COUNT)
    print(Config.SCALE)
    print(Config.DEBUG)
    print(Config.NAME)
    print(Config.FLOOR)
    print(Config.CEILING)
    print(Config.OFFSET)
    c = Config(7)
    print(c.COUNT)
    print(c.NAME)
    print(c.n)


main()
";

#[test]
fn every_inferred_literal_shape_folds_to_its_cpython_value() {
    assert_runs(
        "910_every_literal",
        EVERY_LITERAL,
        "1\n1.5\nTrue\ncfg\n-1024\n2048\n-1.5\n1\ncfg\n7\n",
    );
}

/// The two spellings are interchangeable in the same body, and an inferred
/// attribute participates in expressions exactly like an annotated one.
#[test]
fn an_inferred_and_an_annotated_attribute_coexist() {
    assert_runs(
        "910_mixed_spellings",
        "class C:\n    A = 2\n    B: int = 3\n\n\ndef main() -> None:\n    print(C.A * C.B)\n\n\nmain()\n",
        "6\n",
    );
}

/// An inferred attribute is a constant, so writing it is rejected by the
/// same `T0044` path Part 1 established for the annotated spelling.
#[test]
fn writing_an_inferred_class_attribute_is_rejected() {
    assert_rejected(
        "910_write",
        "class C:\n    X = 1\n\n    def __init__(self) -> None:\n        self.n = 0\n\n\nc = C()\nc.X = 5\n",
        "it is a class-level attribute of class `C`",
    );
}

/// In a `@dataclass` body a bare assignment is Python's class-level default
/// for a field declared elsewhere, not a constant -- so it keeps falling
/// through to the class-body catch-all rather than becoming a class
/// attribute (#378).
#[test]
fn a_bare_assignment_in_a_dataclass_body_stays_unsupported() {
    assert_rejected(
        "910_dataclass_assign",
        "from dataclasses import dataclass\n\n\n@dataclass\nclass C:\n    x: int\n    y = 5\n",
        "a `@dataclass` body statement must be a field declaration",
    );
}

/// Since #1368 both value-bound spellings of `__slots__` are admitted and
/// checked by `class::slots` rather than lowered as an ordinary constant; the
/// value-less annotation binds nothing in CPython and stays refused.
#[test]
fn slots_is_admitted_in_both_value_bound_spellings() {
    for (tag, binding) in [
        ("910_slots_bare", "__slots__ = \"a\""),
        ("910_slots_annotated", "__slots__: str = \"a\""),
    ] {
        assert_runs(
            tag,
            &format!(
                "class C:\n    {binding}\n\n    def __init__(self) -> None:\n        self.a = \
                 1\n\n\nprint(C().a)\n"
            ),
            "1\n",
        );
    }
    assert_rejected(
        "910_slots_value_less",
        "class C:\n    __slots__: str\n\n    def __init__(self) -> None:\n        self.a = 1\n",
        "this `__slots__` spelling is not supported yet",
    );
}

/// The #910 divergence that motivated extending the collision check to the
/// method tables: `B().f()` used to dispatch to `A.f` and print `1`, where
/// CPython raises `TypeError: 'int' object is not callable`.
#[test]
fn an_inferred_attribute_shadowing_an_inherited_method_is_rejected() {
    assert_rejected(
        "910_shadow_base_method",
        "class A:\n    def f(self) -> int:\n        return 1\n\n\nclass B(A):\n    f = 2\n",
        "collides with a method inherited from `A`",
    );
}

/// The remaining method-table collision arms, through the CLI: a class
/// attribute may not shadow a `@staticmethod` or a `@classmethod` either, of
/// its own class or of an MRO base.
#[test]
fn an_inferred_attribute_shadowing_a_static_or_class_method_is_rejected() {
    assert_rejected(
        "910_shadow_staticmethod",
        "class C:\n    f = 2\n\n    @staticmethod\n    def f() -> int:\n        return 1\n",
        "collides with a `@staticmethod`",
    );
    assert_rejected(
        "910_shadow_classmethod",
        "class C:\n    f = 2\n\n    @classmethod\n    def f(cls) -> int:\n        return 1\n",
        "collides with a `@classmethod`",
    );
    assert_rejected(
        "910_shadow_base_staticmethod",
        "class A:\n    @staticmethod\n    def f() -> int:\n        return 1\n\n\nclass B(A):\n    f = 2\n",
        "collides with a `@staticmethod` inherited from `A`",
    );
    assert_rejected(
        "910_shadow_base_classmethod",
        "class A:\n    @classmethod\n    def f(cls) -> int:\n        return 1\n\n\nclass B(A):\n    f = 2\n",
        "collides with a `@classmethod` inherited from `A`",
    );
}

/// The inferred spelling reaches the same literal/annotation reconciliation
/// as the annotated one, so a class attribute of one class may shadow an
/// unrelated name in another without interference.
#[test]
fn an_inferred_attribute_shadowing_a_method_of_an_unrelated_class_is_accepted() {
    assert_runs(
        "910_unrelated_name",
        "class A:\n    def f(self) -> int:\n        return 1\n\n\nclass B:\n    f = 2\n\n\ndef main() -> None:\n    print(A().f())\n    print(B.f)\n\n\nmain()\n",
        "1\n2\n",
    );
}

/// The non-method arms of the same MRO walk: an inferred class attribute may
/// not shadow a base's instance attribute or `@property` either. Both are
/// exercised through their own-class spelling elsewhere; these cover the
/// inherited wording the MRO branch produces.
#[test]
fn an_inferred_attribute_shadowing_an_inherited_slot_or_property_is_rejected() {
    assert_rejected(
        "910_shadow_base_instance_attr",
        "class A:\n    def __init__(self) -> None:\n        self.f = 1\n\n\nclass B(A):\n    f = 2\n",
        "collides with an instance attribute inherited from `A`",
    );
    assert_rejected(
        "910_shadow_base_property",
        "class A:\n    @property\n    def f(self) -> int:\n        return 1\n\n\nclass B(A):\n    f = 2\n",
        "collides with an `@property` inherited from `A`",
    );
}
