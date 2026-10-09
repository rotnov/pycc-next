//! Issue #913 (follow-up to #911, Part 1 of #885): `typing.ClassVar` inside a
//! `@dataclass` body, end to end through the public `pycc` CLI.
//!
//! PEP 557 says a `ClassVar`-annotated name is **not** a dataclass field, so
//! it must be excluded from the synthesized `__init__`, `__eq__` and
//! `__repr__`. In this compiler it is excluded from a fourth thing as well:
//! `pycc_hir::class::lower_class` fills the D-154 instance-slot layout
//! (`HirClassDef::attrs`) from the very same merged field list, so a name that
//! never becomes a field occupies no slot either. #911 rejected the spelling
//! outright because merely *stripping* the wrapper would have turned it into a
//! required `__init__` parameter; #913 routes it to the class-attribute path
//! instead, where it is the compile-time constant it should have been.
//!
//! These tests pin the accepted surface, the two read forms, and every
//! rejection that keeps the model honest -- including three shapes that are
//! only reachable because `ClassVar` is now accepted here at all.

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

/// Asserts that `pycc check` rejects `source` with a diagnostic carrying
/// `code` and containing `needle`.
fn assert_rejected(tag: &str, source: &str, code: &str, needle: &str) {
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
        rendered.contains(code),
        "diagnostic for {tag} should carry {code}, got:\n{rendered}"
    );
    assert!(
        rendered.contains(needle),
        "diagnostic for {tag} should contain {needle:?}, got:\n{rendered}"
    );
}

// -- the accepted surface --------------------------------------------------

/// The issue's own reproduction. CPython 3.13.9 prints exactly these four
/// lines: the `ClassVar` is absent from `__init__` (so `P(1, 2)` still takes
/// two arguments), absent from `__repr__` (`P(x=1, y=2)`, not
/// `P(x=1, y=2, LIMIT=10)`), and absent from `__eq__`.
#[test]
fn a_class_var_in_a_dataclass_body_is_excluded_from_the_synthesized_methods() {
    assert_runs(
        "913_dataclass_classvar",
        "from dataclasses import dataclass\n\
         from typing import ClassVar\n\
         \n\
         \n\
         @dataclass\n\
         class P:\n\
         \x20   x: int\n\
         \x20   y: int\n\
         \x20   LIMIT: ClassVar[int] = 10\n\
         \n\
         \n\
         def main() -> None:\n\
         \x20   p = P(1, 2)\n\
         \x20   print(p.x + p.y)\n\
         \x20   print(p.LIMIT)\n\
         \x20   print(p)\n\
         \x20   print(P(1, 2) == P(1, 2))\n\
         \n\
         \n\
         main()\n",
        "3\n10\nP(x=1, y=2)\nTrue\n",
    );
}

/// Both read forms on the declaring class itself: through an instance and
/// through the class name. Also pins that a `str` `ClassVar` folds, so the
/// accepted value grammar is #911's, not a narrower dataclass-only one.
#[test]
fn a_dataclass_class_var_reads_through_the_instance_and_the_class_name() {
    assert_runs(
        "913_dataclass_classvar_reads",
        "from dataclasses import dataclass\n\
         from typing import ClassVar\n\
         \n\
         \n\
         @dataclass\n\
         class P:\n\
         \x20   x: int\n\
         \x20   LIMIT: ClassVar[int] = 10\n\
         \x20   KIND: ClassVar[str] = \"point\"\n\
         \n\
         \x20   def limit(self) -> int:\n\
         \x20       return self.LIMIT\n\
         \n\
         \n\
         def main() -> None:\n\
         \x20   p = P(1)\n\
         \x20   print(p.LIMIT)\n\
         \x20   print(P.LIMIT)\n\
         \x20   print(p.KIND)\n\
         \x20   print(p.limit())\n\
         \n\
         \n\
         main()\n",
        "10\n10\npoint\n10\n",
    );
}

/// A derived dataclass overriding a base's `ClassVar` is an ordinary
/// override, not a collision: `lookup_class_attr_through_mro` resolves
/// most-derived-first, and the MRO loop in `reject_class_attr_collisions`
/// deliberately does not consult a base's class attributes. CPython 3.13.9
/// agrees line for line -- `B(1)` is `B(x=1)`, `A.LIMIT` stays `8`.
#[test]
fn a_derived_dataclass_may_override_a_base_dataclasss_class_var() {
    assert_runs(
        "913_dataclass_classvar_override",
        "from dataclasses import dataclass\n\
         from typing import ClassVar\n\
         \n\
         \n\
         @dataclass\n\
         class A:\n\
         \x20   x: int\n\
         \x20   LIMIT: ClassVar[int] = 8\n\
         \n\
         \n\
         @dataclass\n\
         class B(A):\n\
         \x20   LIMIT: ClassVar[int] = 9\n\
         \n\
         \n\
         def main() -> None:\n\
         \x20   b = B(1)\n\
         \x20   print(b.LIMIT)\n\
         \x20   print(A.LIMIT)\n\
         \x20   print(B.LIMIT)\n\
         \x20   print(b)\n\
         \n\
         \n\
         main()\n",
        "9\n8\n9\nB(x=1)\n",
    );
}

// -- rejections that keep the model honest ---------------------------------

/// A field and a `ClassVar` of the same name in one dataclass body, in
/// either declaration order. CPython's two orders disagree with *each
/// other*: declaring the field first drops it entirely (`A()` takes no
/// argument), declaring the `ClassVar` first turns it into a field whose
/// default is the class attribute's value. Neither is representable while
/// dataclass field defaults are unsupported.
#[test]
fn a_dataclass_field_colliding_with_an_own_class_var_is_rejected() {
    assert_rejected(
        "913_own_collision_field_first",
        "from typing import ClassVar\n\n\n@dataclass\nclass P:\n    x: int\n    x: ClassVar[int] = 1\n",
        "C0001",
        "dataclass field `x` of class `P` shares its name with the class attribute `P.x`",
    );
    assert_rejected(
        "913_own_collision_class_var_first",
        "from typing import ClassVar\n\n\n@dataclass\nclass P:\n    x: ClassVar[int] = 1\n    x: int\n",
        "C0001",
        "dataclass field `x` of class `P` shares its name with the class attribute `P.x`",
    );
}

/// A derived dataclass declaring a field whose name is a base's `ClassVar`.
/// CPython turns the base's value into the field's *default*
/// (`inspect.signature(B.__init__)` is `(self, LIMIT: int = 8) -> None`), so
/// the synthesized constructor's arity would silently differ. Only reachable
/// because #913 lets a base dataclass carry a `ClassVar` at all.
#[test]
fn a_dataclass_field_over_an_inherited_class_var_is_rejected() {
    assert_rejected(
        "913_inherited_class_var_collision",
        "from typing import ClassVar\n\n\n@dataclass\nclass A:\n    LIMIT: ClassVar[int] = 8\n\n\n@dataclass\nclass B(A):\n    LIMIT: int\n",
        "C0001",
        "dataclass field `LIMIT` of class `B` shares its name with the class attribute `A.LIMIT`",
    );
}

/// The cross-base split, in both base orders. CPython processes fields in
/// reverse-MRO order, so `D(A, B)` lets `A`'s `ClassVar` *remove* `B`'s
/// field while `D(B, A)` keeps it -- an order dependence the fold model
/// cannot represent. `D(B, A)` is therefore a deliberate conservative
/// rejection of a program CPython runs; `D(A, B)` would otherwise be a
/// mis-compile. #969's slot-layout gate catches neither, because a class
/// contributing only a `ClassVar` declares no instance attributes.
#[test]
fn a_dataclass_field_colliding_with_a_sibling_bases_class_var_is_rejected() {
    for (tag, bases) in [
        ("913_cross_base_class_var_first", "A, B"),
        ("913_cross_base_field_first", "B, A"),
    ] {
        assert_rejected(
            tag,
            &format!(
                "from typing import ClassVar\n\n\n@dataclass\nclass A:\n    LIMIT: ClassVar[int] = 8\n\n\n@dataclass\nclass B:\n    LIMIT: int\n    other: int\n\n\n@dataclass\nclass D({bases}):\n    pass\n"
            ),
            "C0001",
            "dataclass field `LIMIT` of class `D` shares its name with the class attribute `A.LIMIT`",
        );
    }
}

/// A derived `ClassVar` shadowing a field inherited from a base dataclass.
/// CPython drops the field from `B.__init__` (`(self) -> None`); pycc's
/// existing `reject_class_attr_collisions` MRO walk already rejects it,
/// because a base dataclass's `attrs` *is* its merged field list. Kept as an
/// intentional conservative rejection, and only reachable since #913.
#[test]
fn a_derived_class_var_shadowing_an_inherited_dataclass_field_is_rejected() {
    assert_rejected(
        "913_class_var_over_inherited_field",
        "from typing import ClassVar\n\n\n@dataclass\nclass A:\n    v: int\n\n\n@dataclass\nclass B(A):\n    v: ClassVar[int] = 5\n",
        "C0001",
        "collides with an instance attribute inherited from `A`",
    );
}

/// A `ClassVar` named after a dunder the dataclass relies on implicitly.
/// CPython's `dataclasses` uses `_set_new_attribute`, which leaves an
/// existing class `__dict__` entry alone -- so `A.__repr__` really is `8`
/// and an `__init__` class attribute leaves the class with no constructor
/// at all. pycc synthesizes `__init__`/`__eq__`/`__repr__` unconditionally,
/// and additionally rewrites `!=` through the synthesized `__eq__`
/// (bypassing `__ne__`) and both `print(instance)` and an f-string
/// interpolation through the synthesized `__repr__` (bypassing `__str__`
/// and `__format__`). Verified against CPython 3.13.9: each of the six
/// raises `TypeError: 'int' object is not callable` at the use site, where
/// pycc would have silently succeeded (D-235).
#[test]
fn a_class_var_named_after_an_implicit_dataclass_dunder_is_rejected() {
    for (tag, name) in [
        ("913_class_var_named_init", "__init__"),
        ("913_class_var_named_eq", "__eq__"),
        ("913_class_var_named_repr", "__repr__"),
        ("913_class_var_named_ne", "__ne__"),
        ("913_class_var_named_str", "__str__"),
        ("913_class_var_named_format", "__format__"),
    ] {
        assert_rejected(
            tag,
            &format!(
                "from typing import ClassVar\n\n\n@dataclass\nclass P:\n    x: int\n    {name}: ClassVar[int] = 8\n"
            ),
            "C0001",
            &format!("a `ClassVar` named `{name}` is not allowed in a `@dataclass` body"),
        );
    }
}

/// A value-less `ClassVar` is legal in CPython (the name simply has no
/// value) but stays `C0001` here, exactly as it does outside a dataclass
/// body: #911's model folds a class attribute to a literal at every read,
/// and there is nothing to fold. Routing through the shared
/// `lower_class_attr` is what makes this hold without a dataclass-specific
/// check.
#[test]
fn a_value_less_class_var_in_a_dataclass_body_is_rejected() {
    assert_rejected(
        "913_class_var_no_value",
        "from typing import ClassVar\n\n\n@dataclass\nclass P:\n    x: int\n    LIMIT: ClassVar[int]\n",
        "C0001",
        "class attribute `LIMIT` has no value",
    );
}

/// The shared `lower_class_attr` restrictions apply unchanged inside a
/// dataclass body -- a non-scalar annotation is rejected there too.
#[test]
fn a_non_scalar_class_var_in_a_dataclass_body_is_rejected() {
    assert_rejected(
        "913_class_var_non_scalar",
        "from typing import ClassVar\n\n\n@dataclass\nclass P:\n    x: int\n    LIMIT: ClassVar[None] = 1\n",
        "C0001",
        "which is not a scalar slot type",
    );
}

/// Diagnostic precedence. `T0052` is raised inside the dataclass field-merge
/// loop, which runs before the new field/class-attribute check, so a program
/// carrying both defects keeps reporting `T0052`. Pinned so a later reorder
/// of these checks cannot pass unnoticed.
#[test]
fn a_dataclass_field_type_conflict_outranks_the_class_var_collision() {
    assert_rejected(
        "913_precedence_t0052",
        "from typing import ClassVar\n\n\n@dataclass\nclass A:\n    v: int\n    LIMIT: ClassVar[int] = 8\n\n\n@dataclass\nclass B(A):\n    v: float\n    LIMIT: int\n",
        "T0052",
        "attribute `v` is declared as `int` in a base class and as `float`",
    );
}

// -- pre-existing #911 behaviour, pinned rather than changed ----------------

/// A class-name-qualified read of an *inherited* class attribute succeeds,
/// matching the instance-qualified read of the same attribute. Until
/// [#974](https://github.com/rotnov/pycc/issues/974) this was `T0044`:
/// `pycc_types`'s class-name arm searched only the literally named class's
/// own `class_attrs` and never walked the MRO, an asymmetry that was
/// pre-existing #911 behaviour rather than anything #913 introduced -- a
/// `@dataclass` was simply the shape that made it visible. This test now
/// pins the fixed answer from the `@dataclass`/`ClassVar` side; the full
/// case matrix lives in
/// `tests/issue_974_inherited_class_attr_by_class_name.rs`.
#[test]
fn a_class_name_read_of_an_inherited_dataclass_class_var_resolves() {
    assert_runs(
        "913_inherited_class_name_read",
        "from typing import ClassVar\n\n\n@dataclass\nclass A:\n    x: int\n    LIMIT: ClassVar[int] = 8\n\n\n@dataclass\nclass B(A):\n    y: int\n\n\ndef main() -> None:\n    print(B.LIMIT)\n\n\nmain()\n",
        "8\n",
    );
}
