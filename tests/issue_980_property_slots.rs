//! Issue #980: a `@property` getter named `__slots__`, end to end through the
//! public `pycc` CLI.
//!
//! Its own file rather than an addition to
//! `tests/issue_975_reserved_dunder_class_attrs.rs`: that file's module header
//! scopes itself to [D-236]'s instantiation and class-creation protocol names
//! and states that the already-rejected `__slots__` shapes are pinned
//! unchanged there. `__slots__` on the property route is neither -- it is a
//! different name set, a different mechanism, and a behavior change.
//!
//! [D-236]: ../docs/decisions/D-236-reject-a-class-attribute-named-after-the-instantiation.md
//!
//! The defect class is D-198 false acceptance. Measured at `f8e9d2e3` against
//! CPython 3.13.9 (`v3.13.9:8183fa5e3f7`):
//!
//! ```text
//! class C:
//!     @property
//!     def __slots__(self) -> int:
//!         return 1
//! ```
//!
//! CPython raises `TypeError: 'property' object is not iterable` while the
//! `class` statement itself executes -- `type.__new__` iterates `__slots__`
//! at class creation, and a `property` object is not iterable, so the class is
//! never created. pycc accepted the program (`check` exit 0) and ran it. The
//! divergence is value-independent: a `property` is never iterable, whatever
//! the getter returns.
//!
//! Three properties are pinned here:
//!
//! 1. **Both bodies that can reach the route.** A plain and a `@dataclass`
//!    class body share one `MethodKind::PropertyGetter` arm and diverge
//!    identically (both measured). An `Enum` body cannot reach it at all --
//!    `lower_enum_class` rejects a method definition outright first -- which
//!    is why the guard takes no route parameter.
//! 2. **The right account, from both sides.** The message must describe
//!    `type.__new__` iterating `__slots__` at class creation, and must *not*
//!    borrow the plain attribute route's explanation (since #1368, "only a
//!    value-bound `__slots__` ... is"; before it, D-154's layout account), which
//!    would be a false account of this failure. Emitting that string here is
//!    exactly what #980 was opened to prevent, and a `contains`-only assertion
//!    would not catch it.
//! 3. **No over-rejection.** A benign dunder property stays accepted:
//!    `@property def __doc__(self) -> int` runs and prints `1` under both
//!    engines, so the guard must not grow into "reject every dunder property".

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

fn check(tag: &str, source: &str) -> (bool, String) {
    let dir = ScratchDir::new(tag).expect("failed to create scratch dir");
    let src = write_fixture(&dir, source);
    let out = Command::new(pycc_bin())
        .args(["check", src.to_str().unwrap()])
        .output()
        .unwrap();
    (
        out.status.success(),
        String::from_utf8_lossy(&out.stdout).to_string(),
    )
}

/// Asserts that `pycc check` rejects `source` with `code` and a diagnostic
/// that contains `needle` and does **not** contain `absent`.
///
/// The negative half is why this file does not reuse the #975 file's
/// `assert_rejected`, which can only express `contains`. Keyed on the plain
/// route's literal wording rather than a paraphrase: the property-route
/// message legitimately mentions neither an instance layout nor `__init__`, so
/// a looser needle would fail against a correct implementation.
fn assert_rejected_and_absent(tag: &str, source: &str, code: &str, needle: &str, absent: &str) {
    let (succeeded, rendered) = check(tag, source);
    assert!(
        !succeeded,
        "pycc check should reject {tag}, but it succeeded:\n{rendered}"
    );
    assert!(
        rendered.contains(code),
        "diagnostic for {tag} should carry {code}, got:\n{rendered}"
    );
    assert!(
        rendered.contains(needle),
        "diagnostic for {tag} should contain {needle:?}, got:\n{rendered}"
    );
    assert!(
        !rendered.contains(absent),
        "diagnostic for {tag} must not contain {absent:?}, got:\n{rendered}"
    );
}

/// Asserts that `pycc check` accepts `source` -- the negative half of the
/// rule, which is what stops the guard from growing past its evidence.
fn assert_accepted(tag: &str, source: &str) {
    let (succeeded, rendered) = check(tag, source);
    assert!(
        succeeded,
        "pycc check should accept {tag}, got:\n{rendered}"
    );
}

/// The distinctive fragment of the property-route `__slots__` message.
const NEEDLE: &str = "a `@property` getter named `__slots__` is not supported yet";

/// The plain attribute route's explanation (#1368; D-154's before it), never on
/// this route.
const D154_FRAGMENT: &str = "only a value-bound `__slots__`";

/// The issue's own program, minus the `main` wrapper the CLI requires.
#[test]
fn a_property_getter_named_slots_is_rejected_in_a_plain_class() {
    assert_rejected_and_absent(
        "issue980_plain_property_slots",
        "class C:\n\
         \x20   @property\n\
         \x20   def __slots__(self) -> int:\n\
         \x20       return 1\n\
         \n\
         \n\
         def main() -> int:\n\
         \x20   c = C()\n\
         \x20   return 0\n",
        "C0001",
        NEEDLE,
        D154_FRAGMENT,
    );
}

/// The `@dataclass` body reaches the same `MethodKind::PropertyGetter` arm and
/// diverges identically (measured: CPython 3.13.9 raises the same `TypeError`
/// at class creation). Pinned rather than assumed.
///
/// The fixture carries a real annotated field so the dataclass is well formed:
/// without it a rejection could come from an unrelated diagnostic and the test
/// would pass for the wrong reason. `body.rs`'s dataclass pre-check matches
/// only `__init__`, `__eq__` and `__repr__`, so `__slots__` falls through to
/// the property guard rather than to D-235's set -- which the absent-fragment
/// assertion also helps pin.
#[test]
fn a_property_getter_named_slots_is_rejected_in_a_dataclass_body() {
    assert_rejected_and_absent(
        "issue980_dataclass_property_slots",
        "from dataclasses import dataclass\n\
         \n\
         \n\
         @dataclass\n\
         class C:\n\
         \x20   x: int\n\
         \n\
         \x20   @property\n\
         \x20   def __slots__(self) -> int:\n\
         \x20       return 1\n\
         \n\
         \n\
         def main() -> int:\n\
         \x20   c = C(1)\n\
         \x20   return 0\n",
        "C0001",
        NEEDLE,
        D154_FRAGMENT,
    );
}

/// The negative half: a dunder property outside the guard's evidence stays
/// accepted. Measured on CPython 3.13.9, `@property def __doc__(self) -> int`
/// creates the class and prints `1`, exactly as pycc does, so rejecting it
/// would be a false rejection rather than a conservative one.
#[test]
fn a_benign_dunder_property_getter_stays_accepted() {
    assert_accepted(
        "issue980_benign_dunder_property",
        "class C:\n\
         \x20   @property\n\
         \x20   def __doc__(self) -> int:\n\
         \x20       return 1\n\
         \n\
         \n\
         def main() -> int:\n\
         \x20   c = C()\n\
         \x20   return 0\n",
    );
}
