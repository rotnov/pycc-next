//! End-to-end coverage for #1292 (Part 2 of #1282): the appended
//! `ImportError` (tag 26) and `ModuleNotFoundError` (tag 27) builtin
//! exception classes.
//!
//! Everything here goes through the public `pycc` CLI, mirroring
//! `tests/issue_1063_overflow_error.rs`'s harness. The whole-program tests
//! show what the `pycc_hir` unit tests cannot: that HIR tag assignment,
//! `builtin_exception_parent`'s two new arms, MIR handler tag sets and the
//! runtime's name-carrying `PyExceptionObj` all agree on CPython's real
//! hierarchy, `ModuleNotFoundError` -> `ImportError` -> `Exception`.
//!
//! Every program raises with a literal message and prints each bound
//! exception at most once: printing a bound exception twice, or once when it
//! was raised with a non-literal message, trips a pre-existing runtime
//! defect in `pycc_rt_str_decref` that this change does not touch.
//!
//! The `ext`-mode test is `#[ignore]`d for the same reason as the one in
//! `tests/issue_1063_overflow_error.rs`: it asks an installed CPython 3.13+
//! to import a built artifact, which is a property of the machine. CI runs it
//! on every Tier-1 `native-build-test` leg through that job's
//! `cargo test --workspace -- --include-ignored`.

use pycc_scratch::ScratchDir;
use std::io::Write;
use std::path::Path;
use std::process::Command;

fn pycc_bin() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_BIN_EXE_pycc"))
}

fn scratch(tag: &str) -> ScratchDir {
    ScratchDir::new(&format!("1292_{tag}")).expect("failed to create scratch dir")
}

fn write_fixture(dir: &Path, name: &str, source: &str) -> std::path::PathBuf {
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

/// Each class is raisable and catchable by its own name, and a
/// `ModuleNotFoundError` is caught by `except ImportError:` and by
/// `except Exception:`. The `ImportError` half holds only because
/// `builtin_exception_parent` parents `ModuleNotFoundError` to `ImportError`
/// rather than straight to `Exception`.
#[test]
fn both_classes_are_catchable_by_their_own_name_by_import_error_and_by_exception() {
    let (ok, stdout, stderr) = build_and_run(
        "raise_catch",
        "def load_plain() -> None:\n\
         \x20   raise ImportError(\"plain\")\n\n\n\
         def load_missing() -> None:\n\
         \x20   raise ModuleNotFoundError(\"No module named 'zz'\")\n\n\n\
         def main() -> None:\n\
         \x20   try:\n\
         \x20       load_plain()\n\
         \x20   except ImportError as e:\n\
         \x20       print(\"own:\", e)\n\
         \x20   try:\n\
         \x20       load_plain()\n\
         \x20   except Exception as e:\n\
         \x20       print(\"base:\", e)\n\
         \x20   try:\n\
         \x20       load_missing()\n\
         \x20   except ModuleNotFoundError as e:\n\
         \x20       print(\"own:\", e)\n\
         \x20   try:\n\
         \x20       load_missing()\n\
         \x20   except ImportError as e:\n\
         \x20       print(\"parent:\", e)\n\
         \x20   try:\n\
         \x20       load_missing()\n\
         \x20   except Exception as e:\n\
         \x20       print(\"root:\", e)\n\n\n\
         main()\n",
    );
    assert!(ok, "program failed: {stderr}");
    assert_eq!(
        stdout,
        "own: plain\nbase: plain\nown: No module named 'zz'\n\
         parent: No module named 'zz'\nroot: No module named 'zz'\n"
    );
}

/// `except ModuleNotFoundError:` does not catch a plain `ImportError`, so the
/// following `except ImportError:` handler takes it.
#[test]
fn a_plain_import_error_skips_a_module_not_found_error_handler() {
    let (ok, stdout, stderr) = build_and_run(
        "ordered",
        "def main() -> None:\n\
         \x20   try:\n\
         \x20       raise ImportError(\"plain\")\n\
         \x20   except ModuleNotFoundError as e:\n\
         \x20       print(\"wrong handler:\", e)\n\
         \x20   except ImportError as e:\n\
         \x20       print(\"second handler:\", e)\n\n\n\
         main()\n",
    );
    assert!(ok, "program failed: {stderr}");
    assert_eq!(stdout, "second handler: plain\n");
}

#[test]
fn issubclass_follows_the_real_hierarchy() {
    let (ok, stdout, stderr) = build_and_run(
        "issubclass",
        "def main() -> None:\n\
         \x20   print(issubclass(ModuleNotFoundError, ImportError), \
         issubclass(ImportError, ModuleNotFoundError))\n\
         \x20   print(issubclass(ImportError, Exception), \
         issubclass(ModuleNotFoundError, Exception))\n\n\n\
         main()\n",
    );
    assert!(ok, "program failed: {stderr}");
    assert_eq!(stdout, "True False\nTrue True\n");
}

/// A user subclass of `ImportError` is newly admitted: before #1292 it was
/// `C0001` "inherits from unknown class `ImportError`".
#[test]
fn a_user_import_error_subclass_is_raisable_and_caught_by_import_error() {
    let (ok, stdout, stderr) = build_and_run(
        "user_subclass",
        "class PluginMissing(ImportError):\n    pass\n\n\n\
         def load() -> None:\n\
         \x20   raise PluginMissing(\"plugin gone\")\n\n\n\
         def main() -> None:\n\
         \x20   try:\n\
         \x20       load()\n\
         \x20   except ImportError as e:\n\
         \x20       print(\"caught:\", e)\n\n\n\
         main()\n",
    );
    assert!(ok, "program failed: {stderr}");
    assert_eq!(stdout, "caught: plugin gone\n");
}

/// An uncaught `ModuleNotFoundError` names itself at the top level, which
/// pins the tag-to-name round trip through `PyExceptionObj`'s carried name.
#[test]
fn an_uncaught_module_not_found_error_reports_its_own_class_name() {
    let (ok, _stdout, stderr) = build_and_run(
        "uncaught",
        "def main() -> None:\n\
         \x20   raise ModuleNotFoundError(\"No module named 'y'\")\n\n\n\
         main()\n",
    );
    assert!(!ok, "the program should have exited non-zero");
    assert!(
        stderr.contains("ModuleNotFoundError: No module named 'y'"),
        "unexpected stderr: {stderr}"
    );
}

/// The `ModuleNotFoundError` twin of `tests/issue_1063_overflow_error.rs`'s
/// shadow-gate regression test: binding the new name at top level withholds
/// seeding of every builtin, and an unrelated `except` naming a class past
/// the flat seven still produces a clean `T0021` rather than a panic.
#[test]
fn shadowing_module_not_found_error_still_yields_a_clean_diagnostic_elsewhere() {
    let text = check_error(
        "shadow",
        "class ModuleNotFoundError:\n    def __init__(self) -> None:\n        pass\n\n\n\
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

/// The behaviour change the all-or-nothing shadow gate brings with the two
/// new names (documented in `docs/RUNTIME.md`'s shadow-gate paragraph): a
/// module declaring its own `class ImportError(Exception)` compiled before
/// #1292, and now withholds seeding, so its own base `Exception` is unknown
/// -- exactly as a user `class OverflowError(Exception)` already did.
#[test]
fn a_user_class_named_import_error_now_withholds_seeding() {
    let text = check_error(
        "user_import_error",
        "class ImportError(Exception):\n    pass\n\n\n\
         def main() -> None:\n\
         \x20   try:\n\
         \x20       raise ImportError(\"m\")\n\
         \x20   except ImportError:\n\
         \x20       print(\"caught\")\n\n\n\
         main()\n",
    );
    assert!(text.contains("C0001"), "unexpected diagnostic: {text}");
    assert!(
        text.contains("class `ImportError` inherits from unknown class `Exception`"),
        "unexpected diagnostic: {text}"
    );
}

/// `ImportError`'s `name`/`path` keyword arguments are out of scope: a
/// keyword argument is rejected with `C0001` rather than silently dropped.
#[test]
fn an_import_error_keyword_argument_is_rejected() {
    let text = check_error(
        "keyword",
        "def main() -> None:\n\
         \x20   raise ImportError(\"m\", name=\"x\")\n\n\n\
         main()\n",
    );
    assert!(text.contains("C0001"), "unexpected diagnostic: {text}");
    assert!(
        text.contains("keyword call arguments are not supported yet"),
        "unexpected diagnostic: {text}"
    );
}

/// The `ext`-mode half, against a real CPython. This is the reason the C
/// shim's `case 26:`/`case 27:` exist: without them both tags fall through
/// `default:` and the host sees a plain `Exception`. A user `ImportError`
/// subclass reaches the host with `ImportError` as its base.
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn a_built_ext_module_raises_the_right_cpython_import_error_classes() {
    let dir = ScratchDir::new("1292_ext").expect("scratch");
    let src = write_fixture(
        &dir,
        "m.py",
        "class PluginMissing(ImportError):\n    pass\n\n\n\
         def f(n: int) -> int:\n\
         \x20   if n == 0:\n\
         \x20       raise ImportError(\"plain\")\n\
         \x20   if n == 1:\n\
         \x20       raise ModuleNotFoundError(\"No module named 'zz'\")\n\
         \x20   if n == 2:\n\
         \x20       raise PluginMissing(\"plug\")\n\
         \x20   return n\n",
    );
    let build = Command::new(pycc_bin())
        .arg("build")
        .arg(&src)
        .arg("-o")
        .arg(dir.join("pycc_import_error_mod"))
        .arg("--ext")
        .output()
        .expect("pycc should spawn");
    assert!(
        build.status.success(),
        "{}",
        String::from_utf8_lossy(&build.stderr)
    );
    let script = "import pycc_import_error_mod as m\n\
         for n in (0, 1, 2):\n\
         \x20   try:\n\
         \x20       m.f(n)\n\
         \x20       raise AssertionError('expected an ImportError')\n\
         \x20   except ImportError as e:\n\
         \x20       print(type(e).__name__, type(e).__mro__[1].__name__, \
         repr(str(e)), isinstance(e, ImportError))\n\
         assert m.f(7) == 7\n";
    let run = Command::new(std::env::var_os("PYCC_PYTHON").unwrap_or_else(|| "python3".into()))
        .arg("-c")
        .arg(script)
        .current_dir(&*dir)
        .output()
        .expect("python3 should spawn");
    assert!(
        run.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&run.stdout),
        String::from_utf8_lossy(&run.stderr)
    );
    // The host interpreter's text-mode stdout translates `\n` to `\r\n` on
    // Windows, the same normalization `tests/issue_1114_numpy_oracle.rs` applies.
    assert_eq!(
        String::from_utf8_lossy(&run.stdout).replace("\r\n", "\n"),
        "ImportError Exception 'plain' True\n\
         ModuleNotFoundError ImportError \"No module named 'zz'\" True\n\
         PluginMissing ImportError 'plug' True\n"
    );
}
