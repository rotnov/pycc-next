//! Part 3 of #1026 (PR 3a of #1082): `len` and truth testing on a CPython
//! object.
//!
//! `tests/issue_1080_foreign_object.rs` owns what a foreign `import` binds
//! and the refusal table around it; `tests/issue_1081_foreign_method_call.rs`
//! owns the method call. This file owns the two operations PR 3a adds: that
//! each of the five condition-position shapes now type-checks (the ten
//! `reject_object_condition` sites are gone), that `len(o)` type-checks to
//! `int`, that PR 2a's positional bound is inherited unchanged by both, and
//! -- in the `#[ignore]`d hosted tests at the bottom -- that a real CPython
//! interpreter observes the right length, the right truth value, and the
//! right exception on each of the two failing calls (`PyObject_Size` and
//! `PyObject_IsTrue`).
//!
//! The hosted tests contribute no line coverage (CI's coverage job runs
//! `llvm-cov` without `--include-ignored`); they are run by the Tier-1
//! `native-build-test` leg's `cargo test --workspace -- --include-ignored`.
//! Every new Rust line is covered by the non-ignored tests here and by the
//! unit tests in `crates/pycc_codegen/src/foreign_len.rs`, which assert the
//! emitted IR for all five condition sites and both failure edges without
//! needing a CPython on `PATH`.
//!
//! One further hosted test, `a_shadowing_len_definition_still_builds_in_the_host`,
//! pins that a module-level `def len` does not divert `len(<object>)` away
//! from the `MirExpr::ObjLen` lowering and into a codegen panic. Its
//! non-ignored counterpart -- the line coverage for that fix -- is
//! `pycc_mir`'s `len_of_a_foreign_object_ignores_a_module_level_len_definition`.

use pycc_scratch::ScratchDir;
use std::path::Path;
use std::process::{Command, Output};

fn pycc() -> Command {
    Command::new(env!("CARGO_BIN_EXE_pycc"))
}

fn stdout_of(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn stderr_of(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

fn source(dir: &Path, body: &str) -> std::path::PathBuf {
    let src = dir.join("m.py");
    std::fs::write(&src, body).expect("write the fixture source");
    src
}

fn check(dir: &Path, body: &str) -> Output {
    pycc()
        .arg("check")
        .arg(source(dir, body))
        .output()
        .expect("pycc should spawn")
}

/// The five condition-position shapes PR 3a admits.
///
/// Exactly the five `reject_object_condition` refused in a module body,
/// and the same five it refused again inside a function body -- ten call
/// sites, five shapes. The comprehension rows spell their iterable as
/// `range(3)` deliberately, so the filter is the comprehension's only
/// CPython-object operation: `[i for i in gc if gc]` is `I0404` for its
/// bare-name `object` iterable long before the guard is reached, and an
/// object iterable expression (Part 1 of #1255) would add a second object
/// operation, so neither would test the guard alone.
const CONDITION_SHAPES: [(&str, &str); 5] = [
    ("if", "if gc:\n    print(1)\n"),
    ("while", "while gc:\n    print(1)\n"),
    (
        "list comprehension guard",
        "xs = [i for i in range(3) if gc]\n",
    ),
    (
        "set comprehension guard",
        "ys = {i for i in range(3) if gc}\n",
    ),
    (
        "dict comprehension guard",
        "zs = {\"k\": i for i in range(3) if gc}\n",
    ),
];

/// Every condition-position shape type-checks against a CPython object.
///
/// The front-end half of PR 3a: `pycc_types` placed no constraint on a
/// truth test to begin with, and the `Ty::Object` exception it carved out
/// existed only because `pycc_codegen`'s `truthy` had no object arm. The
/// arm exists now (`crates/pycc_codegen/src/foreign_len.rs`), so the
/// exception is gone and the general rule applies again.
#[test]
fn every_condition_position_shape_is_admitted() {
    let dir = ScratchDir::new("foreign_truthy_admitted").expect("scratch");
    for (label, shape) in CONDITION_SHAPES {
        let out = check(&dir, &format!("import gc\n\n{shape}"));
        assert!(
            out.status.success(),
            "{label}: {}{}",
            stdout_of(&out),
            stderr_of(&out)
        );
    }
}

/// `len(o)` type-checks, and its result is an ordinary `int`.
///
/// The second assertion is what makes the first useful: binding the result
/// to a name and printing it both go through the `int` paths, so `len` on a
/// foreign object is not a value that has to stay anonymous the way the
/// object itself does (`check_assignment` still refuses *that*).
#[test]
fn len_of_a_cpython_object_is_admitted_as_an_int() {
    let dir = ScratchDir::new("foreign_len_admitted").expect("scratch");
    for body in [
        "import gc\n\nprint(len(gc))\n",
        "import gc\n\nn = len(gc)\nprint(n)\n",
        "import gc\n\nn = len(gc) + 1\nprint(n)\n",
    ] {
        let out = check(&dir, body);
        assert!(
            out.status.success(),
            "{body}: {}{}",
            stdout_of(&out),
            stderr_of(&out)
        );
    }
}

/// `not o` is admitted since Part 10 of #1371.
///
/// PR 3a left it as the one `truthy` call site it did not reach:
/// `pycc_types`' `unop.rs` answered `T0021` for a non-`bool` operand before
/// codegen saw it. Part 10 of #1371 types `not o` as a native `bool` over
/// the same truth test, so it now checks like the condition sites.
#[test]
fn not_on_a_cpython_object_is_admitted() {
    let dir = ScratchDir::new("foreign_truthy_not").expect("scratch");
    let out = check(&dir, "import gc\n\nif not gc:\n    print(1)\n");
    assert!(
        out.status.success(),
        "{}{}",
        stdout_of(&out),
        stderr_of(&out)
    );
}

/// Both operations are admitted inside a function body since #1316, which
/// gave every pycc function a failure edge for a raising `PyObject_Size`
/// or `PyObject_IsTrue` to take (`crates/pycc_codegen/src/foreign_fail.rs`).
#[test]
fn both_operations_are_admitted_in_a_function_body() {
    let dir = ScratchDir::new("foreign_len_positional").expect("scratch");
    for body in [
        "import gc\n\ndef _n() -> int:\n    return len(gc)\n\nprint(_n())\n",
        "import gc\n\ndef _t() -> int:\n    if gc:\n        return 1\n    return 0\n\nprint(_t())\n",
    ] {
        let out = check(&dir, body);
        assert!(out.status.success(), "{body}: {}", stdout_of(&out));
    }
}

/// Builds `body` as an extension module named `module` inside `dir`.
///
/// The output path carries **no** suffix, exactly as
/// `tests/issue_1081_foreign_method_call.rs`'s own helper writes it:
/// `pycc build --ext` appends the one its target triple calls for.
fn build_ext(dir: &Path, module: &str, body: &str) {
    let src = dir.join(format!("{module}.py"));
    std::fs::write(&src, body).expect("write the fixture source");
    let build = pycc()
        .arg("build")
        .arg(&src)
        .arg("-o")
        .arg(dir.join(module))
        .arg("--ext")
        .output()
        .expect("pycc should spawn");
    assert!(build.status.success(), "{}", stderr_of(&build));
}

fn python(dir: &Path, script: &str) -> Output {
    Command::new(std::env::var_os("PYCC_PYTHON").unwrap_or_else(|| "python3".into()))
        .arg("-c")
        .arg(script)
        .current_dir(dir)
        .output()
        .expect("python3 should spawn")
}

/// **The acceptance test for PR 3a.** A built extension module observes a
/// real container's length and a real object's truth value from the host.
///
/// `sys.argv` is chosen because `python3 -c ...` gives it exactly one
/// element, so the printed length is a fixed number rather than something
/// the test would have to compute the same way twice. `sys.warnoptions` is
/// the falsy operand: an empty list under a default interpreter.
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn len_and_truth_testing_reach_the_host() {
    let dir = ScratchDir::new("foreign_len_hosted").expect("scratch");
    build_ext(
        &dir,
        "pycc_len_truth_mod",
        "import sys\n\
         \n\
         print(len(sys.argv))\n\
         if sys.argv:\n\
         \x20   print(\"argv is truthy\")\n\
         if sys.warnoptions:\n\
         \x20   print(\"warnoptions is truthy\")\n\
         else:\n\
         \x20   print(\"warnoptions is falsy\")\n",
    );
    let run = python(&dir, "import pycc_len_truth_mod\n");
    assert!(
        run.status.success(),
        "stdout: {}\nstderr: {}",
        stdout_of(&run),
        stderr_of(&run)
    );
    assert_eq!(
        stdout_of(&run),
        "1\nargv is truthy\nwarnoptions is falsy\n",
        "stderr: {}",
        stderr_of(&run)
    );
}

/// `len` of an operand with no length surfaces CPython's own `TypeError`,
/// and the module body stops there.
///
/// The `-1` edge `foreign_len::emit_len` emits, end to end: a module object
/// has no `__len__`, `PyObject_Size` raises, `pycc_ext_obj_len` returns
/// `-1` with the exception set, and the `Py_mod_exec` slot returns `-1`
/// without clearing it. The `print` below is what proves the body stopped
/// rather than merely reported.
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn a_len_of_an_unsized_object_raises_type_error_in_the_host() {
    let dir = ScratchDir::new("foreign_len_unsized_hosted").expect("scratch");
    build_ext(
        &dir,
        "pycc_len_unsized_mod",
        "import gc\n\nprint(len(gc))\nprint(99)\n",
    );
    let run = python(
        &dir,
        "try:\n\
         \x20   import pycc_len_unsized_mod\n\
         except TypeError as e:\n\
         \x20   print('TypeError', e)\n\
         else:\n\
         \x20   raise AssertionError('the import should have raised')\n",
    );
    assert!(
        run.status.success(),
        "stdout: {}\nstderr: {}",
        stdout_of(&run),
        stderr_of(&run)
    );
    assert!(
        stdout_of(&run).starts_with("TypeError"),
        "{}",
        stdout_of(&run)
    );
    assert!(!stdout_of(&run).contains("99"), "{}", stdout_of(&run));
}

/// A truth test whose `__bool__` raises surfaces that exception in the host,
/// and the module body stops there.
///
/// The `-1` edge `foreign_len::emit_truthy` emits, end to end -- the mirror
/// of `a_len_of_an_unsized_object_raises_type_error_in_the_host` for the
/// other failing call: `PyObject_IsTrue` runs arbitrary user code, so it is
/// at least as likely to raise as `PyObject_Size`. `pycc_ext_obj_truthy`
/// returns `-1` with the exception set, and the `Py_mod_exec` slot returns
/// `-1` without clearing it. The two absent sentinels below are what prove
/// the body stopped at the condition rather than merely reported: neither
/// the `if` body nor the statement after it runs.
///
/// The operand is a `ValueError`, not the sibling test's `TypeError`, so a
/// host that observed the wrong exception class could not pass both tests
/// with one hard-coded answer.
///
/// The helper module lives in a **subdirectory** of the scratch dir rather
/// than beside the fixture source: a `.py` next to the source is resolved
/// as a *project* import, which is still `C0001` (`import <project module>`
/// binds a module namespace), so it would never reach the foreign-object
/// path this test exists to exercise. Putting it out of the driver's reach
/// and onto the host's `sys.path` at run time is what makes the import
/// foreign at compile time and resolvable at run time.
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn a_truth_test_that_raises_propagates_the_exception_in_the_host() {
    let dir = ScratchDir::new("foreign_truthy_raises_hosted").expect("scratch");
    let helper_dir = dir.join("host_only");
    std::fs::create_dir_all(&helper_dir).expect("create the helper directory");
    std::fs::write(
        helper_dir.join("pycc_boom_helper.py"),
        "class _Boom:\n\
         \x20   def __bool__(self) -> bool:\n\
         \x20       raise ValueError(\"no truth value\")\n\
         \n\
         \n\
         boom = _Boom()\n",
    )
    .expect("write the helper module");
    build_ext(
        &dir,
        "pycc_truthy_raises_mod",
        "import pycc_boom_helper\n\
         \n\
         if pycc_boom_helper.boom:\n\
         \x20   print(\"if-body reached\")\n\
         print(\"after the if\")\n",
    );
    let run = python(
        &dir,
        "import sys\n\
         sys.path.insert(0, 'host_only')\n\
         try:\n\
         \x20   import pycc_truthy_raises_mod\n\
         except ValueError as e:\n\
         \x20   print('ValueError', e)\n\
         else:\n\
         \x20   raise AssertionError('the import should have raised')\n",
    );
    assert!(
        run.status.success(),
        "stdout: {}\nstderr: {}",
        stdout_of(&run),
        stderr_of(&run)
    );
    assert!(
        stdout_of(&run).starts_with("ValueError"),
        "{}",
        stdout_of(&run)
    );
    assert!(
        !stdout_of(&run).contains("if-body reached"),
        "{}",
        stdout_of(&run)
    );
    assert!(
        !stdout_of(&run).contains("after the if"),
        "{}",
        stdout_of(&run)
    );
}

/// A module-level `def len` does not derail `len(<foreign object>)`.
///
/// The regression PR 3a's original MIR guard caused: it declined the
/// `MirExpr::ObjLen` split whenever `$fn:len` was in scope, but
/// `pycc_types::check` resolves `len` as the reserved builtin regardless,
/// so the call reached codegen as an ordinary `Call` and tripped
/// `expect_list_pointer`'s internal-error assertion -- a `pycc check` that
/// exits 0 followed by a compiler panic in `pycc build --ext`.
///
/// The printed value is `1`, not `2`: `python3 -c` gives `sys.argv` one
/// element and the shadowing definition is deliberately *not* honoured, so
/// this asserts the builtin's answer. That CPython would print `2` here is
/// the divergence filed as #1098; this test pins the compiler's current,
/// non-panicking behavior rather than that open question.
///
/// The non-ignored coverage for the same fix lives in `pycc_mir`'s
/// `len_of_a_foreign_object_ignores_a_module_level_len_definition`, so the
/// changed lines are covered without a hosting interpreter.
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn a_shadowing_len_definition_still_builds_in_the_host() {
    let dir = ScratchDir::new("foreign_len_shadowed_hosted").expect("scratch");
    build_ext(
        &dir,
        "pycc_len_shadowed_mod",
        "import sys\n\
         \n\
         \n\
         def len(x: int) -> int:\n\
         \x20   return x + 1\n\
         \n\
         \n\
         n: int = 0\n\
         n = len(sys.argv)\n\
         print(n)\n",
    );
    let run = python(&dir, "import pycc_len_shadowed_mod\n");
    assert!(
        run.status.success(),
        "stdout: {}\nstderr: {}",
        stdout_of(&run),
        stderr_of(&run)
    );
    assert_eq!(stdout_of(&run), "1\n", "stderr: {}", stderr_of(&run));
}
