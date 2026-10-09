//! #953 (PEP 544): calling a protocol member that takes a protocol-typed
//! parameter with a concrete conforming argument.
//!
//! `p.same(C())` used to be rejected with a spurious `T0021` even though
//! `C` conforms to `P` and `p: P = C()` was itself accepted:
//! `pycc_types::class::check_call_args` compared arguments with plain
//! nominal assignability, while `expr.rs`' own plain-function-call path
//! already used the environment-aware `is_assignable_env`.
//!
//! Fixing only that would have replaced the spurious rejection with a
//! compiler abort. `monomorphize_protocol_params` drops every module item
//! whose parameters carry a `Ty::Protocol` -- which includes the mangled
//! methods `pycc_hir::class::lower_class` writes into `HirModule::items`
//! -- and used to create specializations only for `HirExpr::Call`, never
//! for `HirExpr::MethodCall`. The accepted call then reached `pycc_mir`
//! with no `$fn:<mangled>` binding and panicked. Both seams therefore move
//! together, and these tests exist to prove the accepted programs actually
//! *run*, not merely type-check.
//!
//! The in-crate unit tests (`pycc_types::tests::protocol_argument`,
//! `pycc_mir::tests::protocol`) own the branch-level coverage; these own
//! the end-to-end behavior of the shipped binary.

use pycc_scratch::ScratchDir;
use std::path::{Path, PathBuf};
use std::process::Command;

fn pycc_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_pycc"))
}

fn write(dir: &ScratchDir, relative: &str, contents: &str) -> PathBuf {
    let path = dir.join(relative);
    std::fs::write(&path, contents).unwrap();
    path
}

/// The issue's own protocol and conforming class: the member takes the
/// protocol itself, which is #948's accepted self-referential shape.
const PRELUDE: &str = "from typing import Protocol\n\nclass P(Protocol):\n    def val(self) -> int: ...\n\n    def same(self, other: P) -> int: ...\n\nclass C:\n    def val(self) -> int:\n        return 4\n\n    def same(self, other: P) -> int:\n        return other.val() + 1\n\n";

/// Runs `pycc check` on `entry` and asserts it accepts the program.
fn assert_checks(entry: &Path) {
    let result = Command::new(pycc_bin())
        .arg("check")
        .arg(entry.to_str().unwrap())
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&result.stdout);
    let stderr = String::from_utf8_lossy(&result.stderr);
    assert!(
        result.status.success(),
        "pycc check should accept the program, got {:?}\nstdout: {stdout}\nstderr: {stderr}",
        result.status
    );
}

/// Runs `pycc run` on `entry` and asserts the program compiles and prints
/// exactly `expected_stdout`. A `pycc_mir`/`pycc_codegen` abort exits 101,
/// so the success assertion is itself the regression guard for the panics
/// this issue's fix removes.
fn assert_runs_and_prints(entry: &Path, expected_stdout: &str) {
    let result = Command::new(pycc_bin())
        .arg("run")
        .arg(entry.to_str().unwrap())
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&result.stdout);
    let stderr = String::from_utf8_lossy(&result.stderr);
    assert!(
        result.status.success(),
        "pycc run should succeed, got {:?}\nstdout: {stdout}\nstderr: {stderr}",
        result.status
    );
    assert_eq!(stdout, expected_stdout, "stderr: {stderr}");
    for forbidden in ["panicked", "internal error", "pycc_rt:"] {
        assert!(
            !stderr.contains(forbidden),
            "the program must run cleanly; found {forbidden:?} in: {stderr}"
        );
    }
}

#[test]
fn the_issue_program_checks_and_runs() {
    let dir = ScratchDir::new("953_repro").expect("failed to create scratch dir");
    let entry = write(
        &dir,
        "main.py",
        &format!(
            "{PRELUDE}def main() -> None:\n    p: P = C()\n    print(p.same(C()))\n\nmain()\n"
        ),
    );
    assert_checks(&entry);
    assert_runs_and_prints(&entry, "5\n");
}

#[test]
fn a_concrete_receiver_checks_and_runs() {
    let dir = ScratchDir::new("953_concrete").expect("failed to create scratch dir");
    let entry = write(
        &dir,
        "main.py",
        &format!("{PRELUDE}def main() -> None:\n    c = C()\n    print(c.same(C()))\n\nmain()\n"),
    );
    assert_checks(&entry);
    assert_runs_and_prints(&entry, "5\n");
}

#[test]
fn a_module_level_receiver_checks_and_runs() {
    let dir = ScratchDir::new("953_module").expect("failed to create scratch dir");
    let entry = write(
        &dir,
        "main.py",
        &format!("{PRELUDE}p: P = C()\nprint(p.same(C()))\n"),
    );
    assert_checks(&entry);
    assert_runs_and_prints(&entry, "5\n");
}

#[test]
fn a_protocol_typed_argument_through_protocol_typed_parameters_runs() {
    // `def g(p: P, q: P)` already passed `check` before this fix -- two
    // equal `Ty::Protocol`s satisfy plain assignability -- and panicked in
    // `pycc_mir` with "`$fn:C.same` has no recorded type". `g` is itself
    // specialized per call site, so `p` and `q` are concrete by the time
    // the method call is rewritten.
    let dir = ScratchDir::new("953_proto_param").expect("failed to create scratch dir");
    let entry = write(
        &dir,
        "main.py",
        &format!(
            "{PRELUDE}def g(p: P, q: P) -> int:\n    return p.same(q)\n\ndef main() -> None:\n    print(g(C(), C()))\n\nmain()\n"
        ),
    );
    assert_checks(&entry);
    assert_runs_and_prints(&entry, "5\n");
}

#[test]
fn an_inherited_protocol_parameter_method_runs() {
    let dir = ScratchDir::new("953_inherited").expect("failed to create scratch dir");
    let entry = write(
        &dir,
        "main.py",
        "from typing import Protocol\n\nclass P(Protocol):\n    def val(self) -> int: ...\n\nclass Base:\n    def take(self, p: P) -> int:\n        return p.val() + 1\n\nclass C(Base):\n    def val(self) -> int:\n        return 7\n\ndef main() -> None:\n    c = C()\n    print(c.take(C()))\n\nmain()\n",
    );
    assert_checks(&entry);
    assert_runs_and_prints(&entry, "8\n");
}

#[test]
fn a_non_conforming_argument_is_still_t0021() {
    let dir = ScratchDir::new("953_negative").expect("failed to create scratch dir");
    let entry = write(
        &dir,
        "main.py",
        &format!(
            "{PRELUDE}class D:\n    def other(self) -> int:\n        return 1\n\ndef main() -> None:\n    c = C()\n    print(c.same(D()))\n\nmain()\n"
        ),
    );
    let result = Command::new(pycc_bin())
        .arg("check")
        .arg(entry.to_str().unwrap())
        .output()
        .unwrap();
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(
        result.status.code(),
        Some(1),
        "a non-conforming argument must still be rejected, got: {combined}"
    );
    assert!(combined.contains("error[T0021]"), "got: {combined}");
    assert!(
        combined.contains("argument 1 of `same` expects `P`, got `D`"),
        "got: {combined}"
    );
    for forbidden in ["panicked", "internal error", "pycc_rt:"] {
        assert!(
            !combined.contains(forbidden),
            "the compiler must diagnose, not abort; found {forbidden:?} in: {combined}"
        );
    }
}
