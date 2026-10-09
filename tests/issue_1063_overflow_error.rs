//! End-to-end coverage for Part A of #1038 (#1063): the appended
//! `OverflowError` exception tag and the four converted `**` abort paths.
//!
//! Everything here goes through the public `pycc` CLI, mirroring
//! `tests/issue_739_oserror_hierarchy.rs`'s harness. The point of the
//! whole-program tests is the one thing the `pycc_rt` unit tests cannot show:
//! that HIR tag assignment, `builtin_exception_parent`'s hierarchy arm, MIR
//! handler tag sets and the runtime's name-carrying `PyExceptionObj` all agree
//! that tag 25 is `OverflowError` and that it is a child of `Exception`. That
//! last property is the one with no other guard: `builtin_exception_parent`'s
//! `_ => None` fallthrough would have made the new class a hierarchy root
//! whose `except Exception:` silently stops matching, and `cargo test` was
//! entirely green with that bug present.
//!
//! The `ext`-mode test is `#[ignore]`d for the same reason every test in
//! `tests/issue_1050_ext_tuple.rs` is: it asks an installed CPython 3.13+ to
//! import a built artifact, which is a property of the machine. CI runs it on
//! every Tier-1 `native-build-test` leg through that job's
//! `cargo test --workspace -- --include-ignored`. It is deliberately not where
//! line coverage comes from -- the coverage job runs `llvm-cov` without
//! `--include-ignored`.

use pycc_scratch::ScratchDir;
use std::io::Write;
use std::path::Path;
use std::process::Command;

fn pycc_bin() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_BIN_EXE_pycc"))
}

fn scratch(tag: &str) -> ScratchDir {
    ScratchDir::new(&format!("1063_{tag}")).expect("failed to create scratch dir")
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

/// `except OverflowError:` resolves the appended name to its tag and matches a
/// raise of it, and `except Exception:` matches it too. The second half is the
/// hierarchy assertion: it holds only because `builtin_exception_parent` was
/// taught the name, which is what gives the synthesized `HirClassDef` an
/// `mro` of `["OverflowError", "Exception"]` rather than a lone root.
#[test]
fn overflow_error_is_raisable_and_catchable_by_its_own_name_and_by_exception() {
    let (ok, stdout, stderr) = build_and_run(
        "raise_catch",
        "def main() -> None:\n\
         \x20   try:\n\
         \x20       raise OverflowError(\"by name\")\n\
         \x20   except OverflowError as e:\n\
         \x20       print(e)\n\
         \x20   try:\n\
         \x20       raise OverflowError(\"by base\")\n\
         \x20   except Exception as e:\n\
         \x20       print(e)\n\n\n\
         main()\n",
    );
    assert!(ok, "program failed: {stderr}");
    assert_eq!(stdout, "by name\nby base\n");
}

/// An uncaught `OverflowError` names itself at the top level, which pins the
/// tag-to-name round trip through `PyExceptionObj`'s carried name.
#[test]
fn an_uncaught_overflow_error_reports_its_own_class_name() {
    let (ok, _stdout, stderr) = build_and_run(
        "uncaught",
        "def main() -> None:\n\
         \x20   raise OverflowError(\"too big\")\n\n\n\
         main()\n",
    );
    assert!(!ok, "the program should have exited non-zero");
    assert!(
        stderr.contains("OverflowError: too big"),
        "unexpected stderr: {stderr}"
    );
}

/// The converted `float_pow` overflow arm, observed end to end: the raise is
/// seen at the next D-173 checkpoint inside the `try`, so the handler runs.
///
/// `Pow` deliberately stays in `expression_can_set_exception`'s infallible arm
/// (that classifier is untouched by #1063, and `nbody.py` runs ten `Pow` nodes
/// per benchmark-gated hot-loop iteration), so the raise is *not* checked
/// immediately after the `**` itself -- the `print` below is the checkpoint
/// that observes it. That is why the pre-raise `print` still runs and reports
/// the `0.0` sentinel, and why this test asserts both lines: the deferral is
/// the observable behaviour, not an accident. Making the `**` its own
/// checkpoint is out of scope here and deferred by D-244's 2026-09-13
/// native-mode amendment.
#[test]
fn a_float_power_that_overflows_raises_at_the_next_checkpoint() {
    let (ok, stdout, stderr) = build_and_run(
        "float_overflow",
        "def main() -> None:\n\
         \x20   base: float = 2.0\n\
         \x20   try:\n\
         \x20       result: float = base ** 1024.0\n\
         \x20       print(result)\n\
         \x20   except OverflowError as e:\n\
         \x20       print(e)\n\n\n\
         main()\n",
    );
    assert!(ok, "program failed: {stderr}");
    assert_eq!(
        stdout,
        "0.0\nfloat power overflowed (result too large to represent)\n"
    );
}

/// The converted `float_pow` zero-base arm, which is `ZeroDivisionError` and
/// conformant with CPython down to the sentence. The `return` that follows its
/// raise is what keeps the overflow arm from relabelling it: without that
/// `return`, `powf` would produce `inf` and this would report `OverflowError`.
#[test]
fn zero_raised_to_a_negative_power_raises_zero_division_error_not_overflow_error() {
    let (ok, stdout, stderr) = build_and_run(
        "zero_base",
        "def main() -> None:\n\
         \x20   base: float = 0.0\n\
         \x20   try:\n\
         \x20       result: float = base ** -1.0\n\
         \x20       print(result)\n\
         \x20   except ZeroDivisionError as e:\n\
         \x20       print(e)\n\n\n\
         main()\n",
    );
    assert!(ok, "program failed: {stderr}");
    assert_eq!(stdout, "0.0\n0.0 cannot be raised to a negative power\n");
}

/// The converted `int_pow` negative-exponent arm. `RuntimeError` rather than a
/// CPython-conformant class, because CPython raises nothing at all here: it
/// evaluates `2 ** -1` as `0.5`. The deviation is the pre-existing
/// `pycc_types::numeric_result_type` simplification, now observable as an
/// exception instead of a process abort; #1068 tracks closing it.
#[test]
fn an_integer_power_with_a_negative_exponent_raises_runtime_error() {
    let (ok, stdout, stderr) = build_and_run(
        "int_negative_exp",
        "def main() -> None:\n\
         \x20   base: int = 2\n\
         \x20   exponent: int = -1\n\
         \x20   try:\n\
         \x20       result: int = base ** exponent\n\
         \x20       print(result)\n\
         \x20   except RuntimeError as e:\n\
         \x20       print(e)\n\n\n\
         main()\n",
    );
    assert!(ok, "program failed: {stderr}");
    assert!(
        stdout.contains("negative exponent for `int ** int` is not supported"),
        "unexpected stdout: {stdout}"
    );
    assert!(
        !stdout.contains("pycc_rt: "),
        "the raised message must not carry the panic prefix: {stdout}"
    );
}

/// Appending the name to `BUILTIN_EXCEPTION_CLASSES` also widens the
/// all-or-nothing shadow gate to it: a module whose top level binds
/// `OverflowError` is seeded with none of the builtin exception classes, and
/// an unrelated `except` naming another family member still produces the
/// pre-existing clean diagnostic rather than a compiler-internal panic. This
/// is the `OverflowError` twin of
/// `tests/issue_739_oserror_hierarchy.rs`'s work-item-5 regression test.
#[test]
fn shadowing_overflow_error_still_yields_a_clean_diagnostic_elsewhere() {
    let text = check_error(
        "shadow",
        "class OverflowError:\n    def __init__(self) -> None:\n        pass\n\n\n\
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

/// The `ext`-mode half, against a real CPython. This is the reason the C
/// shim's `case 25:` exists: without it the tag falls through `default:` and
/// the host sees a plain `Exception`. It also pins the rule-3 artifact
/// contract for all four converted sites at once -- each returns `NULL` with
/// the right exception set, rather than aborting the hosting interpreter,
/// which is the whole point of Part A.
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn a_built_ext_module_raises_the_right_cpython_class_from_every_converted_pow_path() {
    let dir = ScratchDir::new("1063_ext").expect("scratch");
    let src = write_fixture(
        &dir,
        "m.py",
        "def overflow(a: float, b: float) -> float:\n    return a ** b\n\n\
         def int_pow(a: int, b: int) -> int:\n    return a ** b\n",
    );
    let build = Command::new(pycc_bin())
        .arg("build")
        .arg(&src)
        .arg("-o")
        .arg(dir.join("pycc_overflow_mod"))
        .arg("--ext")
        .output()
        .expect("pycc should spawn");
    assert!(
        build.status.success(),
        "{}",
        String::from_utf8_lossy(&build.stderr)
    );
    let script = "import pycc_overflow_mod as m\n\
         # The conformant arms keep CPython's own classes.\n\
         try:\n\
         \x20   m.overflow(2.0, 1024.0)\n\
         \x20   raise AssertionError('expected OverflowError')\n\
         except OverflowError as e:\n\
         \x20   assert 'too large' in str(e), str(e)\n\
         try:\n\
         \x20   m.overflow(0.0, -1.0)\n\
         \x20   raise AssertionError('expected ZeroDivisionError')\n\
         except ZeroDivisionError as e:\n\
         \x20   assert str(e) == '0.0 cannot be raised to a negative power', str(e)\n\
         # The two deliberate deviations reach the host as RuntimeError.\n\
         try:\n\
         \x20   m.overflow(-2.0, 3.5)\n\
         \x20   raise AssertionError('expected RuntimeError')\n\
         except RuntimeError as e:\n\
         \x20   assert 'complex result' in str(e), str(e)\n\
         try:\n\
         \x20   m.int_pow(2, -1)\n\
         \x20   raise AssertionError('expected RuntimeError')\n\
         except RuntimeError as e:\n\
         \x20   assert 'negative exponent' in str(e), str(e)\n\
         # A conforming call still returns normally afterwards, so the\n\
         # wrapper left no pending state behind.\n\
         assert m.overflow(2.0, 3.0) == 8.0\n\
         assert m.int_pow(2, 10) == 1024\n";
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
}

/// The residual D-244's 2026-09-13 scope amendment records, pinned through a
/// real `ext` artifact so the amendment's claims stay measured rather than
/// asserted. `Pow` is deliberately not a checkpoint (plan section 2a: ten
/// `Pow` nodes per `tests/fixtures/nbody.py` hot-loop iteration sit under
/// D-084/D-095/D-140's speedup floors), so an export that handles the
/// exception itself sees it at the next enclosing checkpoint, not at the
/// `**`. Two consequences, both strictly better than the process abort they
/// replace, and both closed by #1031 rather than here:
///
/// 1. a statement following the `**` inside the same `try` suite runs first;
/// 2. a second `**` raise before that checkpoint relabels the first.
///
/// A `try` suite's own end *is* a checkpoint, so a `**` written as the last
/// statement of the suite does reach its handler on time -- that third case
/// is pinned too, because it is what bounds the residual.
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn an_ext_export_that_handles_a_pow_raise_itself_observes_it_at_the_next_checkpoint() {
    let dir = ScratchDir::new("1063_ext_residual").expect("scratch");
    let src = write_fixture(
        &dir,
        "m.py",
        "def on_time(a: float, b: float) -> float:\n    \
         try:\n        x = a ** b\n    except OverflowError:\n        return -1.0\n    \
         return 99.0\n\n\
         def one_late(a: float, b: float) -> float:\n    y = 0.0\n    \
         try:\n        x = a ** b\n        y = 1.0\n    except OverflowError:\n        \
         return y\n    return 99.0\n\n\
         def relabelled(a: float) -> float:\n    \
         try:\n        x = a ** 1024.0\n        z = 0.0 ** -1.0\n    \
         except OverflowError:\n        return 1.0\n    \
         except ZeroDivisionError:\n        return 2.0\n    return 99.0\n",
    );
    let build = Command::new(pycc_bin())
        .arg("build")
        .arg(&src)
        .arg("-o")
        .arg(dir.join("pycc_residual_mod"))
        .arg("--ext")
        .output()
        .expect("pycc should spawn");
    assert!(
        build.status.success(),
        "{}",
        String::from_utf8_lossy(&build.stderr)
    );
    let script = "import pycc_residual_mod as m\n\
         # The suite end is a checkpoint, so this handler runs on time.\n\
         assert m.on_time(2.0, 1024.0) == -1.0, m.on_time(2.0, 1024.0)\n\
         # One statement of the suite runs before the handler: y is already 1.0.\n\
         assert m.one_late(2.0, 1024.0) == 1.0, m.one_late(2.0, 1024.0)\n\
         # The later ZeroDivisionError relabels the earlier OverflowError.\n\
         assert m.relabelled(2.0) == 2.0, m.relabelled(2.0)\n\
         # None of the three left pending state behind.\n\
         assert m.on_time(2.0, 3.0) == 99.0\n";
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
}
