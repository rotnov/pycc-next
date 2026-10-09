//! Issue #916: `Final[...]` on a class-body attribute declaration, end to end
//! through the public `pycc` CLI.
//!
//! PEP 591 explicitly allows `Final` on a class-body attribute. #911 rejected
//! the spelling because `Final`'s non-reassignability is tracked by the type
//! checker's `Environment.finals` set, which is populated from
//! `HirStmt::AnnAssign` -- and a class attribute produces no `HirStmt` at
//! all. #916 accepts it: every write path to a #911 class attribute is
//! already `T0044`, so the binding is immutable without `Environment`
//! knowing anything about it, and `Final[T]` there is a documentation-only
//! wrapper over an already-final binding.
//!
//! These tests pin the accepted surface, the two PEP 591-invalid nestings,
//! and -- the load-bearing one -- that `Final[T]` and a plain `T` are
//! observably indistinguishable, which is what makes the acceptance safe.
//!
//! Every rejection is asserted by diagnostic code and message substring
//! only. A rendered path is not compared: the renderer prints forward
//! slashes on every platform, so a path assertion would need explicit
//! separator normalization (see `tests/issue_941_enum_subclass.rs`).

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
///
/// The compiled program is executed rather than merely built: a silent exit
/// 0 from `pycc check` would not prove the folded constant is the right one.
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

/// Asserts that `pycc check` rejects `source`, returning the rendered
/// diagnostic so the caller can assert on its code and message.
fn check_rejected(tag: &str, source: &str) -> String {
    let dir = ScratchDir::new(tag).expect("failed to create scratch dir");
    let src = write_fixture(&dir, "main.py", source);
    let out = Command::new(pycc_bin())
        .args(["check", src.to_str().unwrap()])
        .output()
        .unwrap();
    let rendered = String::from_utf8_lossy(&out.stdout).to_string();
    assert!(
        !out.status.success(),
        "pycc check should reject {tag}, but it succeeded:\n{rendered}"
    );
    rendered
}

/// Asserts that `pycc check` rejects `source` with a diagnostic containing
/// `needle`.
fn assert_rejected(tag: &str, source: &str, needle: &str) {
    let rendered = check_rejected(tag, source);
    assert!(
        rendered.contains(needle),
        "diagnostic for {tag} should contain {needle:?}, got:\n{rendered}"
    );
}

/// The program from the issue report, verbatim.
const ISSUE_PROGRAM: &str = "\
from typing import Final


class C:
    X: Final[int] = 1

    def __init__(self) -> None:
        self.n = 0


print(C.X)
";

#[test]
fn the_issue_program_compiles_and_prints_its_constant() {
    assert_runs("916_issue", ISSUE_PROGRAM, "1\n");
}

/// PEP 591 allows a bare `Final` with an inferred type. The class-body
/// position reuses #910's own literal inference rather than rejecting it.
#[test]
fn a_bare_final_class_attribute_compiles_and_prints_its_constant() {
    assert_runs(
        "916_bare",
        "from typing import Final\n\n\nclass C:\n    X: Final = 1\n\n    def __init__(self) -> None:\n        self.n = 0\n\n\nprint(C.X)\n",
        "1\n",
    );
}

/// Every literal shape a bare `Final` can infer, folded and printed.
#[test]
fn a_bare_final_class_attribute_infers_every_literal_shape() {
    assert_runs(
        "916_bare_shapes",
        "from typing import Final\n\n\nclass C:\n    I: Final = 1\n    F: Final = 1.5\n    B: Final = True\n    S: Final = \"cfg\"\n    NI: Final = -1024\n\n    def __init__(self) -> None:\n        self.n = 0\n\n\nprint(C.I)\nprint(C.F)\nprint(C.B)\nprint(C.S)\nprint(C.NI)\n",
        "1\n1.5\nTrue\ncfg\n-1024\n",
    );
}

/// The load-bearing test: `Final[int]` and a plain `int` are observably
/// indistinguishable across every read path a class attribute has -- the
/// class name, an instance, `self`, and `super()` ([#915]). If these ever
/// diverge, the `Final` wrapper is carrying meaning this compiler does not
/// model, and accepting the spelling was wrong.
///
/// [#915]: https://github.com/rotnov/pycc/issues/915
#[test]
fn a_final_class_attribute_reads_identically_to_a_plain_one() {
    const PROGRAM: &str = "\
class Base:
    LIMIT: {ANNOTATION} = 7

    def __init__(self) -> None:
        self.n = 0


class Derived(Base):
    def __init__(self) -> None:
        super().__init__()

    def via_super(self) -> int:
        return super().LIMIT

    def via_self(self) -> int:
        return self.LIMIT


d = Derived()
print(Base.LIMIT)
print(d.LIMIT)
print(d.via_self())
print(d.via_super())
";
    const EXPECTED: &str = "7\n7\n7\n7\n";
    assert_runs(
        "916_equiv_plain",
        &PROGRAM.replace("{ANNOTATION}", "int"),
        EXPECTED,
    );
    assert_runs(
        "916_equiv_final",
        &format!(
            "from typing import Final\n\n\n{}",
            PROGRAM.replace("{ANNOTATION}", "Final[int]")
        ),
        EXPECTED,
    );
}

/// A `Final` class attribute stays read-only: the wrapper adds no new
/// guarantee because the binding was already `T0044` on every write path,
/// and the plain annotation is rejected identically.
#[test]
fn writing_to_a_final_class_attribute_is_still_rejected() {
    const PROGRAM: &str = "\
class C:
    X: {ANNOTATION} = 1

    def __init__(self) -> None:
        self.n = 0

    def bump(self) -> None:
        self.X = 2


print(C().X)
";
    let plain = check_rejected("916_write_plain", &PROGRAM.replace("{ANNOTATION}", "int"));
    let final_ = check_rejected(
        "916_write_final",
        &format!(
            "from typing import Final\n\n\n{}",
            PROGRAM.replace("{ANNOTATION}", "Final[int]")
        ),
    );
    assert!(
        plain.contains("T0044"),
        "the plain spelling should still be T0044, got:\n{plain}"
    );
    assert!(
        final_.contains("T0044"),
        "the `Final` spelling should still be T0044, got:\n{final_}"
    );
}

// -- PEP 591-invalid nestings ---------------------------------------------

/// PEP 591 forbids `ClassVar[Final[T]]`. `class/body.rs` strips the
/// `ClassVar` wrapper before the class-attribute path sees the annotation,
/// so without the threaded `is_class_var` flag this would arrive
/// indistinguishable from a valid `Final[int]` and be silently accepted.
#[test]
fn a_class_var_wrapping_final_is_rejected_with_a_specific_message() {
    let rendered = check_rejected(
        "916_classvar_final",
        "from typing import ClassVar, Final\n\n\nclass C:\n    X: ClassVar[Final[int]] = 1\n\n    def __init__(self) -> None:\n        self.n = 0\n\n\nprint(C.X)\n",
    );
    assert!(rendered.contains("C0001"), "got:\n{rendered}");
    assert!(
        rendered.contains("forbids nesting `Final` inside `ClassVar`"),
        "got:\n{rendered}"
    );
}

/// PEP 591 forbids the opposite nesting too, in both the subscripted and the
/// bare inner spelling. The message must name the nesting rather than
/// inheriting `annotation_to_ty`'s generic "`ClassVar` is only valid on a
/// class-body attribute declaration", which would be actively misleading
/// here -- this *is* a class-body attribute declaration.
#[test]
fn a_final_wrapping_class_var_is_rejected_with_a_specific_message() {
    for (tag, annotation) in [
        ("916_final_classvar_sub", "Final[ClassVar[int]]"),
        ("916_final_classvar_bare", "Final[ClassVar]"),
    ] {
        let rendered = check_rejected(
            tag,
            &format!(
                "from typing import ClassVar, Final\n\n\nclass C:\n    X: {annotation} = 1\n\n    def __init__(self) -> None:\n        self.n = 0\n\n\nprint(C.X)\n"
            ),
        );
        assert!(rendered.contains("C0001"), "{tag} got:\n{rendered}");
        assert!(
            rendered.contains("forbids nesting `ClassVar` inside `Final`"),
            "{tag} got:\n{rendered}"
        );
    }
}

/// `Final` takes exactly one type argument here, reusing the variable-level
/// position's own wording.
#[test]
fn a_multi_argument_final_class_attribute_is_rejected() {
    assert_rejected(
        "916_final_arity",
        "from typing import Final\n\n\nclass C:\n    X: Final[int, str] = 1\n\n    def __init__(self) -> None:\n        self.n = 0\n\n\nprint(C.X)\n",
        "Final takes exactly one type argument",
    );
}

/// A value-less `Final` class attribute has nothing to fold, in either the
/// annotated or the bare spelling.
#[test]
fn a_value_less_final_class_attribute_is_rejected() {
    for (tag, annotation) in [
        ("916_no_value_sub", "Final[int]"),
        ("916_no_value_bare", "Final"),
    ] {
        assert_rejected(
            tag,
            &format!(
                "from typing import Final\n\n\nclass C:\n    X: {annotation}\n\n    def __init__(self) -> None:\n        self.n = 0\n"
            ),
            "has no value",
        );
    }
}

/// The `Final` wrapper is transparent to the scalar-slot restriction that
/// keeps `__set_name__` untriggerable (#585/D-213).
#[test]
fn a_non_scalar_final_class_attribute_is_rejected() {
    assert_rejected(
        "916_non_scalar",
        "from typing import Final\n\n\nclass C:\n    X: Final[None] = 1\n\n    def __init__(self) -> None:\n        self.n = 0\n",
        "which is not a scalar slot type",
    );
}

// -- `@dataclass` bodies are unchanged ------------------------------------

/// #916 does not widen into #913. A `@dataclass` field never reaches the
/// class-attribute path at all, so a `Final`-annotated field behaves exactly
/// as the plain one does: a default is still rejected, and a required field
/// still works.
#[test]
fn a_final_dataclass_field_follows_the_plain_spelling() {
    assert_rejected(
        "916_dataclass_default",
        "from dataclasses import dataclass\nfrom typing import Final\n\n\n@dataclass\nclass C:\n    x: Final[int] = 1\n\n\nprint(C(2).x)\n",
        "dataclass field defaults are not supported yet",
    );
    assert_runs(
        "916_dataclass_required",
        "from dataclasses import dataclass\nfrom typing import Final\n\n\n@dataclass\nclass C:\n    x: Final[int]\n\n\nprint(C(2).x)\n",
        "2\n",
    );
}
