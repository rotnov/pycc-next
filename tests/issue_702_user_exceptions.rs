//! End-to-end coverage for raising and catching user-defined exception
//! classes (Part 2 of #541, issue #702, D-189).
//!
//! Everything here goes through the public `pycc` CLI: the point is that the
//! whole pipeline -- HIR tag assignment, MIR handler tag sets, the type
//! checker's raisability gate, codegen's OR-chain, and the runtime's
//! name-carrying `PyExceptionObj` -- agrees on one numbering.

use pycc_scratch::ScratchDir;
use std::io::Write;
use std::process::Command;

fn pycc_bin() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_BIN_EXE_pycc"))
}

fn scratch(tag: &str) -> ScratchDir {
    ScratchDir::new(&format!("702_{tag}")).expect("failed to create scratch dir")
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

const HIERARCHY: &str = "class AppError(Exception):\n    pass\n\n\n\
                         class DatabaseError(AppError):\n    pass\n\n\n\
                         class ConfigError(AppError):\n    pass\n\n\n";

#[test]
fn a_handler_selects_the_most_specific_matching_user_class() {
    let (ok, stdout, stderr) = build_and_run(
        "dispatch",
        &format!(
            "{HIERARCHY}\
             def load(kind: str) -> None:\n\
             \x20   if kind == \"db\":\n        raise DatabaseError(\"refused\")\n\
             \x20   if kind == \"cfg\":\n        raise ConfigError(\"missing\")\n\
             \x20   raise ValueError(\"unknown\")\n\n\n\
             def attempt(kind: str) -> None:\n\
             \x20   try:\n        load(kind)\n\
             \x20   except DatabaseError:\n        print(\"database\")\n\
             \x20   except AppError:\n        print(\"app\")\n\
             \x20   except ValueError:\n        print(\"value\")\n\n\n\
             def main() -> None:\n\
             \x20   attempt(\"db\")\n    attempt(\"cfg\")\n    attempt(\"other\")\n\n\n\
             main()\n"
        ),
    );
    assert!(ok, "program failed: {stderr}");
    assert_eq!(stdout, "database\napp\nvalue\n");
}

#[test]
fn a_base_class_handler_catches_every_subclass() {
    let (ok, stdout, stderr) = build_and_run(
        "base_catch",
        &format!(
            "{HIERARCHY}\
             def main() -> None:\n\
             \x20   try:\n        raise ConfigError(\"missing\")\n\
             \x20   except AppError:\n        print(\"app\")\n\n\n\
             main()\n"
        ),
    );
    assert!(ok, "program failed: {stderr}");
    assert_eq!(stdout, "app\n");
}

#[test]
fn an_exception_handler_catches_a_user_class() {
    let (ok, stdout, stderr) = build_and_run(
        "root_catch",
        &format!(
            "{HIERARCHY}\
             def main() -> None:\n\
             \x20   try:\n        raise DatabaseError(\"refused\")\n\
             \x20   except Exception:\n        print(\"root\")\n\n\n\
             main()\n"
        ),
    );
    assert!(ok, "program failed: {stderr}");
    assert_eq!(stdout, "root\n");
}

#[test]
fn a_subclass_handler_does_not_catch_its_sibling() {
    let (ok, stdout, stderr) = build_and_run(
        "sibling",
        &format!(
            "{HIERARCHY}\
             def main() -> None:\n\
             \x20   try:\n        raise ConfigError(\"missing\")\n\
             \x20   except DatabaseError:\n        print(\"database\")\n\
             \x20   except Exception:\n        print(\"other\")\n\n\n\
             main()\n"
        ),
    );
    assert!(ok, "program failed: {stderr}");
    assert_eq!(stdout, "other\n");
}

#[test]
fn a_user_exception_propagates_across_a_call_boundary_through_finally() {
    let (ok, stdout, stderr) = build_and_run(
        "propagate",
        &format!(
            "{HIERARCHY}\
             def inner() -> None:\n\
             \x20   try:\n        raise DatabaseError(\"refused\")\n\
             \x20   finally:\n        print(\"inner finally\")\n\n\n\
             def main() -> None:\n\
             \x20   try:\n        inner()\n\
             \x20   except AppError:\n        print(\"outer\")\n\n\n\
             main()\n"
        ),
    );
    assert!(ok, "program failed: {stderr}");
    assert_eq!(stdout, "inner finally\nouter\n");
}

#[test]
fn a_bare_raise_reraises_a_user_exception_to_an_enclosing_try() {
    let (ok, stdout, stderr) = build_and_run(
        "reraise",
        &format!(
            "{HIERARCHY}\
             def main() -> None:\n\
             \x20   try:\n\
             \x20       try:\n            raise DatabaseError(\"refused\")\n\
             \x20       except AppError:\n            print(\"inner\")\n            raise\n\
             \x20   except DatabaseError:\n        print(\"outer\")\n\n\n\
             main()\n"
        ),
    );
    assert!(ok, "program failed: {stderr}");
    assert_eq!(stdout, "inner\nouter\n");
}

#[test]
fn an_uncaught_user_exception_prints_its_own_class_name() {
    // The runtime used to map tag -> name with a `match` over the seven
    // builtin tags, which would have printed `Exception` for every user
    // class. The name now travels on `PyExceptionObj`.
    let (ok, stdout, stderr) = build_and_run(
        "uncaught",
        &format!(
            "{HIERARCHY}def main() -> None:\n    raise ConfigError(\"missing\")\n\n\nmain()\n"
        ),
    );
    assert!(!ok, "an uncaught exception must exit non-zero");
    assert_eq!(stdout, "");
    // #707 added a traceback-frame header naming the enclosing function.
    assert_eq!(
        stderr,
        "Traceback (most recent call last):\n  File \"<compiled>\", in main\nConfigError: missing\n"
    );
}

#[test]
fn an_uncaught_builtin_exception_still_prints_its_own_class_name() {
    let (ok, stdout, stderr) = build_and_run(
        "uncaught_builtin",
        "def main() -> None:\n    raise KeyError(\"nope\")\n\n\nmain()\n",
    );
    assert!(!ok, "an uncaught exception must exit non-zero");
    assert_eq!(stdout, "");
    // #707 added a traceback-frame header naming the enclosing function.
    assert_eq!(
        stderr,
        "Traceback (most recent call last):\n  File \"<compiled>\", in main\nKeyError: nope\n"
    );
}

#[test]
fn raise_from_accepts_a_user_exception_cause() {
    let (ok, stdout, stderr) = build_and_run(
        "cause",
        &format!(
            "{HIERARCHY}\
             def main() -> None:\n\
             \x20   try:\n\
             \x20       raise DatabaseError(\"refused\") from ConfigError(\"missing\")\n\
             \x20   except AppError:\n        print(\"caught\")\n\n\n\
             main()\n"
        ),
    );
    assert!(ok, "program failed: {stderr}");
    assert_eq!(stdout, "caught\n");
}

#[test]
fn raising_a_bound_user_exception_value_is_rejected() {
    // #714 closed the route this test used to reach a bound
    // `Ty::Instance("AppError")` value (`e = AppError("x")`) at an even
    // earlier point -- that instantiation is now itself a `C0001`, since
    // codegen has no real `__init__` body for a class that only inherits
    // the synthetic `Exception.__init__` placeholder -- so the bound value
    // here comes from a parameter annotation instead, which type-checks a
    // `Ty::Instance("AppError")` binding without instantiating anything.
    let text = check_error(
        "bound",
        &format!("{HIERARCHY}def main(e: AppError) -> None:\n    raise e\n"),
    );
    assert!(text.contains("T0021"), "unexpected diagnostic: {text}");
    assert!(
        text.contains("can only raise exception instances"),
        "unexpected diagnostic: {text}"
    );
}

#[test]
fn binding_a_caught_user_exception_is_rejected() {
    let text = check_error(
        "as_binding",
        &format!(
            "{HIERARCHY}def main() -> None:\n    try:\n        raise AppError(\"x\")\n\
             \x20   except AppError as exc:\n        print(\"caught\")\n"
        ),
    );
    assert!(text.contains("C0001"), "unexpected diagnostic: {text}");
    assert!(
        text.contains("with `as` is not supported yet"),
        "unexpected diagnostic: {text}"
    );
}

#[test]
fn an_exception_class_with_its_own_constructor_is_rejected() {
    let text = check_error(
        "own_init",
        "class AppError(Exception):\n\
         \x20   def __init__(self, code: int) -> None:\n        self.code = code\n\n\n\
         def main() -> None:\n    raise AppError(3)\n",
    );
    assert!(text.contains("C0001"), "unexpected diagnostic: {text}");
    assert!(
        text.contains("declares or inherits an `__init__` other than"),
        "unexpected diagnostic: {text}"
    );
}

#[test]
fn a_non_string_constructor_argument_reports_the_argument_diagnostic() {
    let text = check_error(
        "bad_arg",
        &format!("{HIERARCHY}def main() -> None:\n    raise AppError(3)\n"),
    );
    assert!(text.contains("T0021"), "unexpected diagnostic: {text}");
    assert!(
        text.contains("expects `str`"),
        "unexpected diagnostic: {text}"
    );
}

#[test]
fn a_module_with_more_user_exception_classes_than_tags_is_rejected() {
    let mut source = String::new();
    for index in 0..250 {
        source.push_str(&format!("class E{index}(Exception):\n    pass\n\n\n"));
    }
    source.push_str("def main() -> None:\n    raise E0(\"x\")\n");
    let text = check_error("too_many", &source);
    assert!(text.contains("C0001"), "unexpected diagnostic: {text}");
    // Part 3 of #382 (#542, PEP 654, D-202) added `ExceptionGroup` and
    // `BaseExceptionGroup` to `BUILTIN_EXCEPTION_CLASSES`, shrinking the
    // remaining per-module user-exception tag budget from 233 to 231; Part A
    // of #1038 (#1063) appended `OverflowError`, shrinking it again to 230;
    // #1292 appended `ImportError`/`ModuleNotFoundError`, shrinking it to 228;
    // #1316 reserved tag 255 for a bridged non-`Exception` `BaseException`,
    // shrinking it to 227; #1369 appended `AssertionError`, shrinking it to
    // 226; #1388 appended `AttributeError`, shrinking it to 225.
    assert!(
        text.contains("at most 225"),
        "unexpected diagnostic: {text}"
    );
}

/// A user class rooted at a builtin *other* than `Exception` still gets a tag,
/// and a handler for that builtin widens to it (#702). Verified byte-identical
/// to CPython 3.14.
#[test]
fn a_class_derived_from_a_non_exception_builtin_is_raisable_and_caught_by_its_base() {
    let (ok, stdout, stderr) = build_and_run(
        "value_error_root",
        "class ParseError(ValueError):\n    pass\n\n\n\
         def main() -> None:\n\
         \x20   try:\n        raise ParseError(\"bad token\")\n\
         \x20   except ValueError:\n        print(\"caught via ValueError\")\n\
         \x20   try:\n        raise ParseError(\"again\")\n\
         \x20   except ParseError:\n        print(\"caught via ParseError\")\n\n\n\
         main()\n",
    );
    assert!(ok, "program failed: {stderr}");
    assert_eq!(stdout, "caught via ValueError\ncaught via ParseError\n");
}

/// A generic class may not inherit at all (#432), so `class E[T](Exception)`
/// never reaches monomorphization -- which is why the monomorphized
/// `HirClassDef` may hardcode `exception_type_tag: None`. This locks that
/// rejection so relaxing #432 cannot silently mint a tagless exception class.
#[test]
fn a_generic_class_cannot_inherit_from_an_exception_class() {
    let text = check_error(
        "generic_exception",
        "class MyError[T](Exception):\n    pass\n\n\n\
         def main() -> None:\n    raise MyError[int](\"boom\")\n",
    );
    assert!(
        text.contains("generic class `MyError` with base classes"),
        "expected the #432 rejection, got: {text}"
    );
}
