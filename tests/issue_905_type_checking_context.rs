// Issue #905: #790's `if TYPE_CHECKING:` constant-fold hides the guarded
// body from `lower_body`, and with it every *context* check `lower_stmt`
// performs. CPython rejects a `return` in a `finally`, a `break` with no
// enclosing loop, a `yield` outside a function and an `async for` outside an
// `async def` at compile time whether or not the branch ever runs, so a
// program that hid one of them behind the guard used to compile silently.
//
// `pycc_hir::stmt::type_checking` re-walks the guarded body for exactly
// those `L0001` violations, and for nothing else: a body containing
// constructs pycc does not implement must still be accepted, which is the
// whole point of #790. These tests exercise both halves end-to-end through
// the public CLI.

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

/// Runs `pycc check` on `source` and asserts it fails with an `L0001`
/// carrying `expected_message`. The rendered *path* is deliberately not
/// asserted (the renderer prints forward slashes on every platform, so a
/// path assertion would be a Windows-only trap); the code and the message
/// are what this issue is about.
fn assert_check_reports_l0001(scratch: &str, name: &str, source: &str, expected_message: &str) {
    let dir = ScratchDir::new(scratch).expect("failed to create scratch dir");
    let src = write_fixture(&dir, name, source);
    let output = Command::new(pycc_bin())
        .args(["check", src.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(
        !output.status.success(),
        "pycc check should reject a context violation hidden behind a `TYPE_CHECKING` guard"
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("error[L0001]") && stdout.contains(expected_message),
        "expected an L0001 mentioning {expected_message:?}; got:\n{stdout}"
    );
}

/// Runs `pycc check` on `source` and asserts it succeeds -- the #790
/// contract the walker must not regress.
fn assert_check_succeeds(scratch: &str, name: &str, source: &str) {
    let dir = ScratchDir::new(scratch).expect("failed to create scratch dir");
    let src = write_fixture(&dir, name, source);
    let output = Command::new(pycc_bin())
        .args(["check", src.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "pycc check should still accept a `TYPE_CHECKING` guard whose body only contains \
         constructs pycc does not implement; stdout:\n{}",
        String::from_utf8_lossy(&output.stdout)
    );
}

/// The issue's own first reproducer: a `return` inside a `finally`, hidden
/// behind the guard (PEP 765 / D-193).
#[test]
fn a_guarded_return_in_a_finally_block_is_rejected() {
    assert_check_reports_l0001(
        "905_finally_return",
        "finally_return.py",
        "from typing import TYPE_CHECKING\n\n\ndef f() -> int:\n    try:\n        pass\n    finally:\n        if TYPE_CHECKING:\n            return 1\n    return 0\n\n\ndef main() -> None:\n    print(f())\n",
        "'return' in a 'finally' block",
    );
}

/// The issue's own second reproducer: a `break` with no enclosing loop
/// (D-148).
#[test]
fn a_guarded_break_outside_a_loop_is_rejected() {
    assert_check_reports_l0001(
        "905_break",
        "break_outside_loop.py",
        "from typing import TYPE_CHECKING\n\n\ndef f() -> int:\n    if TYPE_CHECKING:\n        break\n    return 0\n\n\ndef main() -> None:\n    print(f())\n",
        "'break' outside loop",
    );
}

/// A guarded `continue` with no enclosing loop, through the qualified
/// `typing.TYPE_CHECKING` spelling of the guard.
#[test]
fn a_guarded_continue_outside_a_loop_is_rejected_through_the_qualified_guard() {
    assert_check_reports_l0001(
        "905_continue",
        "continue_outside_loop.py",
        "import typing\n\n\ndef main() -> None:\n    if typing.TYPE_CHECKING:\n        continue\n    print(0)\n",
        "'continue' not properly in loop",
    );
}

/// #795 (PEP 654): a `return` inside an `except*` clause body stays fatal
/// when the guard is what encloses it.
#[test]
fn a_guarded_return_in_an_except_star_block_is_rejected() {
    assert_check_reports_l0001(
        "905_except_star",
        "except_star_return.py",
        "from typing import TYPE_CHECKING\n\n\ndef f() -> int:\n    try:\n        pass\n    except* ValueError:\n        if TYPE_CHECKING:\n            return 1\n    return 0\n\n\ndef main() -> None:\n    print(f())\n",
        "'return' in an 'except*' block",
    );
}

/// A `yield` at module scope, hidden behind the guard. The walker does not
/// restate `expr.rs`'s rule -- it hands the expression statement to the real
/// `lower_expr` and forwards only its `L0001`.
#[test]
fn a_guarded_yield_outside_a_function_is_rejected() {
    assert_check_reports_l0001(
        "905_yield",
        "yield_outside_function.py",
        "from typing import TYPE_CHECKING\n\nif TYPE_CHECKING:\n    yield 1\n\nprint(\"ok\")\n",
        "'yield' outside function",
    );
}

/// An `async for` under the guard, with no `async def` anywhere (D-148).
#[test]
fn a_guarded_async_for_is_rejected() {
    assert_check_reports_l0001(
        "905_async_for",
        "async_for.py",
        "from typing import TYPE_CHECKING\n\nif TYPE_CHECKING:\n    async for x in xs:\n        print(x)\n\nprint(\"ok\")\n",
        "'async for' outside async function",
    );
}

/// The second fold site: `elif TYPE_CHECKING:` gets the identical re-check.
#[test]
fn an_elif_type_checking_guard_is_checked_too() {
    assert_check_reports_l0001(
        "905_elif",
        "elif_guard.py",
        "from typing import TYPE_CHECKING\n\nif False:\n    print(\"first\")\nelif TYPE_CHECKING:\n    break\nelse:\n    print(\"else\")\n",
        "'break' outside loop",
    );
}

/// A violation nested several statements deep under the guard is still
/// found -- the walk is total, not a scan of the body's top level.
#[test]
fn a_deeply_nested_guarded_violation_is_rejected() {
    assert_check_reports_l0001(
        "905_nested",
        "nested.py",
        "from typing import TYPE_CHECKING\n\n\ndef f() -> int:\n    if TYPE_CHECKING:\n        while True:\n            try:\n                pass\n            finally:\n                return 1\n    return 0\n\n\ndef main() -> None:\n    print(f())\n",
        "'return' in a 'finally' block",
    );
}

/// The #790 contract, restated end-to-end: a guarded body made only of
/// constructs pycc does not implement must still check cleanly. This is the
/// regression the walker most plausibly breaks.
#[test]
fn a_guarded_unsupported_body_still_checks_successfully() {
    assert_check_succeeds(
        "905_unsupported_body",
        "unsupported_body.py",
        "from typing import TYPE_CHECKING\n\nif TYPE_CHECKING:\n    import some_module_that_does_not_exist_at_runtime_or_compile_time\n\n    f = lambda: 1\n\n    for a, b in pairs:\n        break\n\nprint(\"ok\")\n",
    );
}

/// A legal guarded body still folds away, builds, and runs as a no-op.
#[test]
fn a_legal_guarded_body_still_builds_and_runs() {
    let dir = ScratchDir::new("905_legal").expect("failed to create scratch dir");
    let src = write_fixture(
        &dir,
        "legal.py",
        "from typing import TYPE_CHECKING\n\nif TYPE_CHECKING:\n    x: int = 1\n\nprint(2)\n",
    );
    let exe = dir.join("legal");
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
        "2\n",
        "the folded guard must stay a no-op"
    );
}
