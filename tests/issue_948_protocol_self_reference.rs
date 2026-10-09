//! #948 (PEP 544): a protocol member whose annotation names its own protocol.
//!
//! `class P(Protocol): def clone(self) -> P: ...` used to lower to
//! `Ty::Instance("P")` rather than `Ty::Protocol("P")`, because
//! `pycc_hir::func::annotation_to_ty`'s self-reference arms ran before the
//! `class_defs` lookup that resolves a protocol name. Every concrete class
//! implementing `clone` was then rejected with a spurious `T0046` (return type
//! mismatch) rendered at the module's first line, even when its `clone`
//! returned a conforming instance. The PEP 673 spelling (`-> Self`) had the
//! same defect and compiled silently when no conforming class was present.
//!
//! Both spellings now resolve through `func::enclosing_class_ty` to
//! `Ty::Protocol("P")` and reach #934's existing `C0001` protocol-return gate,
//! on the annotation's own span and with #934's own message -- the issue's
//! second stated outcome ("the self-reference is rejected with a real `C0001`
//! naming the construct"). The first ("structural conformance accepts `-> C`")
//! is not reachable at this scale: `pycc_types::class::
//! check_protocol_conformance` compares member signatures with plain
//! `is_assignable` and does no structural matching, so even a cross-class
//! `other: D` against a member `other: Q` is `T0046` today.
//!
//! The *parameter* position is not gated -- `Ty::Protocol` is supported there
//! (D-166) -- so a protocol member declared `def same(self, other: P) -> bool`
//! and a conforming class spelling the parameter the same way now pass the
//! conformance check and the program compiles and runs. Before this fix it
//! failed with the self-contradictory `T0046: ... parameter 1 has type `P`,
//! expected `P``, one side `Ty::Protocol("P")` and the other
//! `Ty::Instance("P")`. What that did *not* buy at the time was calling the
//! member through a protocol-typed receiver: `p.same(C())` was a separate,
//! pre-existing `T0021`, since `pycc_types`' `check_call_args` used plain
//! assignability rather than the environment-aware conformance path. That
//! limitation was lifted afterwards by
//! [#953](https://github.com/rotnov/pycc/issues/953) and is covered by
//! `tests/issue_953_protocol_argument.rs`; the tests below stay as written,
//! dispatching through a *second*, non-self-referential member, so they keep
//! pinning this issue's own conformance fix independently of that one.
//!
//! Unit tests beside the gate (`pycc_hir::class::protocol_return_tests`) pin
//! the lowered `Ty` for every member position; these prove the binary reports
//! the diagnostic (exit 1, never the 101 a panic exits with) and that the
//! accepted shape runs.

use pycc_scratch::ScratchDir;
use std::path::{Path, PathBuf};
use std::process::Command;

fn pycc_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_pycc"))
}

fn write(root: &Path, relative: &str, contents: &str) -> PathBuf {
    let path = root.join(relative);
    std::fs::write(&path, contents).unwrap();
    path
}

const MESSAGE: &str = "a protocol class (`P`) as a return type annotation is not supported yet -- a protocol type is currently supported in parameter and variable positions only";

/// The issue's own program, with the protocol member's return annotation
/// spelled `annotation` (`P` or `Self`).
fn self_return_program(annotation: &str) -> String {
    format!(
        "from typing import Protocol\n\nclass P(Protocol):\n    def clone(self) -> {annotation}: ...\n\nclass C:\n    def __init__(self) -> None:\n        self.x = 0\n\n    def clone(self) -> C:\n        return C()\n\ndef main() -> None:\n    c: P = C()\n    print(c.x)\n\nmain()\n"
    )
}

/// Runs `pycc <subcommand>` on `entry`, asserting it exits 1 with the
/// protocol-return `C0001` rendered at `line:column` on the source line
/// `header`, never panics, and leaves no binary behind. `pycc check` renders
/// on stdout and `pycc build` on stderr, so both streams are searched.
fn assert_self_return_rejected(
    dir: &ScratchDir,
    subcommand: &str,
    entry: &Path,
    column: u32,
    header: &str,
) {
    let out = dir.join("prog");
    let mut cmd = Command::new(pycc_bin());
    cmd.arg(subcommand).arg(entry.to_str().unwrap());
    if subcommand == "build" {
        cmd.args(["-o", out.to_str().unwrap()]);
    }
    let result = cmd.output().unwrap();
    let stderr = String::from_utf8_lossy(&result.stderr);
    let stdout = String::from_utf8_lossy(&result.stdout);
    let combined = format!("{stdout}{stderr}");
    assert_eq!(
        result.status.code(),
        Some(1),
        "pycc {subcommand} should fail with a diagnostic, got {:?}\nstdout: {stdout}\nstderr: {stderr}",
        result.status
    );
    assert!(
        combined.contains("error[C0001]"),
        "the output should carry a C0001 diagnostic, got: {combined}"
    );
    assert!(
        combined.contains(MESSAGE),
        "the output should carry {MESSAGE:?}, got: {combined}"
    );
    assert!(
        !combined.contains("T0046"),
        "the spurious conformance error must be gone, got: {combined}"
    );
    // The renderer prints paths with forward slashes on every platform, so
    // normalize the expected location the same way.
    let location = format!("{}:4:{column}", entry.to_string_lossy().replace('\\', "/"));
    assert!(
        combined.contains(&location),
        "the diagnostic should point at the return annotation {location}, got: {combined}"
    );
    assert!(
        combined.contains(&format!("4 | {header}")),
        "the rendered source line should be the member declaration {header:?}, got: {combined}"
    );
    for forbidden in ["panicked", "internal error", "pycc_rt:"] {
        assert!(
            !combined.contains(forbidden),
            "the compiler must diagnose, not abort; found {forbidden:?} in: {combined}"
        );
    }
    assert!(!out.exists(), "a failing build must not leave a binary");
}

/// The issue's own reproduction: `T0046` at `1:1` blaming `C.clone` before
/// this fix, a `C0001` on the annotation now.
#[test]
fn the_issue_program_is_c0001_on_the_member_annotation() {
    let dir = ScratchDir::new("948_check").expect("failed to create scratch dir");
    let entry = write(&dir, "prog.py", &self_return_program("P"));
    assert_self_return_rejected(&dir, "check", &entry, 24, "    def clone(self) -> P: ...");
}

/// `pycc build` stops at HIR lowering too, so no artifact is produced.
#[test]
fn building_the_issue_program_is_c0001_and_leaves_no_binary() {
    let dir = ScratchDir::new("948_build").expect("failed to create scratch dir");
    let entry = write(&dir, "prog.py", &self_return_program("P"));
    assert_self_return_rejected(&dir, "build", &entry, 24, "    def clone(self) -> P: ...");
}

/// The PEP 673 spelling reaches the same gate. `from typing import Self` is
/// itself `C0002` in this version, so the bare name is the only reachable
/// spelling -- and before this fix it lowered to `Ty::Instance("P")` exactly
/// as the bare protocol name did.
#[test]
fn the_self_spelling_is_c0001_on_the_member_annotation() {
    let dir = ScratchDir::new("948_self").expect("failed to create scratch dir");
    let entry = write(&dir, "prog.py", &self_return_program("Self"));
    assert_self_return_rejected(
        &dir,
        "check",
        &entry,
        24,
        "    def clone(self) -> Self: ...",
    );
}

/// ... and stops `pycc build` at the same seam, leaving no binary, exactly
/// as the bare-name spelling does.
#[test]
fn building_the_self_spelling_is_c0001_and_leaves_no_binary() {
    let dir = ScratchDir::new("948_self_build").expect("failed to create scratch dir");
    let entry = write(&dir, "prog.py", &self_return_program("Self"));
    assert_self_return_rejected(
        &dir,
        "build",
        &entry,
        24,
        "    def clone(self) -> Self: ...",
    );
}

/// A protocol whose member declares a *self-referential parameter*, plus a
/// concrete class spelling that parameter the same way, conforms and runs:
/// the parameter position supports `Ty::Protocol` (D-166), and dispatch goes
/// through the conforming class's own method table. Before this fix the two
/// sides were `Ty::Protocol("P")` and `Ty::Instance("P")` and conformance
/// failed with `T0046: ... parameter 1 has type `P`, expected `P``.
///
/// The executed call is `p.value()`, not `p.same(...)`: what this pins is
/// that the self-referential member no longer blocks conformance, not that
/// such a member is callable. Calling it with a concrete argument was a
/// separate, pre-existing `T0021` when this test was written and is now
/// accepted -- see the module doc comment and
/// `tests/issue_953_protocol_argument.rs`.
#[test]
fn a_self_referential_protocol_parameter_conforms_and_runs() {
    let dir = ScratchDir::new("948_param").expect("failed to create scratch dir");
    let entry = write(
        &dir,
        "prog.py",
        "from typing import Protocol\n\nclass P(Protocol):\n    def same(self, other: P) -> bool: ...\n    def value(self) -> int: ...\n\nclass C:\n    def __init__(self) -> None:\n        self.x = 7\n\n    def same(self, other: P) -> bool:\n        return True\n\n    def value(self) -> int:\n        return self.x\n\ndef use(p: P) -> int:\n    return p.value()\n\nprint(use(C()))\n",
    );
    let result = Command::new(pycc_bin())
        .arg("run")
        .arg(entry.to_str().unwrap())
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&result.stderr);
    let stdout = String::from_utf8_lossy(&result.stdout);
    assert!(
        result.status.success(),
        "pycc run should succeed, got {:?}\nstdout: {stdout}\nstderr: {stderr}",
        result.status
    );
    assert_eq!(stdout, "7\n", "stderr: {stderr}");
}
