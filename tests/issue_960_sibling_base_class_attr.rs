//! Issue #960: a sibling MRO base's class attribute no longer shadows another
//! base's instance slot, end to end through the public `pycc` CLI.
//!
//! `pycc_hir`'s `reject_class_attr_collisions` walks only a class's *own*
//! `class_attrs` against its own MRO, so two independent sibling bases -- one
//! establishing an instance slot in `__init__`, the other declaring a
//! same-named class attribute -- are never compared with each other. That
//! shape is legal and `pycc_types::class::resolve_attr_get` already resolves
//! it CPython's way (an instance `__dict__` entry shadows a
//! non-data-descriptor class attribute), but `pycc_mir`'s instance `AttrGet`
//! arm folded the class attribute *before* looking for the slot. The result
//! was a silently wrong value, and -- when the two declared types differ -- a
//! `pycc_codegen` abort, because the checker and the lowering disagreed about
//! the read's type.
//!
//! The first three tests below fail under the old fold-first order and so
//! pin the fix itself; the remaining three are guards, for the #911 fold path
//! that must survive the reorder and for the two `pycc_types` gates this
//! change deliberately leaves rejecting. Every accepting test asserts the
//! program's *stdout*, never a bare exit 0: the whole defect was a wrong
//! value from a program that compiled fine. Rejections are matched on the
//! diagnostic code plus a message substring rather than on a rendered path,
//! which prints with forward slashes on Windows CI.

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

/// Asserts that `pycc check` rejects `source` with a diagnostic containing
/// both `code` and `needle`.
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
        rendered.contains(code) && rendered.contains(needle),
        "diagnostic for {tag} should contain {code:?} and {needle:?}, got:\n{rendered}"
    );
}

/// The issue's own program. Before the fix it printed `2` -- `B.x`'s folded
/// constant -- where CPython prints `1`.
#[test]
fn a_sibling_bases_class_attribute_does_not_shadow_the_instance_slot() {
    assert_runs(
        "issue960_read",
        "\
class A:
    def __init__(self) -> None:
        self.x = 1

class B:
    x: int = 2

class C(A, B):
    pass

c = C()
print(c.x)
",
        "1\n",
    );
}

/// The same shape with the two declared types *differing*. This is the worse
/// half of the defect: `pycc_types` resolved `c.x` to `A.x`'s `int` and
/// accepted `c.x + 1`, while `pycc_mir` folded `B.x`'s `str`, so
/// `pycc_codegen` aborted with `expected an int-or-bool operand, got str`.
/// Nothing rejects this program at any layer, so the disagreement surfaced as
/// an internal-error abort rather than a diagnostic.
#[test]
fn a_type_mismatched_sibling_class_attribute_no_longer_aborts_codegen() {
    assert_runs(
        "issue960_typed",
        "\
class A:
    def __init__(self) -> None:
        self.x = 1

class B:
    x: str = \"hi\"

class C(A, B):
    pass

c = C()
print(c.x + 1)
",
        "2\n",
    );
}

/// The same precedence through a `self.` read inside a method of the derived
/// class -- the read path `pycc_mir` lowers for `self`, not just for a
/// module-level binding.
#[test]
fn a_self_read_in_a_method_resolves_to_the_inherited_slot() {
    assert_runs(
        "issue960_self",
        "\
class A:
    def __init__(self) -> None:
        self.x = 3

class B:
    x: int = 2

class C(A, B):
    def doubled(self) -> int:
        return self.x * 2

c = C()
print(c.doubled())
",
        "6\n",
    );
}

/// Regression for #911: a class attribute that shares its name with *no*
/// instance slot anywhere in the MRO still folds to its constant, including
/// when the same class also reads a real slot. Pins that the reorder moved
/// the fold behind the slot lookup rather than removing it.
#[test]
fn a_class_attribute_with_no_matching_slot_still_folds() {
    assert_runs(
        "issue960_fold",
        "\
class A:
    def __init__(self) -> None:
        self.n = 4

class B:
    LIMIT: int = 8

class C(A, B):
    pass

c = C()
print(c.n)
print(c.LIMIT)
print(B.LIMIT)
",
        "4\n8\n8\n",
    );
}

/// The store side is deliberately unchanged: `pycc_types::class::check_attr_set`
/// consults `lookup_class_attr_through_mro` before the slot walk, so a write
/// to this shape stays `T0044` even though the read now resolves to `A`'s
/// slot. Lifting that would relax `docs/TYPE_SYSTEM.md`'s "every write path to
/// a class attribute is `T0044`" contract, which is a separate,
/// decision-bearing change; this test pins the current answer so the follow-up
/// has to update it deliberately.
#[test]
fn a_write_to_the_shadowed_name_is_still_rejected() {
    assert_rejected(
        "issue960_write",
        "\
class A:
    def __init__(self) -> None:
        self.x = 1

class B:
    x: int = 2

class C(A, B):
    pass

c = C()
c.x = 5
print(c.x)
",
        "T0044",
        "which is a compile-time constant with no storage to write to",
    );
}

/// The other conservative pre-check that asks only whether a class attribute
/// *exists* in the MRO: reading through a non-bare-name base stays `T0044`
/// even though this read would now resolve to a slot and therefore would not
/// discard the base expression. Same follow-up as the write case above.
#[test]
fn a_non_name_base_read_of_the_shadowed_name_is_still_rejected() {
    assert_rejected(
        "issue960_call_base",
        "\
class A:
    def __init__(self) -> None:
        self.x = 1

class B:
    x: int = 2

class C(A, B):
    pass

def make() -> C:
    return C()

print(make().x)
",
        "T0044",
        "can only be read through a plain name",
    );
}
