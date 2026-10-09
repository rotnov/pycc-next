//! Issue #984: a non-`@property` `def __slots__` in a class body, end to end
//! through the public `pycc` CLI.
//!
//! Its own file rather than an addition to
//! `tests/issue_980_property_slots.rs`, for the same reason that file is
//! separate from `tests/issue_975_reserved_dunder_class_attrs.rs`: #980's file
//! scopes itself to the `@property` getter route and its own third `__slots__`
//! message, while this is a different set of spellings, a fourth message, a
//! second call site, and a behavior change of its own.
//!
//! The defect class is [D-198] false acceptance. Measured at `e77b4b13`
//! against CPython 3.13.9 (`v3.13.9:8183fa5e3f7`), seven spellings diverged:
//!
//! | shape | CPython 3.13.9 | `pycc check` (before) |
//! |---|---|---|
//! | `def __slots__(self)` | `TypeError: 'function' object is not iterable` | accepted |
//! | `@staticmethod def __slots__()` | `TypeError: 'staticmethod' object is not iterable` | accepted |
//! | `@classmethod def __slots__(cls)` | `TypeError: 'classmethod' object is not iterable` | accepted |
//! | `@abstractmethod def __slots__` on an `ABC` | `TypeError: 'function' object is not iterable` | accepted |
//! | `@override def __slots__` | `TypeError: 'function' object is not iterable` | accepted |
//! | `@dataclass` body + `def __slots__` | `TypeError: 'function' object is not iterable` | accepted |
//! | `Protocol` body + `def __slots__` | `TypeError: 'function' object is not iterable` | accepted |
//!
//! In every case CPython exits 1 without creating the class, while `pycc run`
//! printed `1` and exited 0. `type.__new__` iterates `__slots__` while the
//! `class` statement itself executes, and none of those carriers is iterable,
//! so the divergence is value-independent -- whatever the method returns.
//!
//! [D-198]: ../docs/decisions/D-198-treat-a-cpython-divergence-as-a-compiler-defect.md
//!
//! Four properties are pinned here:
//!
//! 1. **Each carrier, named correctly.** CPython names only three types across
//!    all seven spellings, so the message interpolates a carrier resolved from
//!    the binding form rather than carrying one string per spelling.
//! 2. **The right account, from both sides.** The message must describe
//!    `type.__new__` iterating `__slots__` at class creation, and must *not*
//!    borrow the plain attribute route's explanation (since #1368, "only a
//!    value-bound `__slots__` ... is"; before it, D-154's layout account), which
//!    would be a false account of a class that is never created. A
//!    `contains`-only assertion would not catch that substitution.
//! 3. **Both class-body walks.** A `Protocol` body never reaches
//!    `walk_class_body` -- `lower_class` returns through
//!    `lower_protocol_class` first -- so it needs its own call site, and a fix
//!    that covered only the method loop would leave it falsely accepted.
//! 4. **No over-rejection.** A benign dunder method stays accepted: `def
//!    __doc__(self) -> int` runs and prints `1` under both engines, so the
//!    guard must not grow into "reject every dunder method".
//!
//! `@override def __slots__` is deliberately not tested here: `@override`
//! requires a base class declaring the same name, and that base's own
//! `def __slots__` is rejected first, at the base's line, so any such test
//! would pass for the wrong reason. It binds a plain `function` and folds into
//! the same carrier as the bare `def` in any case.

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
/// that contains `needle` and does **not** contain `absent`, mirroring
/// `tests/issue_980_property_slots.rs`'s helper of the same name. The negative
/// half is the point: `contains` alone cannot express "and not the other
/// route's account".
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

/// The opening of the method-route `__slots__` message, shared by every
/// spelling below.
const NEEDLE: &str = "a `def __slots__` in a class body is not supported yet";

/// The plain attribute route's explanation (#1368; D-154's before it), never on
/// this route.
const D154_FRAGMENT: &str = "only a value-bound `__slots__`";

/// Asserts the whole message for one spelling: the shared opening, the
/// carrier CPython itself names, and the absence of D-154's account.
fn assert_slots_method_rejected(tag: &str, source: &str, carrier: &str) {
    assert_rejected_and_absent(tag, source, "C0001", NEEDLE, D154_FRAGMENT);
    let (_, rendered) = check(tag, source);
    let carrier_needle = format!("a `{carrier}` object is not iterable");
    assert!(
        rendered.contains(&carrier_needle),
        "diagnostic for {tag} should name the carrier {carrier:?}, got:\n{rendered}"
    );
}

/// The issue's own program, minus the `main` wrapper the CLI requires.
#[test]
fn a_plain_method_named_slots_is_rejected() {
    assert_slots_method_rejected(
        "issue984_plain_slots_method",
        "class C:\n\
         \x20   def __slots__(self) -> int:\n\
         \x20       return 1\n\
         \n\
         \n\
         def main() -> int:\n\
         \x20   c = C()\n\
         \x20   return 0\n",
        "function",
    );
}

/// `@staticmethod` binds a `staticmethod` object, and CPython's `TypeError`
/// names it, so the diagnostic does too.
#[test]
fn a_static_method_named_slots_is_rejected() {
    assert_slots_method_rejected(
        "issue984_static_slots_method",
        "class C:\n\
         \x20   @staticmethod\n\
         \x20   def __slots__() -> int:\n\
         \x20       return 1\n\
         \n\
         \n\
         def main() -> int:\n\
         \x20   c = C()\n\
         \x20   return 0\n",
        "staticmethod",
    );
}

/// `@classmethod` binds a `classmethod` object -- the third and last carrier
/// CPython names for this shape.
#[test]
fn a_class_method_named_slots_is_rejected() {
    assert_slots_method_rejected(
        "issue984_class_slots_method",
        "class C:\n\
         \x20   @classmethod\n\
         \x20   def __slots__(cls) -> int:\n\
         \x20       return 1\n\
         \n\
         \n\
         def main() -> int:\n\
         \x20   c = C()\n\
         \x20   return 0\n",
        "classmethod",
    );
}

/// A `@dataclass` body reaches the same method loop and diverges identically.
/// The fixture carries a real annotated field so the dataclass is well formed:
/// without it a rejection could come from an unrelated diagnostic and the test
/// would pass for the wrong reason.
#[test]
fn a_dataclass_method_named_slots_is_rejected() {
    assert_slots_method_rejected(
        "issue984_dataclass_slots_method",
        "from dataclasses import dataclass\n\
         \n\
         \n\
         @dataclass\n\
         class C:\n\
         \x20   x: int\n\
         \n\
         \x20   def __slots__(self) -> int:\n\
         \x20       return 1\n\
         \n\
         \n\
         def main() -> int:\n\
         \x20   c = C(1)\n\
         \x20   return 0\n",
        "function",
    );
}

/// A `Protocol` body is the second call site, not the same one. This is the
/// test that fails if the guard is added only to `walk_class_body`.
#[test]
fn a_protocol_method_named_slots_is_rejected() {
    assert_slots_method_rejected(
        "issue984_protocol_slots_method",
        "from typing import Protocol\n\
         \n\
         \n\
         class P(Protocol):\n\
         \x20   def __slots__(self) -> int:\n\
         \x20       ...\n\
         \n\
         \n\
         def main() -> int:\n\
         \x20   return 0\n",
        "function",
    );
}

/// The negative half: a dunder method outside the guard's evidence stays
/// accepted. Measured on CPython 3.13.9, `def __doc__(self) -> int` creates
/// the class and prints `1`, exactly as pycc does, so rejecting it would be a
/// false rejection rather than a conservative one.
#[test]
fn a_benign_dunder_method_stays_accepted() {
    assert_accepted(
        "issue984_benign_dunder_method",
        "class C:\n\
         \x20   def __doc__(self) -> int:\n\
         \x20       return 1\n\
         \n\
         \n\
         def main() -> int:\n\
         \x20   c = C()\n\
         \x20   return 0\n",
    );
}
