//! End-to-end coverage for #815 (Part 1 of #737, fixes #711): an explicit
//! dunder call on a synthesized builtin-exception class must produce a
//! `C0001` diagnostic instead of a compiler panic (#711) or a clean
//! type-check that only aborts at runtime (#714).
//!
//! Follows the same per-issue test file convention as its siblings
//! (`tests/issue_702_user_exceptions.rs`'s `check_error` harness).

use pycc_scratch::ScratchDir;
use std::io::Write;
use std::process::Command;

fn pycc_bin() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_BIN_EXE_pycc"))
}

fn write_fixture(dir: &std::path::Path, name: &str, source: &str) -> std::path::PathBuf {
    let path = dir.join(name);
    let mut file = std::fs::File::create(&path).unwrap();
    file.write_all(source.as_bytes()).unwrap();
    path
}

/// The combined diagnostic text of a rejected `pycc check`.
fn check_error(tag: &str, source: &str) -> String {
    let dir = ScratchDir::new(&format!("711_{tag}")).expect("failed to create scratch dir");
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

/// #711's own reproducer: a direct dunder call on a caught builtin
/// `Exception` instance used to hit `resolve_method_call`'s
/// `lookup_function` panic (`Exception.__init__` is in `Exception`'s own
/// method table but was never registered as an ordinary function, because
/// D-173 propagates a raised exception through global runtime state rather
/// than a real allocated instance with real methods). It must now be a
/// clean `C0001` diagnostic, not a panic/abort.
#[test]
fn calling_a_dunder_directly_on_a_caught_builtin_exception_is_a_clean_diagnostic() {
    let text = check_error(
        "direct",
        "def main() -> None:\n\
         \x20   try:\n        raise Exception(\"boom\")\n\
         \x20   except Exception as e:\n        e.__init__(\"oops\")\n",
    );
    assert!(text.contains("C0001"), "unexpected diagnostic: {text}");
    assert!(
        text.contains("cannot call `__init__` directly on `Exception`"),
        "unexpected diagnostic: {text}"
    );
}

/// #714's reproducer, the load-bearing regression test distinguishing this
/// design from the rejected "guard only when `lookup_function` would fail"
/// alternative: `MyError` is a user subclass with no method of its own, so
/// `any_user_exception_class` makes the mangled `Exception.__init__` HIR
/// item exist -- `lookup_function` would *succeed* here, so a guard keyed
/// only on lookup failure would miss this case entirely and let the
/// *built binary* abort at runtime instead. The guard must fire on the
/// class that actually owns the resolved method (`Exception`, found via
/// the MRO walk), not on `MyError` (the call's own receiver class), which
/// is why this must produce the identical `C0001` diagnostic named above.
///
/// The receiver is obtained through a parameter annotation rather than
/// `e = MyError("x")`: #714 made that instantiation itself a `C0001`
/// (binding the inherited, synthetic-placeholder constructor's result to a
/// name), which would otherwise fire first and mask the dunder-call
/// diagnostic this test exists to pin. A parameter of type `MyError` still
/// produces the exact `Ty::Instance("MyError")` receiver the dunder call
/// needs, without ever going through `resolve_instantiation`.
#[test]
fn calling_a_dunder_on_an_instance_of_a_user_subclass_with_no_own_method_is_the_same_diagnostic() {
    let text = check_error(
        "inherited",
        "class MyError(Exception):\n    pass\n\n\n\
         def main(e: MyError) -> None:\n\
         \x20   e.__init__(\"y\")\n",
    );
    assert!(text.contains("C0001"), "unexpected diagnostic: {text}");
    assert!(
        text.contains("cannot call `__init__` directly on `Exception`"),
        "unexpected diagnostic: {text}"
    );
}
