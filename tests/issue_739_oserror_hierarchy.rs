//! End-to-end coverage for the PEP 3151 `OSError` exception hierarchy (Part 2
//! of #543, issue #739).
//!
//! Everything here goes through the public `pycc` CLI, mirroring
//! `tests/issue_702_user_exceptions.rs`'s `build_and_run`/`check_error`
//! harness: the point is that HIR tag assignment, MIR handler tag sets, the
//! type checker's raisability/shadow gates, and the runtime's name-carrying
//! `PyExceptionObj` all agree on the real, non-flat `OSError` tree.

use pycc_scratch::ScratchDir;
use std::io::Write;
use std::process::Command;

fn pycc_bin() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_BIN_EXE_pycc"))
}

fn scratch(tag: &str) -> ScratchDir {
    ScratchDir::new(&format!("739_{tag}")).expect("failed to create scratch dir")
}

fn write_fixture(dir: &std::path::Path, name: &str, source: &str) -> std::path::PathBuf {
    let path = dir.join(name);
    let mut file = std::fs::File::create(&path).unwrap();
    file.write_all(source.as_bytes()).unwrap();
    path
}

/// `(exit_success, stdout, stderr)` of the built program.
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

/// Whether a `pycc check` on `source` succeeds.
fn check_ok(tag: &str, source: &str) -> (bool, String) {
    let dir = scratch(tag);
    let src = write_fixture(&dir, &format!("{tag}.py"), source);
    let output = Command::new(pycc_bin())
        .args(["check", src.to_str().unwrap()])
        .output()
        .unwrap();
    (
        output.status.success(),
        format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        ),
    )
}

const ALL_16: &[&str] = &[
    "OSError",
    "BlockingIOError",
    "ChildProcessError",
    "ConnectionError",
    "FileExistsError",
    "FileNotFoundError",
    "InterruptedError",
    "IsADirectoryError",
    "NotADirectoryError",
    "PermissionError",
    "ProcessLookupError",
    "TimeoutError",
    "BrokenPipeError",
    "ConnectionAbortedError",
    "ConnectionRefusedError",
    "ConnectionResetError",
];

// -- raise and catch each of the 16, by its own name -------------------

#[test]
fn each_of_the_16_classes_is_raisable_and_catchable_by_its_own_name() {
    for name in ALL_16 {
        let tag = format!("own_name_{name}");
        let (ok, stdout, stderr) = build_and_run(
            &tag,
            &format!(
                "def main() -> None:\n\
                 \x20   try:\n        raise {name}(\"boom\")\n\
                 \x20   except {name}:\n        print(\"caught\")\n\n\n\
                 main()\n"
            ),
        );
        assert!(ok, "`{name}` program failed: {stderr}");
        assert_eq!(stdout, "caught\n", "`{name}` did not report caught");
    }
}

// -- except OSError catches every family member -------------------------

#[test]
fn except_os_error_catches_a_direct_child() {
    let (ok, stdout, stderr) = build_and_run(
        "os_direct",
        "def main() -> None:\n\
         \x20   try:\n        raise FileNotFoundError(\"missing\")\n\
         \x20   except OSError:\n        print(\"caught\")\n\n\n\
         main()\n",
    );
    assert!(ok, "program failed: {stderr}");
    assert_eq!(stdout, "caught\n");
}

#[test]
fn except_os_error_catches_a_two_level_descendant() {
    let (ok, stdout, stderr) = build_and_run(
        "os_two_level",
        "def main() -> None:\n\
         \x20   try:\n        raise BrokenPipeError(\"pipe\")\n\
         \x20   except OSError:\n        print(\"caught\")\n\n\n\
         main()\n",
    );
    assert!(ok, "program failed: {stderr}");
    assert_eq!(stdout, "caught\n");
}

// -- except ConnectionError catches its own four children, not a sibling ---

#[test]
fn except_connection_error_catches_each_of_its_four_children() {
    for name in [
        "BrokenPipeError",
        "ConnectionAbortedError",
        "ConnectionRefusedError",
        "ConnectionResetError",
    ] {
        let tag = format!("conn_{name}");
        let (ok, stdout, stderr) = build_and_run(
            &tag,
            &format!(
                "def main() -> None:\n\
                 \x20   try:\n        raise {name}(\"x\")\n\
                 \x20   except ConnectionError:\n        print(\"caught\")\n\n\n\
                 main()\n"
            ),
        );
        assert!(ok, "`{name}` program failed: {stderr}");
        assert_eq!(
            stdout, "caught\n",
            "`{name}` was not caught by ConnectionError"
        );
    }
}

#[test]
fn except_connection_error_does_not_catch_an_unrelated_os_error_sibling() {
    // `TimeoutError` is a sibling of `ConnectionError` under `OSError`, not
    // one of its children -- mirrors
    // `tests/issue_702_user_exceptions.rs`'s `a_subclass_handler_does_not_catch_its_sibling`.
    let (ok, stdout, stderr) = build_and_run(
        "conn_sibling",
        "def main() -> None:\n\
         \x20   try:\n        raise TimeoutError(\"slow\")\n\
         \x20   except ConnectionError:\n        print(\"connection\")\n\
         \x20   except OSError:\n        print(\"os\")\n\n\n\
         main()\n",
    );
    assert!(ok, "program failed: {stderr}");
    assert_eq!(stdout, "os\n");
}

// -- an unrelated builtin does not catch, and vice versa ------------------

#[test]
fn a_value_error_handler_does_not_catch_an_os_error_family_exception() {
    let (ok, stdout, stderr) = build_and_run(
        "unrelated_handler",
        "def main() -> None:\n\
         \x20   try:\n        raise FileNotFoundError(\"missing\")\n\
         \x20   except ValueError:\n        print(\"value\")\n\
         \x20   except OSError:\n        print(\"os\")\n\n\n\
         main()\n",
    );
    assert!(ok, "program failed: {stderr}");
    assert_eq!(stdout, "os\n");
}

#[test]
fn an_os_error_handler_does_not_catch_an_unrelated_value_error() {
    let (ok, stdout, stderr) = build_and_run(
        "unrelated_raise",
        "def main() -> None:\n\
         \x20   try:\n        raise ValueError(\"bad\")\n\
         \x20   except OSError:\n        print(\"os\")\n\
         \x20   except ValueError:\n        print(\"value\")\n\n\n\
         main()\n",
    );
    assert!(ok, "program failed: {stderr}");
    assert_eq!(stdout, "value\n");
}

// -- except Exception catches every new class (tag-0 catch-all shortcut) ---

#[test]
fn except_exception_catches_every_new_class() {
    for name in ALL_16 {
        let tag = format!("catch_all_{name}");
        let (ok, stdout, stderr) = build_and_run(
            &tag,
            &format!(
                "def main() -> None:\n\
                 \x20   try:\n        raise {name}(\"x\")\n\
                 \x20   except Exception:\n        print(\"caught\")\n\n\n\
                 main()\n"
            ),
        );
        assert!(ok, "`{name}` program failed: {stderr}");
        assert_eq!(stdout, "caught\n", "`{name}` was not caught by Exception");
    }
}

// -- an uncaught instance prints its own class name ------------------------

#[test]
fn an_uncaught_file_not_found_error_prints_its_own_class_name() {
    let (ok, stdout, stderr) = build_and_run(
        "uncaught_fnf",
        "def main() -> None:\n    raise FileNotFoundError(\"missing\")\n\n\nmain()\n",
    );
    assert!(!ok, "an uncaught exception must exit non-zero");
    assert_eq!(stdout, "");
    // #707 added a traceback-frame header naming the enclosing function.
    assert_eq!(
        stderr,
        "Traceback (most recent call last):\n  File \"<compiled>\", in main\nFileNotFoundError: missing\n"
    );
}

#[test]
fn an_uncaught_broken_pipe_error_prints_its_own_class_name() {
    let (ok, stdout, stderr) = build_and_run(
        "uncaught_bpe",
        "def main() -> None:\n    raise BrokenPipeError(\"pipe\")\n\n\nmain()\n",
    );
    assert!(!ok, "an uncaught exception must exit non-zero");
    assert_eq!(stdout, "");
    // #707 added a traceback-frame header naming the enclosing function.
    assert_eq!(
        stderr,
        "Traceback (most recent call last):\n  File \"<compiled>\", in main\nBrokenPipeError: pipe\n"
    );
}

// -- C0001 no longer fires for the 16 names on raise/except; still fires for
//    instantiation-as-a-value ---------------------------------------------

#[test]
fn raising_and_catching_the_16_no_longer_hits_c0001() {
    for name in ALL_16 {
        let tag = format!("no_c0001_{name}");
        let (ok, text) = check_ok(
            &tag,
            &format!(
                "def main() -> None:\n\
                 \x20   try:\n        raise {name}(\"x\")\n\
                 \x20   except {name}:\n        pass\n"
            ),
        );
        assert!(ok, "`{name}` should type-check cleanly: {text}");
    }
}

#[test]
fn instantiating_an_os_error_family_class_as_a_value_is_still_c0001() {
    // Unchanged from before Part 2 of #543: only the *raise*/`except`
    // surface is new here. Binding the instantiation result to a variable
    // stays `C0001`, exactly as for the original flat seven -- verified
    // directly against `ValueError` below, so this is not a regression
    // specific to the new classes. (The exact diagnostic text observed here
    // -- "call to builtin ... not implemented yet", from
    // `KNOWN_CALLABLE_BUILTINS`'s capability-gap classification in
    // `expr.rs` -- is a pre-existing property of this exact
    // `x = ValueError("...")`-shaped source for *every* builtin exception
    // class, not something Part 2 of #543 changes; the class-table
    // "cannot instantiate builtin exception class" message is reached only
    // via the direct `class::resolve_instantiation` API, exercised in
    // `pycc_types::exception::synthetic_class_tests`.)
    let os_family_text = check_error(
        "value_c0001",
        "def main() -> None:\n    e = FileNotFoundError(\"x\")\n    print(e)\n",
    );
    assert!(
        os_family_text.contains("C0001"),
        "unexpected diagnostic: {os_family_text}"
    );

    let flat_seven_text = check_error(
        "value_c0001_precedent",
        "def main() -> None:\n    e = ValueError(\"x\")\n    print(e)\n",
    );
    assert!(
        flat_seven_text.contains("C0001"),
        "unexpected diagnostic: {flat_seven_text}"
    );
}

// -- a user class inheriting from one of the new classes -------------------

#[test]
fn a_user_class_may_inherit_from_a_deep_oserror_family_member() {
    // Exercises `compute_c3_mro`'s depth-agnostic correctness for a *user*
    // class subclassing a depth-3 builtin, previously unexercised past
    // depth 2 (every prior user-exception test subclassed a depth-1
    // builtin).
    let (ok, stdout, stderr) = build_and_run(
        "user_subclass",
        "class MyIOError(FileNotFoundError):\n    pass\n\n\n\
         def main() -> None:\n\
         \x20   try:\n        raise MyIOError(\"x\")\n\
         \x20   except MyIOError:\n        print(\"own\")\n\n\n\
         main()\n",
    );
    assert!(ok, "program failed: {stderr}");
    assert_eq!(stdout, "own\n");
}

#[test]
fn a_user_class_inheriting_a_deep_family_member_is_caught_by_os_error_directly() {
    let (ok, stdout, stderr) = build_and_run(
        "user_subclass_base_catch",
        "class MyIOError(FileNotFoundError):\n    pass\n\n\n\
         def main() -> None:\n\
         \x20   try:\n        raise MyIOError(\"x\")\n\
         \x20   except OSError:\n        print(\"base\")\n\n\n\
         main()\n",
    );
    assert!(ok, "program failed: {stderr}");
    assert_eq!(stdout, "base\n");
}

// -- work item 5: the shadow-gate crash-risk regression test ---------------

/// A module that shadows one `OSError`-family name at top level (which
/// withholds seeding for *every* builtin exception name, per the
/// all-or-nothing shadow gate) and, unrelated to it, names a different
/// family member in a bare `except`, must compile to a clean `T0021` --
/// never a compiler-internal panic. This is the regression test for work
/// item 5's `is_unshadowed_builtin_exception` fix; without it, an
/// unmitigated widening of `BUILTIN_EXCEPTION_CLASSES` would let
/// `pycc_mir::exception::handler_type_tags`'s `.expect()` panic on this
/// input instead.
#[test]
fn a_shadowed_family_member_elsewhere_turns_an_unrelated_except_into_a_clean_diagnostic() {
    let text = check_error(
        "shadow_except",
        "class OSError:\n    def __init__(self) -> None:\n        pass\n\n\n\
         def main() -> None:\n\
         \x20   try:\n        pass\n\
         \x20   except FileNotFoundError:\n        pass\n",
    );
    assert!(
        text.contains("T0021"),
        "expected a clean T0021, not a panic: {text}"
    );
    assert!(
        text.contains("not a recognized exception class"),
        "unexpected diagnostic: {text}"
    );
}

/// The `raise`-side twin of the test above: `check_raise_operand` calls the
/// identical `is_unshadowed_builtin_exception` predicate, so this exercises
/// the same fixed conjunct through its other call site.
///
/// The resulting diagnostic code differs from the `except`-side test above
/// (`C0001`, not `T0021`) even though both call sites share the fixed
/// predicate: with `is_unshadowed_builtin_exception` now `false` and
/// `FileNotFoundError` not a user-defined class either,
/// `check_raise_operand`'s final fallback calls `infer_expr_in` on the
/// `FileNotFoundError("x")` call expression, which -- finding no seeded
/// `HirClassDef` for it (seeding was withheld module-wide) -- reaches
/// `expr.rs`'s own `KNOWN_CALLABLE_BUILTINS` capability-gap classification
/// and returns `C0001` before `check_raise_operand`'s own `T0021` fallback
/// is ever reached. What matters for this regression test is unchanged: a
/// clean compile-time diagnostic, not a panic.
#[test]
fn a_shadowed_family_member_elsewhere_turns_an_unrelated_raise_into_a_clean_diagnostic() {
    let text = check_error(
        "shadow_raise",
        "class OSError:\n    def __init__(self) -> None:\n        pass\n\n\n\
         def main() -> None:\n    raise FileNotFoundError(\"x\")\n",
    );
    assert!(
        text.contains("C0001"),
        "expected a clean diagnostic, not a panic: {text}"
    );
}
