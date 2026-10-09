//! End-to-end coverage for #1369: the `assert` statement and the appended
//! `AssertionError` builtin exception class (tag 28) it raises.
//!
//! Expected outputs are CPython 3.14.7's, checked by hand against the same
//! sources; `tests/fixtures/assert_statement.py` is the byte-for-byte oracle
//! comparison. An uncaught exception is compared only on its final stderr
//! line and a non-zero exit, because pycc's traceback frames differ from
//! CPython's by design.
//!
//! Everything here goes through the public `pycc` CLI, mirroring
//! `tests/issue_1292_import_error.rs`'s harness. The whole-program tests show
//! what the `pycc_hir` unit tests cannot: that HIR tag assignment,
//! `builtin_exception_parent`'s new arm, MIR handler tag sets and the
//! runtime's name-carrying `PyExceptionObj` agree on CPython's real
//! hierarchy, `AssertionError` -> `Exception`.
//!
//! The `ext`-mode tests are `#[ignore]`d because they ask an installed
//! CPython 3.13+ to import a built artifact, which is a property of the
//! machine. CI runs them on every Tier-1 `native-build-test` leg through that
//! job's `cargo test --workspace -- --include-ignored`.

use pycc_scratch::ScratchDir;
use std::io::Write;
use std::path::Path;
use std::process::{Command, Output};

fn pycc_bin() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_BIN_EXE_pycc"))
}

fn scratch(tag: &str) -> ScratchDir {
    ScratchDir::new(&format!("1369_{tag}")).expect("failed to create scratch dir")
}

fn write_fixture(dir: &Path, name: &str, source: &str) -> std::path::PathBuf {
    let path = dir.join(name);
    let mut file = std::fs::File::create(&path).unwrap();
    file.write_all(source.as_bytes()).unwrap();
    path
}

/// `(exit_success, stdout, stderr)` of the built program, with `\r\n`
/// normalized to `\n`.
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
        String::from_utf8_lossy(&run.stdout).replace("\r\n", "\n"),
        String::from_utf8_lossy(&run.stderr).replace("\r\n", "\n"),
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

fn python() -> Command {
    Command::new(std::env::var_os("PYCC_PYTHON").unwrap_or_else(|| "python3".into()))
}

fn stdout_of(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).replace("\r\n", "\n")
}

fn stderr_of(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).replace("\r\n", "\n")
}

/// Builds `body` as the extension module `module` inside `dir`, with
/// `dir/hostlib` on `PYTHONPATH`.
fn build_ext(dir: &Path, module: &str, body: &str) {
    let src = write_fixture(dir, "m.py", body);
    let build = Command::new(pycc_bin())
        .arg("build")
        .arg(&src)
        .arg("-o")
        .arg(dir.join(module))
        .arg("--ext")
        .env("PYTHONPATH", dir.join("hostlib"))
        .output()
        .expect("pycc should spawn");
    assert!(
        build.status.success(),
        "{}{}",
        stdout_of(&build),
        stderr_of(&build)
    );
}

/// Runs `script` in the host interpreter with `dir` as the working directory
/// and `dir/hostlib` -- the host-only helper modules, which pycc must not
/// compile as sibling modules -- on `PYTHONPATH`.
fn run_host(dir: &Path, script: &str) -> Output {
    let run = python()
        .arg("-c")
        .arg(script)
        .current_dir(dir)
        .env("PYTHONPATH", dir.join("hostlib"))
        .env("PYTHONUNBUFFERED", "1")
        .output()
        .expect("python3 should spawn");
    assert!(
        run.status.success(),
        "stdout: {}\nstderr: {}",
        stdout_of(&run),
        stderr_of(&run)
    );
    run
}

/// `AssertionError` is raisable and catchable by its own name and by
/// `except Exception:`, and `issubclass` follows its real parentage.
#[test]
fn assertion_error_is_catchable_by_its_own_name_and_by_exception() {
    let (ok, stdout, stderr) = build_and_run(
        "raise_catch",
        "def check() -> None:\n\
         \x20   raise AssertionError(\"broken\")\n\n\n\
         def main() -> None:\n\
         \x20   try:\n\
         \x20       check()\n\
         \x20   except AssertionError as e:\n\
         \x20       print(\"own:\", e)\n\
         \x20   try:\n\
         \x20       check()\n\
         \x20   except ValueError as e:\n\
         \x20       print(\"wrong handler:\", e)\n\
         \x20   except Exception as e:\n\
         \x20       print(\"base:\", e)\n\
         \x20   print(issubclass(AssertionError, Exception), \
         issubclass(AssertionError, ValueError))\n\n\n\
         main()\n",
    );
    assert!(ok, "program failed: {stderr}");
    assert_eq!(stdout, "own: broken\nbase: broken\nTrue False\n");
}

/// A user subclass of `AssertionError` is raisable and caught by
/// `except AssertionError:`.
#[test]
fn a_user_assertion_error_subclass_is_caught_by_assertion_error() {
    let (ok, stdout, stderr) = build_and_run(
        "user_subclass",
        "class InvariantBroken(AssertionError):\n    pass\n\n\n\
         def main() -> None:\n\
         \x20   try:\n\
         \x20       raise InvariantBroken(\"inv\")\n\
         \x20   except AssertionError as e:\n\
         \x20       print(\"caught:\", e)\n\n\n\
         main()\n",
    );
    assert!(ok, "program failed: {stderr}");
    assert_eq!(stdout, "caught: inv\n");
}

/// An uncaught `AssertionError` names itself at the top level, which pins the
/// tag-to-name round trip through `PyExceptionObj`'s carried name.
#[test]
fn an_uncaught_assertion_error_reports_its_own_class_name() {
    let (ok, _stdout, stderr) = build_and_run(
        "uncaught",
        "def main() -> None:\n\
         \x20   raise AssertionError(\"top\")\n\n\n\
         main()\n",
    );
    assert!(!ok, "the program should have exited non-zero");
    assert_eq!(
        stderr.lines().last(),
        Some("AssertionError: top"),
        "unexpected stderr: {stderr}"
    );
}

/// The all-or-nothing shadow gate now covers the new name (documented in
/// `docs/RUNTIME.md`'s shadow-gate paragraph): a module declaring its own
/// `class AssertionError(Exception)` withholds seeding, so its base
/// `Exception` is unknown -- exactly as a user `class ImportError(Exception)`
/// already did.
#[test]
fn a_user_class_named_assertion_error_withholds_seeding() {
    let text = check_error(
        "user_assertion_error",
        "class AssertionError(Exception):\n    pass\n\n\n\
         def main() -> None:\n\
         \x20   try:\n\
         \x20       raise AssertionError(\"m\")\n\
         \x20   except AssertionError:\n\
         \x20       print(\"caught\")\n\n\n\
         main()\n",
    );
    assert!(text.contains("C0001"), "unexpected diagnostic: {text}");
    assert!(
        text.contains("class `AssertionError` inherits from unknown class `Exception`"),
        "unexpected diagnostic: {text}"
    );
}

/// The `ext`-mode half: the C shim's `case 28:` hands the host CPython's own
/// `AssertionError`; without it tag 28 falls through `default:` and the host
/// sees a plain `Exception`.
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn a_built_ext_module_raises_cpython_assertion_error() {
    let dir = ScratchDir::new("1369_ext").expect("scratch");
    build_ext(
        &dir,
        "pycc_assertion_error_mod",
        "def f(n: int) -> int:\n\
         \x20   if n == 0:\n\
         \x20       raise AssertionError(\"explicit\")\n\
         \x20   return n\n",
    );
    let run = run_host(
        &dir,
        "import pycc_assertion_error_mod as m\n\
         try:\n\
         \x20   m.f(0)\n\
         \x20   raise RuntimeError('expected an AssertionError')\n\
         except AssertionError as e:\n\
         \x20   print(type(e).__name__, type(e).__mro__[1].__name__, repr(str(e)))\n\
         print(m.f(7))\n",
    );
    assert_eq!(stdout_of(&run), "AssertionError Exception 'explicit'\n7\n");
}

/// The #1316 foreign-to-pycc direction: a host `AssertionError` raised by a
/// foreign call inside a compiled function now maps to tag 28, so the
/// function's own `except AssertionError:` catches it (before #1369 it was
/// bridged as a plain `Exception` and the handler could not name it).
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn a_foreign_assertion_error_is_caught_by_except_assertion_error() {
    let dir = ScratchDir::new("1369_bridge").expect("scratch");
    let hostlib = dir.join("hostlib");
    std::fs::create_dir_all(&hostlib).expect("create hostlib");
    write_fixture(
        &hostlib,
        "pycc_1369_helper.py",
        "def boom():\n    raise AssertionError('from the host')\n",
    );
    build_ext(
        &dir,
        "pycc_1369_bridge",
        "import pycc_1369_helper\n\n\n\
         def f() -> int:\n\
         \x20   try:\n\
         \x20       pycc_1369_helper.boom()\n\
         \x20   except ValueError:\n\
         \x20       return 1\n\
         \x20   except AssertionError:\n\
         \x20       return 2\n\
         \x20   return 0\n",
    );
    let run = run_host(&dir, "import pycc_1369_bridge as m\nprint(m.f())\n");
    assert_eq!(stdout_of(&run), "2\n");
}

// -- the `assert` statement ---------------------------------------------

/// A passing `assert` does nothing; a failing one raises `AssertionError`
/// carrying its message, or an empty one when there is none.
#[test]
fn a_failing_assert_raises_assertion_error_with_its_message() {
    let (ok, stdout, stderr) = build_and_run(
        "assert_basic",
        "def check(x: int) -> None:\n\
         \x20   assert x > 0\n\
         \x20   assert x > 1, \"need more than one\"\n\
         \x20   print(\"ok\", x)\n\n\n\
         def main() -> None:\n\
         \x20   check(5)\n\
         \x20   try:\n\
         \x20       check(1)\n\
         \x20   except AssertionError as e:\n\
         \x20       print(f\"message [{e}]\")\n\
         \x20   try:\n\
         \x20       check(0)\n\
         \x20   except Exception as e:\n\
         \x20       print(f\"empty [{e}]\")\n\n\n\
         main()\n",
    );
    assert!(ok, "program failed: {stderr}");
    assert_eq!(stdout, "ok 5\nmessage [need more than one]\nempty []\n");
}

/// The message is evaluated only when the test fails, and the test exactly
/// once -- CPython's order, which the `if`/`else` rewrite gives by
/// construction.
#[test]
fn the_message_is_evaluated_only_on_failure() {
    let (ok, stdout, stderr) = build_and_run(
        "assert_lazy",
        "def note(tag: str) -> str:\n\
         \x20   print(\"evaluated\", tag)\n\
         \x20   return tag\n\n\n\
         def probe() -> bool:\n\
         \x20   print(\"tested\")\n\
         \x20   return False\n\n\n\
         def main() -> None:\n\
         \x20   assert True, note(\"passing\")\n\
         \x20   try:\n\
         \x20       assert probe(), note(\"failing\")\n\
         \x20   except AssertionError as e:\n\
         \x20       print(f\"caught {e}\")\n\n\n\
         main()\n",
    );
    assert!(ok, "program failed: {stderr}");
    assert_eq!(stdout, "tested\nevaluated failing\ncaught failing\n");
}

/// The test uses `if` truthiness, so a non-`bool` test works as it does in
/// CPython: a non-empty `str`, a non-zero `int` or `float` passes, and the
/// empty/zero values fail.
#[test]
fn a_non_bool_test_uses_python_truthiness() {
    let (ok, stdout, stderr) = build_and_run(
        "assert_truthiness",
        "def fails(label: str, s: str, n: int, f: float) -> None:\n\
         \x20   try:\n\
         \x20       assert s, \"str\"\n\
         \x20       assert n, \"int\"\n\
         \x20       assert f, \"float\"\n\
         \x20       print(label, \"passed\")\n\
         \x20   except AssertionError as e:\n\
         \x20       print(label, f\"failed on {e}\")\n\n\n\
         def main() -> None:\n\
         \x20   fails(\"a\", \"x\", 3, 0.5)\n\
         \x20   fails(\"b\", \"\", 3, 0.5)\n\
         \x20   fails(\"c\", \"x\", 0, 0.5)\n\
         \x20   fails(\"d\", \"x\", 3, 0.0)\n\n\n\
         main()\n",
    );
    assert!(ok, "program failed: {stderr}");
    assert_eq!(
        stdout,
        "a passed\nb failed on str\nc failed on int\nd failed on float\n"
    );
}

/// An uncaught failing `assert` with no message ends in a bare
/// `AssertionError` line, exactly CPython's; with a message, in
/// `AssertionError: <message>`. Both at module level.
#[test]
fn an_uncaught_module_level_assert_reports_like_cpython() {
    let (ok, stdout, stderr) = build_and_run(
        "assert_uncaught_bare",
        "assert 1 < 2\nprint(\"before\")\nassert 2 < 1\nprint(\"after\")\n",
    );
    assert!(!ok, "the program should have exited non-zero");
    assert_eq!(stdout, "before\n");
    assert_eq!(stderr.lines().last(), Some("AssertionError"), "{stderr}");

    let (ok, _stdout, stderr) =
        build_and_run("assert_uncaught_msg", "assert 2 < 1, \"two is not less\"\n");
    assert!(!ok, "the program should have exited non-zero");
    assert_eq!(
        stderr.lines().last(),
        Some("AssertionError: two is not less"),
        "{stderr}"
    );
}

/// The runtime half of the no-message case: CPython prints a bare
/// `ValueError` for `raise ValueError("")`, so an empty message drops the
/// `: ` separator for every class, not just `AssertionError`.
#[test]
fn an_uncaught_empty_message_prints_the_bare_class_name() {
    let (ok, _stdout, stderr) = build_and_run("empty_value_error", "raise ValueError(\"\")\n");
    assert!(!ok, "the program should have exited non-zero");
    assert_eq!(stderr.lines().last(), Some("ValueError"), "{stderr}");
}

/// A walrus in the test is admitted, as it is in an `if` test, and the name
/// it binds is usable after the `assert`.
#[test]
fn a_walrus_in_the_test_binds_a_name_usable_afterwards() {
    let (ok, stdout, stderr) = build_and_run(
        "assert_walrus_test",
        "def main() -> None:\n\
         \x20   assert (y := 4) > 3\n\
         \x20   print(y + 1)\n\n\n\
         main()\n",
    );
    assert!(ok, "program failed: {stderr}");
    assert_eq!(stdout, "5\n");
}

/// A walrus in the message is refused exactly as it is in a `raise`
/// operand, which the message becomes.
#[test]
fn a_walrus_in_the_message_is_refused() {
    let text = check_error(
        "assert_walrus_msg",
        "def main() -> None:\n    assert False, (m := \"x\")\n\n\nmain()\n",
    );
    assert!(text.contains("C0001"), "unexpected diagnostic: {text}");
    assert!(
        text.contains("a walrus assignment (`:=`) is only supported in an `if`/`while`"),
        "unexpected diagnostic: {text}"
    );
}

/// `AssertionError` takes a `str` message, like every builtin exception
/// pycc constructs, so a non-`str` message is `T0021` rather than an
/// implicit `str()` conversion.
#[test]
fn a_non_str_message_is_rejected() {
    let text = check_error(
        "assert_int_msg",
        "def main() -> None:\n    assert False, 3\n\n\nmain()\n",
    );
    assert!(
        text.contains("T0021")
            && text.contains("`AssertionError` expects a `str` message argument, got `int`"),
        "unexpected diagnostic: {text}"
    );
}

/// `assert TYPE_CHECKING` always fails at runtime, as in CPython: the
/// constant is `False` there, and the rewrite's `if TYPE_CHECKING:` fold
/// keeps only the raising `else` branch.
#[test]
fn assert_type_checking_always_fails() {
    let (ok, stdout, stderr) = build_and_run(
        "assert_type_checking",
        "from typing import TYPE_CHECKING\n\n\n\
         def main() -> None:\n\
         \x20   try:\n\
         \x20       assert TYPE_CHECKING, \"not at runtime\"\n\
         \x20   except AssertionError as e:\n\
         \x20       print(f\"{e}\")\n\n\n\
         main()\n",
    );
    assert!(ok, "program failed: {stderr}");
    assert_eq!(stdout, "not at runtime\n");
}

/// An `assert` in a class body stays refused with the class-body `C0001`,
/// like every other non-definition statement there.
#[test]
fn an_assert_in_a_class_body_is_refused() {
    let text = check_error("assert_class_body", "class C:\n    assert True\n");
    assert!(
        text.contains("C0001")
            && text.contains("a class body statement must be a method definition"),
        "unexpected diagnostic: {text}"
    );
}

/// A module whose top level binds a builtin exception name withholds every
/// builtin class, so an `assert` in it is refused with one clear `C0001`
/// naming that binding -- including a top-level binding of
/// `AssertionError` itself, which CPython's `assert` would ignore.
#[test]
fn an_assert_in_a_module_that_binds_a_builtin_exception_name_is_refused() {
    for (tag, source, name) in [
        (
            "assert_shadow_value_error",
            "class ValueError:\n    pass\n\n\ndef f() -> None:\n    assert True\n",
            "ValueError",
        ),
        (
            "assert_shadow_assign",
            "AssertionError = 3\nassert False\n",
            "AssertionError",
        ),
        (
            "assert_shadow_def",
            "def AssertionError() -> None:\n    pass\n\n\nassert False\n",
            "AssertionError",
        ),
    ] {
        let text = check_error(tag, source);
        assert!(
            text.contains("C0001")
                && text.contains(&format!(
                    "an `assert` statement needs the builtin `AssertionError`, which is \
                     unavailable because this module binds the builtin exception name \
                     `{name}` at top level"
                )),
            "unexpected diagnostic for {tag}: {text}"
        );
    }
}

/// The refusal joins the per-item diagnostic collection in source order: a
/// module with an earlier independently failing top-level statement reports
/// that one first, then the `assert` refusal, and nothing is skipped.
#[test]
fn an_earlier_failure_is_reported_before_the_assert_refusal() {
    let text = check_error(
        "assert_shadow_after_failure",
        "x = 1 @ 2\nclass ValueError:\n    pass\n\n\nassert True\n",
    );
    let earlier = text
        .find("binary operator `@` is not supported yet")
        .unwrap_or_else(|| panic!("missing the earlier diagnostic: {text}"));
    let refusal = text
        .find("an `assert` statement needs the builtin `AssertionError`")
        .unwrap_or_else(|| panic!("missing the assert refusal: {text}"));
    assert!(earlier < refusal, "wrong order: {text}");
}

/// An `assert` inside an `if TYPE_CHECKING:` body is dead code the #790 fold
/// discards, so a module that shadows a builtin exception name still
/// compiles and runs when its only `assert`s sit there -- at top level, in an
/// `elif` arm, or inside a function.
#[test]
fn an_assert_under_a_type_checking_guard_is_not_refused() {
    let (ok, stdout, stderr) = build_and_run(
        "assert_shadow_type_checking",
        "from typing import TYPE_CHECKING\n\
         class ValueError:\n\
         \x20   pass\n\
         if TYPE_CHECKING:\n\
         \x20   assert False\n\
         x = 1\n\
         if x:\n\
         \x20   pass\n\
         elif TYPE_CHECKING:\n\
         \x20   assert False\n\
         def f() -> None:\n\
         \x20   if TYPE_CHECKING:\n\
         \x20       assert False\n\
         \x20   print(\"ok\")\n\
         f()\n",
    );
    assert!(ok, "stdout: {stdout}\nstderr: {stderr}");
    assert_eq!(stdout, "ok\n");
}

/// The `else:` of a `TYPE_CHECKING` guard is live at runtime, so an `assert`
/// there in a shadowing module is still refused.
#[test]
fn an_assert_in_the_else_of_a_type_checking_guard_is_refused() {
    let text = check_error(
        "assert_shadow_type_checking_else",
        "from typing import TYPE_CHECKING\nclass ValueError:\n    pass\n\
         if TYPE_CHECKING:\n    pass\nelse:\n    assert False\n",
    );
    assert!(
        text.contains("C0001")
            && text.contains("an `assert` statement needs the builtin `AssertionError`"),
        "unexpected diagnostic: {text}"
    );
}

/// An `assert` directly in a class body never reaches statement lowering,
/// so a shadowing module reports the class lowering's own refusal for it,
/// not the `AssertionError` one; an `assert` in a method of the same kind of
/// module still gets the `AssertionError` refusal.
#[test]
fn a_class_body_assert_keeps_its_own_diagnostic_in_a_shadowing_module() {
    let text = check_error(
        "assert_shadow_class_body",
        "class ValueError:\n    assert True\n",
    );
    assert!(
        text.contains("a class body statement must be a method definition")
            && !text.contains("needs the builtin `AssertionError`"),
        "unexpected diagnostic: {text}"
    );
    let text = check_error(
        "assert_shadow_method",
        "class ValueError:\n    def m(self) -> None:\n        assert True\n",
    );
    assert!(
        text.contains("C0001") && text.contains("needs the builtin `AssertionError`"),
        "unexpected diagnostic: {text}"
    );
}

/// A function-local binding of `AssertionError` cannot hold a class, so the
/// rewritten call is refused with `T0021` rather than calling something
/// other than the builtin -- never a miscompilation.
#[test]
fn a_function_local_assertion_error_binding_is_refused() {
    for (tag, source) in [
        (
            "assert_local_assign",
            "def f() -> None:\n    AssertionError = 3\n    assert False\n\n\nf()\n",
        ),
        (
            "assert_param",
            "def f(AssertionError: int) -> None:\n    assert False\n\n\nf(1)\n",
        ),
    ] {
        let text = check_error(tag, source);
        assert!(
            text.contains("T0021")
                && text.contains("name `AssertionError` is bound to a non-callable value"),
            "unexpected diagnostic for {tag}: {text}"
        );
    }
}

/// Every other binding form of `AssertionError` is refused by a diagnostic
/// that already existed, so the rewritten call can never reach something
/// other than the builtin class: the module fails to compile rather than
/// compiling and raising something else.
#[test]
fn other_assertion_error_binding_forms_are_refused() {
    for (tag, source, expected) in [
        (
            "assert_shadow_for",
            "for AssertionError in range(1):\n    pass\nassert True\n",
            "T0041",
        ),
        (
            "assert_shadow_except_as",
            "try:\n    pass\nexcept ValueError as AssertionError:\n    pass\nassert True\n",
            "T0041",
        ),
        (
            "assert_shadow_import_as",
            "import os as AssertionError\nassert True\n",
            "C0001",
        ),
        (
            "assert_shadow_from_import_as",
            "from math import sqrt as AssertionError\nassert True\n",
            "C0001",
        ),
    ] {
        let text = check_error(tag, source);
        assert!(
            text.contains(expected),
            "expected {expected} for {tag}: {text}"
        );
        if expected == "T0041" {
            assert!(
                text.contains(
                    "local name `AssertionError` may not be bound on every path reaching this use"
                ),
                "unexpected diagnostic for {tag}: {text}"
            );
        }
    }
}

/// The `ext`-mode half of the statement: a failing `assert` in a compiled
/// function reaches the host as CPython's own `AssertionError`, with its
/// message or an empty `str`.
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn a_failing_assert_in_an_ext_module_raises_cpython_assertion_error() {
    let dir = ScratchDir::new("1369_ext_assert").expect("scratch");
    build_ext(
        &dir,
        "pycc_assert_mod",
        "def f(n: int) -> int:\n\
         \x20   assert n != 0\n\
         \x20   assert n != 1, \"n is one\"\n\
         \x20   return n\n",
    );
    let run = run_host(
        &dir,
        "import pycc_assert_mod as m\n\
         for n in (0, 1):\n\
         \x20   try:\n\
         \x20       m.f(n)\n\
         \x20       raise RuntimeError('expected an AssertionError')\n\
         \x20   except AssertionError as e:\n\
         \x20       print(type(e).__name__, repr(str(e)))\n\
         print(m.f(7))\n",
    );
    assert_eq!(
        stdout_of(&run),
        "AssertionError ''\nAssertionError 'n is one'\n7\n"
    );
}

/// A failing `assert` inside an extension module's own `try` is caught by
/// the enclosing `except AssertionError as e:` in compiled code, and never
/// reaches the host; the passing path runs through the same function.
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn a_failing_assert_is_caught_by_an_enclosing_try_in_an_ext_module() {
    let dir = ScratchDir::new("1369_ext_assert_try").expect("scratch");
    build_ext(
        &dir,
        "pycc_assert_try_mod",
        "def g(n: int) -> str:\n\
         \x20   try:\n\
         \x20       assert n != 0\n\
         \x20       assert n > 0, \"n must be positive\"\n\
         \x20       return \"ok\"\n\
         \x20   except AssertionError as e:\n\
         \x20       return f\"caught: {e}\"\n",
    );
    let run = run_host(
        &dir,
        "import pycc_assert_try_mod as m\n\
         for n in (1, 0, -1):\n\
         \x20   print(repr(m.g(n)))\n",
    );
    assert_eq!(
        stdout_of(&run),
        "'ok'\n'caught: '\n'caught: n must be positive'\n"
    );
}
