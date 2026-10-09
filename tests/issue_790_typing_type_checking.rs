// Issue #790: `typing.TYPE_CHECKING` is CPython's standard idiom for
// guarding imports and statements meant only for static type checkers --
// `TYPE_CHECKING` is always `False` at runtime, so a guarded `if
// TYPE_CHECKING: ...` body never executes. `pycc_hir` constant-folds the
// guard away before either lowering or type-checking ever sees its body
// (`pycc_hir::stmt::is_type_checking_guard`), so the body may freely
// contain constructs pycc doesn't support elsewhere.
//
// These tests exercise the fix end-to-end through the public CLI: `pycc
// check`/`pycc build` accept the import and the guard, the guarded body is
// never checked (proven by a body that would otherwise fail to compile),
// and the live `else` branch is checked and executed normally.

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

/// #790: before this change, `from typing import TYPE_CHECKING` alone
/// failed with `C0002` ("module `typing` has no importable symbol named
/// `TYPE_CHECKING`"). Now the import resolves and the guarded body -- an
/// `import` of a module that does not exist, which would fail with `C0001`
/// if it were ever lowered -- is skipped entirely, so `pycc check` succeeds.
#[test]
fn bare_type_checking_guard_with_an_unsupported_body_checks_successfully() {
    let dir = ScratchDir::new("790_bare").expect("failed to create scratch dir");
    let src = write_fixture(
        &dir,
        "bare.py",
        "from typing import TYPE_CHECKING\n\nif TYPE_CHECKING:\n    import some_module_that_does_not_exist_at_runtime_or_compile_time\n\nprint(\"ok\")\n",
    );
    let output = Command::new(pycc_bin())
        .args(["check", src.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "pycc check should succeed for a bare `if TYPE_CHECKING:` guard with an \
         otherwise-unsupported body; stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

/// #790: the qualified `import typing; if typing.TYPE_CHECKING:` spelling
/// gets the identical fold as the bare-name form above.
#[test]
fn qualified_type_checking_guard_with_an_unsupported_body_checks_successfully() {
    let dir = ScratchDir::new("790_qualified").expect("failed to create scratch dir");
    let src = write_fixture(
        &dir,
        "qualified.py",
        "import typing\n\nif typing.TYPE_CHECKING:\n    import some_module_that_does_not_exist_at_runtime_or_compile_time\n\nprint(\"ok\")\n",
    );
    let output = Command::new(pycc_bin())
        .args(["check", src.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "pycc check should succeed for a qualified `typing.TYPE_CHECKING` guard with an \
         otherwise-unsupported body; stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

/// #790: the `TYPE_CHECKING` guard's body never executes, but its `else`
/// branch is live code checked and run exactly like any other `if`/`else`.
/// Builds and runs the program end-to-end, proving the observable behavior
/// matches CPython's own `TYPE_CHECKING == False` semantics: only "else"
/// prints.
#[test]
fn the_else_branch_of_a_type_checking_guard_builds_and_runs() {
    let dir = ScratchDir::new("790_else").expect("failed to create scratch dir");
    let src = write_fixture(
        &dir,
        "with_else.py",
        "from typing import TYPE_CHECKING\n\nif TYPE_CHECKING:\n    import some_module_that_does_not_exist_at_runtime_or_compile_time\nelse:\n    print(\"else\")\n",
    );
    let exe = dir.join("with_else");
    let build = Command::new(pycc_bin())
        .args(["build", src.to_str().unwrap(), "-o", exe.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(
        build.status.success(),
        "pycc build should succeed; stderr: {}",
        String::from_utf8_lossy(&build.stderr)
    );
    let run = Command::new(&exe).output().unwrap();
    assert!(
        run.status.success(),
        "the built binary should exit successfully; stderr: {}",
        String::from_utf8_lossy(&run.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&run.stdout),
        "else\n",
        "only the live `else` branch should ever execute"
    );
}

/// #790: `elif TYPE_CHECKING:` gets the same constant-fold as a leading `if
/// TYPE_CHECKING:` -- the first (false, non-`TYPE_CHECKING`) branch is
/// live and skipped at runtime as usual, the `elif TYPE_CHECKING:` branch's
/// unsupported body is never checked, and the final `else` runs.
#[test]
fn elif_type_checking_guard_with_an_unsupported_body_builds_and_runs() {
    let dir = ScratchDir::new("790_elif").expect("failed to create scratch dir");
    let src = write_fixture(
        &dir,
        "elif.py",
        "from typing import TYPE_CHECKING\n\nif False:\n    print(\"first\")\nelif TYPE_CHECKING:\n    import some_module_that_does_not_exist_at_runtime_or_compile_time\nelse:\n    print(\"else\")\n",
    );
    let exe = dir.join("elif");
    let build = Command::new(pycc_bin())
        .args(["build", src.to_str().unwrap(), "-o", exe.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(
        build.status.success(),
        "pycc build should succeed; stderr: {}",
        String::from_utf8_lossy(&build.stderr)
    );
    let run = Command::new(&exe).output().unwrap();
    assert!(
        run.status.success(),
        "the built binary should exit successfully; stderr: {}",
        String::from_utf8_lossy(&run.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&run.stdout),
        "else\n",
        "only the final live `else` branch should ever execute"
    );
}

/// #791 D-068 review finding: every other test in this file exercises the
/// guard either through `pycc check` alone or with a live `else`/`elif`
/// clause -- none proves a *bare* `if TYPE_CHECKING:` with no `else` at all
/// builds and runs to completion. Confirms the constant-folded `HirStmt::If`
/// (empty `body`, empty `orelse`) reaches codegen and executes as a
/// genuine no-op, not just that `check` accepts it.
#[test]
fn a_bare_type_checking_guard_with_no_else_builds_and_runs() {
    let dir = ScratchDir::new("790_bare_no_else").expect("failed to create scratch dir");
    let src = write_fixture(
        &dir,
        "bare_no_else.py",
        "from typing import TYPE_CHECKING\n\nif TYPE_CHECKING:\n    import some_module_that_does_not_exist_at_runtime_or_compile_time\n\nprint(\"ran\")\n",
    );
    let exe = dir.join("bare_no_else");
    let build = Command::new(pycc_bin())
        .args(["build", src.to_str().unwrap(), "-o", exe.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(
        build.status.success(),
        "pycc build should succeed for a bare `if TYPE_CHECKING:` with no `else`; stderr: {}",
        String::from_utf8_lossy(&build.stderr)
    );
    let run = Command::new(&exe).output().unwrap();
    assert!(
        run.status.success(),
        "the built binary should exit successfully; stderr: {}",
        String::from_utf8_lossy(&run.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&run.stdout),
        "ran\n",
        "the folded guard's empty body/orelse must be a genuine no-op, and the \
         statement after the guard must still execute"
    );
}

/// #790: `TYPE_CHECKING` referenced as a first-class value (not the test of
/// an `if`/`elif`) is rejected by the type checker -- it is a compile-time
/// marker for exactly one purpose, not a general-purpose boolean.
#[test]
fn type_checking_used_as_a_value_is_rejected_before_build() {
    let dir = ScratchDir::new("790_value").expect("failed to create scratch dir");
    let src = write_fixture(
        &dir,
        "as_value.py",
        "import typing\n\nx = typing.TYPE_CHECKING\nprint(x)\n",
    );
    let check = Command::new(pycc_bin())
        .args(["check", src.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(
        !check.status.success(),
        "pycc check should reject `typing.TYPE_CHECKING` used as a value"
    );
    let stdout = String::from_utf8_lossy(&check.stdout);
    assert!(
        stdout.contains("T0021") && stdout.contains("compile-time marker"),
        "expected a T0021 compile-time-marker diagnostic, got: {stdout}"
    );
}
