//! End-to-end coverage for issue #714: binding a user-declared exception
//! subclass as an ordinary value used to compile cleanly and then abort at
//! **runtime** with a `NameError` naming the synthetic `Exception.__init__`
//! placeholder -- the generated code called that placeholder's global
//! function-pointer slot before codegen's always-last module-position
//! binding for it had run, so the call observed a null pointer.
//!
//! Everything here goes through the public `pycc` CLI, exactly like
//! `tests/issue_702_user_exceptions.rs`: the point is that the whole
//! pipeline -- type checking, MIR lowering, codegen, and the produced
//! binary -- now agree that this shape is a compile-time diagnostic, not a
//! runtime abort, while `raise MyError("boom")` (the shape #714 must not
//! regress) keeps working end to end.

use pycc_scratch::ScratchDir;
use std::io::Write;
use std::process::Command;

fn pycc_bin() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_BIN_EXE_pycc"))
}

fn scratch(tag: &str) -> ScratchDir {
    ScratchDir::new(&format!("714_{tag}")).expect("failed to create scratch dir")
}

fn write_fixture(dir: &std::path::Path, name: &str, source: &str) -> std::path::PathBuf {
    let path = dir.join(name);
    let mut file = std::fs::File::create(&path).unwrap();
    file.write_all(source.as_bytes()).unwrap();
    path
}

/// The combined diagnostic text of a rejected `pycc check`.
fn check_error(tag: &str, source: &str) -> String {
    let dir = scratch(tag);
    let src = write_fixture(&dir, &format!("{tag}.py"), source);
    let output = Command::new(pycc_bin())
        .args(["check", src.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(!output.status.success(), "expected `{tag}` to be rejected");
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

/// `(exit_success, stdout, stderr)` of the built and run program.
fn build_and_run(tag: &str, source: &str) -> (bool, String, String) {
    let dir = scratch(tag);
    let src = write_fixture(&dir, &format!("{tag}.py"), source);
    let out = dir.join(tag);
    let build = Command::new(pycc_bin())
        .args(["build", src.to_str().unwrap(), "-o", out.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(
        build.status.success(),
        "build failed: {}",
        String::from_utf8_lossy(&build.stderr)
    );
    let run = Command::new(&out).output().unwrap();
    (
        run.status.success(),
        String::from_utf8_lossy(&run.stdout).to_string(),
        String::from_utf8_lossy(&run.stderr).to_string(),
    )
}

/// The exact reproduction from the issue: `pycc build` must now reject this
/// at compile time (rc != 0, a diagnostic) instead of succeeding and
/// producing a binary that aborts with `SIGABRT` on a `NameError` at
/// runtime.
#[test]
fn binding_a_user_exception_subclass_as_a_value_is_a_compile_time_diagnostic() {
    let text = check_error(
        "bind",
        "class MyError(Exception):\n    pass\n\n\n\
         def main() -> None:\n    e = MyError(\"boom\")\n    print(\"ok\")\n\n\n\
         main()\n",
    );
    assert!(text.contains("C0001"), "unexpected diagnostic: {text}");
    assert!(
        text.contains("cannot instantiate exception class"),
        "unexpected diagnostic: {text}"
    );
}

/// The same rejection applies through direct inheritance from `Exception`
/// or from a builtin exception family member, and regardless of whether the
/// binding is later used.
#[test]
fn binding_a_deeper_user_exception_subclass_as_a_value_is_also_rejected() {
    let text = check_error(
        "deep",
        "class AppError(Exception):\n    pass\n\n\n\
         class DatabaseError(AppError):\n    pass\n\n\n\
         def main() -> None:\n    e = DatabaseError(\"refused\")\n\n\n\
         main()\n",
    );
    assert!(text.contains("C0001"), "unexpected diagnostic: {text}");
}

/// #714 must not regress the shape it exists to keep working: raising a
/// freshly constructed user exception subclass, and catching it, still
/// builds and runs correctly end to end.
#[test]
fn raising_and_catching_the_same_user_exception_subclass_still_works() {
    let (ok, stdout, stderr) = build_and_run(
        "raise",
        "class MyError(Exception):\n    pass\n\n\n\
         def main() -> None:\n    try:\n        raise MyError(\"boom\")\n    \
         except MyError:\n        print(\"caught\")\n\n\n\
         main()\n",
    );
    assert!(ok, "program did not exit successfully: {stderr}");
    assert_eq!(stdout, "caught\n");
}

/// A malformed `raise` operand for a user exception subclass still reports
/// its own argument diagnostic (arity), matching the message an ordinary
/// function call reports -- #714's fix must not weaken this to the generic
/// "cannot instantiate" message.
#[test]
fn raising_a_user_exception_subclass_with_the_wrong_argument_count_is_rejected() {
    let text = check_error(
        "arity",
        "class MyError(Exception):\n    pass\n\n\n\
         def main() -> None:\n    raise MyError(\"a\", \"b\")\n\n\n\
         main()\n",
    );
    assert!(text.contains("T0021"), "unexpected diagnostic: {text}");
    assert!(
        text.contains("expects") && text.contains("argument"),
        "unexpected diagnostic: {text}"
    );
}

/// A malformed `raise` operand for a user exception subclass still reports
/// its own argument-type diagnostic, not the generic "cannot instantiate"
/// message.
#[test]
fn raising_a_user_exception_subclass_with_the_wrong_argument_type_is_rejected() {
    let text = check_error(
        "argtype",
        "class MyError(Exception):\n    pass\n\n\n\
         def main() -> None:\n    raise MyError(1)\n\n\n\
         main()\n",
    );
    assert!(text.contains("T0021"), "unexpected diagnostic: {text}");
    assert!(text.contains("str"), "unexpected diagnostic: {text}");
}
