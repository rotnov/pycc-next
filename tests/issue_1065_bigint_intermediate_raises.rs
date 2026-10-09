//! End-to-end coverage for Part C of #1038
//! ([#1065](https://github.com/rotnov/pycc/issues/1065)): the seven
//! bigint-intermediate abort paths that ran through
//! `int_encoding::require_inline_int`.
//!
//! Parts A (#1063) and B (#1064) converted the `**` and list/set/float sites;
//! this part converts the last family. Each site now raises a catchable
//! `OverflowError` (D-173) and returns a sentinel that is a valid value of its
//! own return type -- `tag_smallint(0)` for the four encoded-`int` returns,
//! a plain `0` ordering for `int_cmp`, `0.0` for `int_to_float`, and no
//! sentinel at all for `pycc_rt_int_set_add`, which returns `()`. The set
//! site has no public-CLI test here: codegen validates a `s.add(v)` element
//! through `pycc_rt_int_untag_checked` first, so a compiled program aborts at
//! that separate, out-of-scope D-141 boundary before reaching it.
//!
//! Per the issue's own measurement this needed **zero codegen change**: an
//! `ext` wrapper already emits its pending-exception check ahead of every
//! return arm, so no packer ever reads a sentinel.
//!
//! The hazard this file exists to pin: `raise_builtin` installs
//! unconditionally and does not check for an already-pending exception, while
//! `int_floordiv` and `int_floormod` test their divisor for `0` immediately
//! after decoding it, and `int_pow` tests its exponent for `< 0`. Had a bigint
//! operand fallen through as a decoded `0`, `ZeroDivisionError` (or, for
//! `**`, a silently-computed `1`) would have displaced the `OverflowError`.
//! Every converted site returns before reaching its own later raise instead.
//!
//! The `ext`-mode test is `#[ignore]`d for the same reason every test in
//! `tests/issue_1050_ext_tuple.rs` is: it asks an installed CPython 3.13+ to
//! import a built artifact, which is a property of the machine, and the
//! coverage job runs `llvm-cov` without `--include-ignored`.

use pycc_scratch::ScratchDir;
use std::io::Write;
use std::path::Path;
use std::process::Command;

/// `2^62`, the smallest magnitude that does not round-trip through D-061's
/// tagged 63-bit encoding -- the same literal `tests/issue_148_*` uses.
const OVERSIZED: &str = "4611686018427387904";

fn pycc_bin() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_BIN_EXE_pycc"))
}

fn write_fixture(dir: &Path, name: &str, source: &str) -> std::path::PathBuf {
    let path = dir.join(name);
    let mut file = std::fs::File::create(&path).unwrap();
    file.write_all(source.as_bytes()).unwrap();
    path
}

/// `(exit_success, stdout, stderr)` of the built program.
fn build_and_run(tag: &str, source: &str) -> (bool, String, String) {
    let dir = ScratchDir::new(&format!("1065_{tag}")).expect("failed to create scratch dir");
    let src = write_fixture(&dir, &format!("{tag}.py"), source);
    let out = dir.join(format!("{tag}_bin"));
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

/// Five of the six compiler-reachable converted sites are catchable with an
/// ordinary `try`/`except OverflowError`, in one program; the sixth,
/// `int_cmp`, is reached by `no_converted_message_carries_the_pycc_rt_panic_prefix`
/// below. Every one of them aborted the process with `SIGABRT` before this
/// change, so none of these `except` suites could have run at all.
///
/// `set.add()` is deliberately *absent*: codegen emits a
/// `pycc_rt_int_untag_checked` element validation (`set_validate_added`,
/// `crates/pycc_codegen/src/lib.rs`) ahead of every `pycc_rt_int_set_add`
/// call, so a bigint element aborts at that separate, still-open D-141
/// boundary before the converted guard is ever reached. The guard is
/// defense-in-depth for a direct ABI caller and is covered by
/// `a_bigint_value_added_to_an_int_set_raises_and_leaves_the_set_intact` in
/// `crates/pycc_rt/src/lib.rs`.
#[test]
fn every_converted_arithmetic_bigint_path_is_catchable_as_overflow_error() {
    let (ok, stdout, stderr) = build_and_run(
        "catchable",
        &format!(
            "big = {OVERSIZED} + 0\n\
             try:\n\
             \x20   a = big * 2\n\
             except OverflowError as e:\n\
             \x20   print(e)\n\
             try:\n\
             \x20   b = big // 2\n\
             except OverflowError as e:\n\
             \x20   print(e)\n\
             try:\n\
             \x20   c = big % 2\n\
             except OverflowError as e:\n\
             \x20   print(e)\n\
             try:\n\
             \x20   d = big ** 2\n\
             except OverflowError as e:\n\
             \x20   print(e)\n\
             try:\n\
             \x20   e2 = big / 2\n\
             except OverflowError as e:\n\
             \x20   print(e)\n\
             print(\"END\")\n"
        ),
    );
    assert!(ok, "program failed: {stderr}");
    assert_eq!(
        stdout,
        "multiplying a bigint-valued `int` is not supported yet\n\
         dividing a bigint-valued `int` is not supported yet\n\
         computing the modulo of a bigint-valued `int` is not supported yet\n\
         exponentiating a bigint-valued `int` is not supported yet\n\
         converting a bigint-valued `int` is not supported yet\n\
         END\n",
        "stderr: {stderr}"
    );
}

/// The named #1065 hazard, closed and pinned: a bigint *divisor* must surface
/// `OverflowError`, never the `ZeroDivisionError` its `b == 0` arm would have
/// raised over it. `%` carries the identical arm, and `**`'s `exp < 0` arm is
/// the same shape with a `RuntimeError`.
///
/// The `except` order is deliberate: `ZeroDivisionError` and `RuntimeError`
/// are listed *first*, so a regression that lets the later raise win prints
/// the wrong line rather than falling through to the `OverflowError` handler
/// by accident.
#[test]
fn a_bigint_divisor_surfaces_overflow_error_and_not_zero_division() {
    let (ok, stdout, stderr) = build_and_run(
        "divisor_collision",
        &format!(
            "big = {OVERSIZED} + 0\n\
             try:\n\
             \x20   a = 6 // big\n\
             except ZeroDivisionError as e:\n\
             \x20   print(\"WRONG floordiv\")\n\
             except OverflowError as e:\n\
             \x20   print(e)\n\
             try:\n\
             \x20   b = 6 % big\n\
             except ZeroDivisionError as e:\n\
             \x20   print(\"WRONG floormod\")\n\
             except OverflowError as e:\n\
             \x20   print(e)\n\
             try:\n\
             \x20   c = 2 ** big\n\
             except RuntimeError as e:\n\
             \x20   print(\"WRONG pow\")\n\
             except OverflowError as e:\n\
             \x20   print(e)\n"
        ),
    );
    assert!(ok, "program failed: {stderr}");
    assert_eq!(
        stdout,
        "dividing a bigint-valued `int` is not supported yet\n\
         computing the modulo of a bigint-valued `int` is not supported yet\n\
         exponentiating a bigint-valued `int` is not supported yet\n",
        "stderr: {stderr}"
    );
}

/// A real zero divisor still raises `ZeroDivisionError`: the early return
/// added above must not shadow the arm it precedes.
#[test]
fn an_ordinary_zero_divisor_still_raises_zero_division_error() {
    let (ok, stdout, stderr) = build_and_run(
        "zero_divisor",
        "z = 1 - 1\n\
         try:\n\
         \x20   a = 6 // z\n\
         except ZeroDivisionError as e:\n\
         \x20   print(e)\n\
         try:\n\
         \x20   b = 6 % z\n\
         except ZeroDivisionError as e:\n\
         \x20   print(e)\n\
         neg = 0 - 1\n\
         try:\n\
         \x20   c = 2 ** neg\n\
         except RuntimeError as e:\n\
         \x20   print(\"negative exponent\")\n",
    );
    assert!(ok, "program failed: {stderr}");
    assert_eq!(
        stdout,
        "integer division by zero\n\
         integer modulo by zero\n\
         negative exponent\n",
        "stderr: {stderr}"
    );
}

/// None of the seven messages carries Part A's old `pycc_rt: ` panic prefix:
/// a raised exception's message is user-facing Python text, not a runtime
/// diagnostic. `int_cmp`'s message is checked here rather than above because
/// a comparison is most naturally written as a statement condition.
#[test]
fn no_converted_message_carries_the_pycc_rt_panic_prefix() {
    let (ok, stdout, stderr) = build_and_run(
        "no_prefix",
        &format!(
            "big = {OVERSIZED} + 0\n\
             try:\n\
             \x20   flag = big > 0\n\
             except OverflowError as e:\n\
             \x20   print(e)\n\
             try:\n\
             \x20   x = big * 2\n\
             except OverflowError as e:\n\
             \x20   print(e)\n"
        ),
    );
    assert!(ok, "program failed: {stderr}");
    assert!(!stdout.contains("pycc_rt: "), "unexpected prefix: {stdout}");
    assert_eq!(
        stdout,
        "comparing a bigint-valued `int` is not supported yet\n\
         multiplying a bigint-valued `int` is not supported yet\n"
    );
}

/// An uncaught raise from a converted site reports the CPython class name and
/// exits non-zero, rather than aborting with `SIGABRT` as it used to.
#[test]
fn an_uncaught_converted_raise_names_its_class_and_exits_non_zero() {
    let (ok, _stdout, stderr) = build_and_run(
        "uncaught",
        &format!("big = {OVERSIZED} + 0\nprint(big // 2)\n"),
    );
    assert!(!ok, "the program should have exited non-zero");
    assert!(
        stderr.contains("OverflowError: dividing a bigint-valued `int` is not supported yet"),
        "unexpected stderr: {stderr}"
    );
}

/// The accepted D-173 sentinel residual, measured rather than predicted.
/// `pycc_codegen::exception::expression_can_set_exception` classifies `Div`,
/// `FloorDiv` and `Mod` as checkpoints but **not** `Mul`, `Pow` or `Compare`,
/// so `print(big * 2)` writes `*`'s `tag_smallint(0)` sentinel before the
/// enclosing statement's own checkpoint reports the pending `OverflowError`,
/// while `print(big // 2)` prints nothing at all.
///
/// This is the same already-accepted class as D-244's 2026-09-13 amendments
/// (Part A's `**` sentinel, Part B's float sentinel), and is deliberately not
/// guarded here: widening the classifier is #1031's work, and the pre-change
/// behavior -- a `SIGABRT` mid-`print` -- was strictly worse.
#[test]
fn the_multiply_sentinel_is_printed_but_the_floordiv_sentinel_is_not() {
    let (ok, stdout, stderr) = build_and_run(
        "sentinel_mul",
        &format!("big = {OVERSIZED} + 0\nprint(big * 2)\n"),
    );
    assert!(!ok, "the program should have exited non-zero");
    assert_eq!(stdout, "0\n", "unexpected stdout: {stdout:?}");
    assert!(stderr.contains("OverflowError: multiplying"), "{stderr}");

    let (ok, stdout, stderr) = build_and_run(
        "sentinel_floordiv",
        &format!("big = {OVERSIZED} + 0\nprint(big // 2)\n"),
    );
    assert!(!ok, "the program should have exited non-zero");
    assert_eq!(stdout, "", "unexpected stdout: {stdout:?}");
    assert!(stderr.contains("OverflowError: dividing"), "{stderr}");
}

/// The same raises seen from a CPython host through an `ext`-mode artifact
/// (D-244), which is the mode where being catchable actually matters: the
/// host interpreter, not pycc, owns the process, so a `SIGABRT` there took the
/// whole interpreter down. This is the reproduction recipe from the parent
/// plan's section 11, which measured exit `-6` before this change.
///
/// Every bigint here is produced *inside* the compiled function, never passed
/// in: the generated wrapper's own argument unpacker rejects a bigint argument
/// with its own `OverflowError` (the #1040 boundary) before the body runs, so
/// `m.divide(6, 2 ** 62)` would never reach `int_floordiv` at all.
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn a_bigint_intermediate_reaches_a_cpython_host_as_a_catchable_overflow_error() {
    let dir = ScratchDir::new("1065_ext").expect("scratch");
    let src = write_fixture(
        &dir,
        "m.py",
        "def square_then_zero(x: int) -> int:\n\
         \x20   return (x * x) * 0\n\n\
         def square_then_divide(x: int, d: int) -> int:\n\
         \x20   return (x * x) // d\n\n\
         def divide(a: int, b: int) -> int:\n\
         \x20   return a // b\n\n\
         def ok(x: int) -> int:\n\
         \x20   return x + 1\n",
    );
    let build = Command::new(pycc_bin())
        .arg("build")
        .arg(&src)
        .arg("-o")
        .arg(dir.join("pycc_1065_mod"))
        .arg("--ext")
        .output()
        .expect("pycc should spawn");
    assert!(
        build.status.success(),
        "{}",
        String::from_utf8_lossy(&build.stderr)
    );
    let script = "import pycc_1065_mod as m\n\
         try:\n\
         \x20   m.square_then_zero((2 ** 62) - 1)\n\
         \x20   raise AssertionError('expected OverflowError')\n\
         except OverflowError as e:\n\
         \x20   assert 'multiplying' in str(e), str(e)\n\
         try:\n\
         \x20   m.square_then_divide((2 ** 62) - 1, 2)\n\
         \x20   raise AssertionError('expected OverflowError')\n\
         except OverflowError as e:\n\
         \x20   assert 'dividing' in str(e), str(e)\n\
         try:\n\
         \x20   m.divide(6, 0)\n\
         \x20   raise AssertionError('expected ZeroDivisionError')\n\
         except ZeroDivisionError as e:\n\
         \x20   assert str(e) == 'integer division by zero', str(e)\n\
         # A conforming call still returns normally afterwards, so none of the\n\
         # wrappers left pending state behind.\n\
         assert m.ok(1) == 2\n";
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
