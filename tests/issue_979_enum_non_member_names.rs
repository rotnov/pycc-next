//! Issue #979 ([D-238]): an `Enum`-body assignment CPython's `_EnumDict` keeps
//! out of the member list, end to end through the public `pycc` CLI.
//!
//! [D-238]: ../docs/decisions/D-238-reject-enum-body-assignments-cpython-keeps-out-of-the.md
//!
//! The defect class is D-198 false acceptance: pycc compiled and ran programs
//! whose observable behavior differs from CPython's.
//! `class C(Enum): __repr__ = 1` followed by `B = 2` iterated two members here
//! and one under CPython 3.13.9, and `C.__repr__.value` printed `1` here where
//! CPython raises `AttributeError: 'int' object has no attribute 'value'`,
//! because `lower_enum_class` lowers every assignment as a member and has no
//! non-member class-attribute representation at all.
//!
//! Four properties are pinned here, each of which a narrower fix would have
//! missed:
//!
//! 1. **Four shapes, not one.** The issue proposed a dunder-only predicate.
//!    CPython's rule is three sibling branches of one function
//!    (`enum._EnumDict.__setitem__`'s `_is_private` / `_is_sunder` /
//!    `_is_dunder`), and all three were measured to diverge: `__x = 1; B = 2`
//!    is not a dunder at all, and `_order_ = 'B'; B = 'b'` slips past the
//!    value-kind mismatch that accidentally rejected `_order_ = 'B'; B = 2`.
//!    `_is_private` needs two arms rather than one, because it matches the
//!    *raw* key: `_C__x = 1` inside `class C(Enum)` is kept out of the member
//!    list even though nothing mangled it.
//! 2. **That arm is class-name-keyed.** The identical `_C__x = 1` inside
//!    `class D(Enum)` is an ordinary member on CPython 3.13.9, so a shape-only
//!    `_*__*` predicate would trade one divergence for another.
//! 3. **`_x = 1` stays a member**, under both engines -- as does `_foo = 1`.
//! 4. **The guard is `Enum`-route-only.** In a plain class body every one of
//!    these names is an ordinary class attribute under both engines.
//!
//! The in-crate sibling is `crates/pycc_hir/src/tests/enum_non_member_names.rs`;
//! this file exists because the diagnostic must reach the user through
//! `pycc check`, which the unit tests do not exercise.
//!
//! `tests/issue_975_reserved_dunder_class_attrs.rs` keeps the inverted
//! `__repr__` pin and D-236's own four names, whose messages this change must
//! not repoint.

use pycc_scratch::ScratchDir;
use std::io::Write;
use std::process::Command;

fn pycc_bin() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_BIN_EXE_pycc"))
}

fn write_fixture(dir: &std::path::Path, source: &str) -> std::path::PathBuf {
    let path = dir.join("main.py");
    let mut f = std::fs::File::create(&path).unwrap();
    f.write_all(source.as_bytes()).unwrap();
    path
}

/// Asserts that `pycc check` rejects `source` with `C0001` and a diagnostic
/// containing `needle`.
fn assert_rejected(tag: &str, source: &str, needle: &str) {
    let dir = ScratchDir::new(tag).expect("failed to create scratch dir");
    let src = write_fixture(&dir, source);
    let out = Command::new(pycc_bin())
        .args(["check", src.to_str().unwrap()])
        .output()
        .unwrap();
    let rendered = String::from_utf8_lossy(&out.stdout).to_string();
    assert!(
        !out.status.success(),
        "pycc check should reject {tag}, but it succeeded:\n{rendered}"
    );
    assert!(
        rendered.contains("C0001"),
        "diagnostic for {tag} should carry C0001, got:\n{rendered}"
    );
    assert!(
        rendered.contains(needle),
        "diagnostic for {tag} should contain {needle:?}, got:\n{rendered}"
    );
}

/// Asserts that `pycc check` accepts `source`.
fn assert_accepted(tag: &str, source: &str) {
    let dir = ScratchDir::new(tag).expect("failed to create scratch dir");
    let src = write_fixture(&dir, source);
    let out = Command::new(pycc_bin())
        .args(["check", src.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "pycc check should accept {tag}, got:\n{}",
        String::from_utf8_lossy(&out.stdout)
    );
}

/// A whole module whose `Enum` body binds `name` to `value`, alongside a
/// member `B`.
fn enum_module(name: &str, value: &str, member: &str) -> String {
    format!(
        "from enum import Enum\n\
         \n\
         \n\
         class C(Enum):\n\
         \x20   {name} = {value}\n\
         \x20   B = {member}\n\
         \n\
         \n\
         def main() -> int:\n\
         \x20   return 0\n"
    )
}

/// The four shapes and the distinctive fragment of each one's message.
///
/// Every name is written for [`enum_module`]'s `class C`, because the fourth
/// is class-name-keyed.
const NON_MEMBER: [(&str, &str, &str); 4] = [
    (
        "979_enum_dunder",
        "__repr__",
        "a dunder-named assignment in an `Enum` body is not supported yet",
    ),
    (
        "979_enum_private",
        "__x",
        "a name-mangled private assignment in an `Enum` body is not supported yet",
    ),
    (
        "979_enum_mangled_spelling",
        "_C__x",
        "an `Enum`-body assignment whose name already carries the mangled-private prefix of \
         its own class is not supported yet",
    ),
    (
        "979_enum_sunder",
        "_foo_",
        "a sunder-named assignment in an `Enum` body is not supported yet",
    ),
];

/// Each shape reaches the user as `C0001` with its own message, on an
/// `int`-valued enum.
///
/// `__repr__` is the issue's own reproduction; `__x` is the shape a
/// dunder-only predicate misses (CPython's compiler mangles it to `_C__x`
/// before `_EnumDict` sees it); `_foo_` is an unrecognized sunder, for which
/// CPython raises `ValueError` while the `class` statement itself executes.
#[test]
fn each_enum_non_member_shape_is_rejected() {
    for (tag, name, needle) in NON_MEMBER {
        assert_rejected(tag, &enum_module(name, "1", "2"), needle);
    }
}

/// The same four shapes on a `str`-valued enum.
///
/// The guard runs on the name alone, before value extraction, so the value
/// kind cannot change the outcome -- which is what makes it catch the shape
/// the pre-#979 tree let through. `_order_ = 'B'` beside `B = 2` was already
/// rejected, but only as an `int`/`str` kind mismatch; beside `B = 'b'` it
/// compiled to a two-member enum where CPython has one.
#[test]
fn each_enum_non_member_shape_is_rejected_on_a_str_enum() {
    for (tag, name, needle) in NON_MEMBER {
        assert_rejected(
            &format!("{tag}_str"),
            &enum_module(name, "\"a\"", "\"b\""),
            needle,
        );
    }
}

/// `_order_` end to end: a sunder CPython *recognizes* rather than raising on,
/// and the one shape whose rejection closes a real member-count divergence
/// rather than merely refusing a program CPython runs.
#[test]
fn a_recognized_sunder_is_rejected_end_to_end() {
    assert_rejected(
        "979_enum_order_sunder",
        &enum_module("_order_", "\"B\"", "\"b\""),
        "a sunder-named assignment in an `Enum` body is not supported yet",
    );
}

/// A single leading underscore stays an ordinary member under both engines
/// (`list(C.__members__) == ['_x', 'B']` on CPython 3.13.9), and so does a
/// longer one with no trailing underscore.
#[test]
fn single_underscore_members_stay_accepted() {
    assert_accepted("979_enum_underscore_x", &enum_module("_x", "1", "2"));
    assert_accepted("979_enum_underscore_foo", &enum_module("_foo", "1", "2"));
}

/// The guard is `Enum`-route-only. In a plain class body each of these names
/// is an ordinary class attribute under both engines, so the program still
/// compiles.
#[test]
fn the_non_member_shapes_stay_accepted_in_a_plain_class() {
    for (tag, name) in [
        ("979_plain_dunder", "__repr__"),
        ("979_plain_private", "__x"),
        ("979_plain_mangled_spelling", "_C__x"),
        ("979_plain_sunder", "_foo_"),
        ("979_plain_order", "_order_"),
    ] {
        assert_accepted(
            tag,
            &format!(
                "class C:\n\
                 \x20   {name} = 8\n\
                 \n\
                 \n\
                 def main() -> int:\n\
                 \x20   c = C()\n\
                 \x20   return 0\n"
            ),
        );
    }
}

/// The class-name-keyed arm end to end: `_C__x = 1` inside `class D(Enum)` is
/// still an ordinary member and the program still compiles.
///
/// CPython 3.13.9 agrees -- `_is_private('D', '_C__x')` tests the literal
/// `_D__` prefix, so `list(D.__members__) == ['_C__x', 'B']`. Without this
/// case a shape-only predicate would pass every other test in this file while
/// introducing a fresh over-rejection.
#[test]
fn the_mangled_spelling_of_another_class_stays_accepted() {
    assert_accepted(
        "979_enum_mangled_other_class",
        "from enum import Enum\n\
         \n\
         \n\
         class D(Enum):\n\
         \x20   _C__x = 1\n\
         \x20   B = 2\n\
         \n\
         \n\
         def main() -> int:\n\
         \x20   return 0\n",
    );
}
