//! Part 4 of #1026: the scalar conversions out of a CPython object --
//! `float(o)` and `bool(o)` (PR 4a of #1083), `int(o)` and `str(o)`
//! (PR 4b).
//!
//! `tests/issue_1080_foreign_object.rs` owns what a foreign `import` binds
//! and the refusal table around it; `tests/issue_1081_foreign_method_call.rs`
//! owns the method call; `tests/issue_1082_foreign_len_and_truth.rs` owns
//! `len`, truth testing, the subscript load and `for` iteration. This file
//! owns the four conversions.
//!
//! **These are explicit conversions, not implicit boundary crossings.**
//! D-244 rule 7 keeps the type boundary closed at the *thunk export seam*,
//! where a value crosses implicitly and its annotation is the whole
//! contract. `float(o)` in user source names its destination type, so
//! running CPython's own conversion protocol (`PyNumber_Float`,
//! `PyObject_IsTrue`, `PyNumber_Long`, `PyObject_Str`) is exactly what the
//! author asked for. `docs/TYPE_SYSTEM.md`'s `object` row carries the same
//! paragraph.
//!
//! **The relaxation is `Ty::Object`-only, and the residual incoherence is
//! stated rather than hidden.** `bool(o)` and `str(o)` compile while
//! `bool(1)` and `str(1)` keep their `C0001`; #1017/#1018 own the general
//! builtin-conversion story. Every other argument type reaches the unchanged
//! refusal, which the tests below pin in both directions.
//!
//! PR 4b's own two consequences are pinned here as well: `print(str(o))`
//! type-checks (the `str` the shim copies out of CPython is an ordinary pycc
//! `str`, so `string_conversion.rs` needs nothing for it; `print(o)` itself
//! is admitted too since #1340), and `int(o)` refuses a value outside the D-141
//! inline-integer range with `OverflowError` rather than growing a bigint
//! path (#1040).
//!
//! The hosted tests contribute no line coverage (CI's coverage job runs
//! `llvm-cov` without `--include-ignored`); they are run by the Tier-1
//! `native-build-test` leg's `cargo test --workspace -- --include-ignored`.
//! Every new Rust line is covered by the non-ignored tests here, by
//! `crates/pycc_codegen/src/foreign_len.rs`'s IR assertions, and by the unit
//! tests in `crates/pycc_types` and `crates/pycc_mir`.

use pycc_scratch::ScratchDir;
use std::path::Path;
use std::process::{Command, Output};

fn pycc() -> Command {
    Command::new(env!("CARGO_BIN_EXE_pycc"))
}

/// Normalizes the two line-ending conventions a single captured stream can
/// carry at once.
///
/// A pycc-compiled `print` writes `\n` verbatim, while the host CPython
/// driver's own `print` goes through CPython's text layer, which translates
/// `\n` to `\r\n` on Windows -- so `both_conversions_reach_the_host`
/// compares a pycc-written stream against a CPython-written one, and
/// `a_float_of_an_unconvertible_object_raises_type_error_in_the_host`
/// compares CPython's own stderr against a `\n` literal. Both are
/// unsatisfiable byte-exactly on `windows-latest`. Normalizing in these
/// helpers rather than at the two assertions that happened to fail covers
/// every assertion in this file, following
/// `tests/issue_1082_foreign_iteration.rs`, which established the
/// convention for the same reason.
fn normalize_newlines(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).replace("\r\n", "\n")
}

fn stdout_of(output: &Output) -> String {
    normalize_newlines(&output.stdout)
}

fn stderr_of(output: &Output) -> String {
    normalize_newlines(&output.stderr)
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

/// Both conversions type-check, and their results are ordinary scalars.
///
/// The second and third bodies of each row are what make the first useful:
/// binding the result to a name and feeding it to arithmetic or a `print`
/// both go through the ordinary `float`/`bool` paths, so a converted value
/// is not something that has to stay anonymous the way the object itself
/// does (`check_assignment` still refuses *that* inside a function body).
#[test]
fn both_conversions_of_a_cpython_object_are_admitted() {
    let dir = ScratchDir::new("foreign_conversions_admitted").expect("scratch");
    for body in [
        "import gc\n\nprint(float(gc))\n",
        "import gc\n\nx = float(gc)\nprint(x)\n",
        "import gc\n\nx = float(gc) + 1.0\nprint(x)\n",
        "import gc\n\nprint(bool(gc))\n",
        "import gc\n\nb = bool(gc)\nprint(b)\n",
        "import gc\n\nb = bool(gc)\nif b:\n    print(1)\n",
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

/// `bool` on anything that is not a CPython object keeps `C0001` verbatim.
///
/// Fork 1 of the plan, pinned from the outside: PR 4a admits the builtin for
/// `Ty::Object` alone, so every previously-refused `bool` call is refused in
/// exactly the way it was. A regression here would most likely arrive as an
/// "improvement" -- adding `bool` to `KNOWN_CALLABLE_BUILTINS`' neighbours
/// or generalizing the arm -- which is why the code, not just the object
/// case, is asserted.
#[test]
fn bool_of_a_non_object_keeps_its_unchanged_refusal() {
    let dir = ScratchDir::new("foreign_bool_non_object").expect("scratch");
    for body in [
        "print(bool(1))\n",
        "print(bool(1.5))\n",
        "print(bool(\"x\"))\n",
        "xs = [1]\nprint(bool(xs))\n",
    ] {
        let out = check(&dir, body);
        assert!(!out.status.success(), "{body} should still be refused");
        let text = format!("{}{}", stdout_of(&out), stderr_of(&out));
        assert!(text.contains("C0001"), "{body}: {text}");
    }
}

/// `float` on a non-numeric, non-object argument keeps its `T0021` message
/// unchanged, including the enumeration that deliberately omits `object`.
///
/// `object` is unspellable in an annotation (`docs/TYPE_SYSTEM.md`'s
/// `object` row), so "pass an `object`"
/// would be advice nobody can act on by writing a type. The omission is a
/// decision, and this is its outside pin;
/// `pycc_types::tests::float_of_a_str_is_rejected_as_t0021` is the inside
/// one.
#[test]
fn float_of_a_str_keeps_its_unchanged_message() {
    let dir = ScratchDir::new("foreign_float_str").expect("scratch");
    let out = check(&dir, "x = \"s\"\nprint(float(x))\n");
    assert!(!out.status.success());
    let text = format!("{}{}", stdout_of(&out), stderr_of(&out));
    assert!(
        text.contains("`float` expects an `int`, `float`, or `bool` argument, got `str`"),
        "{text}"
    );
}

/// A user-defined `def float`/`def bool` still wins over the builtin.
///
/// Plan correction C4: both are valid, working programs on `main` today, so
/// every new arm carries the same guard `float` already carried, in every
/// mirror. The `+ 1` is what discriminates -- it type-checks only if the
/// call resolved to the user function's `int` return.
#[test]
fn a_user_defined_conversion_function_still_wins() {
    let dir = ScratchDir::new("foreign_conversion_shadowed").expect("scratch");
    for name in ["float", "bool"] {
        let body = format!(
            "def {name}(x: int) -> int:\n\
             \x20   return x + 1\n\
             \n\
             print({name}(2) + 1)\n"
        );
        let out = check(&dir, &body);
        assert!(
            out.status.success(),
            "{name}: {}{}",
            stdout_of(&out),
            stderr_of(&out)
        );
    }
}

/// A user-defined `class float`/`class bool` still wins over the builtin.
///
/// Reviewer finding (codex, PR 4a of #1083), the class-shaped twin of the
/// `def` guard above. MIR resolves a call whose callee names a class as an
/// instantiation, so admitting `Ty::Object` in the builtin arm without
/// consulting the class table let `pycc check` return `float`/`bool` for a
/// program MIR would build as a constructor call -- accepted here, panicking
/// in codegen. Each arm now defers to the class table, and the refusal each
/// program gets back is `main`'s own, unchanged:
///
/// * `float` never reaches the class table at all -- excluding `Ty::Object`
///   from its admission drops the argument into the same `T0021` enumeration
///   a `str` argument hits, which is exactly what `main` (where `Ty::Object`
///   was never admitted) answers.
/// * `bool` has no builtin arm on `main` at all, so its call already resolved
///   as an instantiation there and was refused by the constructor's own
///   parameter check. Skipping the arm restores precisely that.
///
/// The two messages therefore differ by design; asserting them, rather than
/// just a non-zero exit, is what pins "identical to `main`".
#[test]
fn a_user_defined_conversion_class_still_wins() {
    let dir = ScratchDir::new("foreign_conversion_shadowed_class").expect("scratch");
    for (name, message) in [
        (
            "float",
            "`float` expects an `int`, `float`, or `bool` argument, got `object`",
        ),
        ("bool", "argument 1 of `bool` expects `int`, got `object`"),
    ] {
        let body = format!(
            "import gc\n\
             \n\
             class {name}:\n\
             \x20   def __init__(self, x: int) -> None:\n\
             \x20       self.x = x\n\
             \n\
             v = {name}(gc)\n\
             print(v)\n"
        );
        let out = check(&dir, &body);
        assert!(
            !out.status.success(),
            "{name}: a class-shadowed conversion must be refused: {}{}",
            stdout_of(&out),
            stderr_of(&out)
        );
        let text = format!("{}{}", stdout_of(&out), stderr_of(&out));
        assert!(
            text.contains(message),
            "{name}: expected `main`'s own refusal ({message}), got: {text}"
        );
    }
}

/// Both conversions are admitted inside a function body since #1316, which
/// lifted PR 2a's positional bound for a module-level foreign name.
#[test]
fn a_conversion_inside_a_function_body_is_admitted() {
    let dir = ScratchDir::new("foreign_conversion_in_function").expect("scratch");
    for name in ["float", "bool"] {
        let body = format!(
            "import gc\n\
             \n\
             def f() -> int:\n\
             \x20   {name}(gc)\n\
             \x20   return 1\n\
             \n\
             print(f())\n"
        );
        let out = check(&dir, &body);
        assert!(out.status.success(), "{name}: {}", stdout_of(&out));
    }
}

/// The `I0404` enumeration names every admitted conversion.
///
/// It is narrowed once per PR, and a stale enumeration is a user-facing lie
/// about what the compiler can do.
#[test]
fn the_refusal_message_lists_the_admitted_conversions() {
    let dir = ScratchDir::new("foreign_conversion_enumeration").expect("scratch");
    // A `match` subject is still refused; `print(gc)`, the former probe, is
    // admitted since #1340.
    let out = check(
        &dir,
        "import gc\n\nmatch gc:\n    case 1:\n        print(1)\n",
    );
    assert!(!out.status.success());
    let text = format!("{}{}", stdout_of(&out), stderr_of(&out));
    assert!(text.contains("error[I0404]"), "{text}");
    assert!(
        text.contains("the `float`, `bool`, `int` and `str` conversions"),
        "{text}"
    );
    assert!(
        text.contains("printing it and f-string interpolation"),
        "{text}"
    );
}

/// Both conversions reach a real CPython interpreter and answer what CPython
/// itself would.
///
/// `sys.argv` is a non-empty list under `python -c`, so it is truthy;
/// `sys.warnoptions` is empty under a default interpreter, so it is falsy.
/// `sys.maxunicode` is an `int`, which `PyNumber_Float` converts through
/// `__float__`/`__index__` -- the protocol the shim runs, observed from the
/// outside. It is `sys.maxunicode` rather than `sys.maxsize` because pycc's
/// own float formatting has no scientific notation yet and `sys.maxsize` as
/// a `double` needs it; the conversion itself is what is under test, and the
/// expectation is computed by the same interpreter so the value stays the
/// oracle's rather than this test's.
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn both_conversions_reach_the_host() {
    let dir = ScratchDir::new("foreign_conversions_hosted").expect("scratch");
    build_ext(
        &dir,
        "pycc_conversions_mod",
        "import sys\n\
         \n\
         print(float(sys.maxunicode))\n\
         print(bool(sys.argv))\n\
         print(bool(sys.warnoptions))\n",
    );
    let run = python(&dir, "import pycc_conversions_mod\n");
    assert!(
        run.status.success(),
        "stdout: {}\nstderr: {}",
        stdout_of(&run),
        stderr_of(&run)
    );
    let expected = python(
        &dir,
        "import sys\nprint(float(sys.maxunicode))\nprint(bool(sys.argv))\nprint(bool(sys.warnoptions))\n",
    );
    assert_eq!(
        stdout_of(&run),
        stdout_of(&expected),
        "stderr: {}",
        stderr_of(&run)
    );
}

/// `float` of an operand with no conversion surfaces CPython's own
/// `TypeError`, and the module body stops there.
///
/// The `-1` edge `foreign_len::emit_to_float` emits, end to end: a module
/// object has no `__float__`, `PyNumber_Float` raises, the shim releases
/// nothing it did not acquire and returns `-1` with the exception set, and
/// the `Py_mod_exec` slot returns `-1` without clearing it. The trailing
/// `print` is what proves the body stopped rather than merely reported.
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn a_float_of_an_unconvertible_object_raises_type_error_in_the_host() {
    let dir = ScratchDir::new("foreign_float_unconvertible_hosted").expect("scratch");
    build_ext(
        &dir,
        "pycc_float_bad_mod",
        "import gc\n\nprint(float(gc))\nprint(99)\n",
    );
    let run = python(
        &dir,
        "try:\n\
         \x20   import pycc_float_bad_mod\n\
         except TypeError as e:\n\
         \x20   print(\"TypeError\")\n",
    );
    assert!(
        run.status.success(),
        "stdout: {}\nstderr: {}",
        stdout_of(&run),
        stderr_of(&run)
    );
    assert_eq!(
        stdout_of(&run),
        "TypeError\n",
        "stderr: {}",
        stderr_of(&run)
    );
}

/// A module-level `def float`/`def bool` still builds in the host.
///
/// The `pycc_mir` counterpart of the C4 guard: without the `$fn:` scope scan
/// in the `bool` lowering arm, `pycc check` would exit 0 and `pycc build
/// --ext` would abort in `lookup`. The non-ignored line coverage for that
/// guard is `pycc_mir`'s
/// `a_user_defined_bool_function_is_lowered_as_a_real_call_not_the_builtin`.
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn a_shadowing_conversion_definition_still_builds_in_the_host() {
    let dir = ScratchDir::new("foreign_conversion_shadow_hosted").expect("scratch");
    build_ext(
        &dir,
        "pycc_conversion_shadow_mod",
        "def bool(x: int) -> int:\n\
         \x20   return x + 1\n\
         \n\
         print(bool(2))\n",
    );
    let run = python(&dir, "import pycc_conversion_shadow_mod\n");
    assert!(
        run.status.success(),
        "stdout: {}\nstderr: {}",
        stdout_of(&run),
        stderr_of(&run)
    );
    assert_eq!(stdout_of(&run), "3\n", "stderr: {}", stderr_of(&run));
}

/// PR 4b's two conversions type-check, and their results are ordinary
/// scalars.
///
/// `both_conversions_of_a_cpython_object_are_admitted`'s claim for `int` and
/// `str`, with the same three shapes per row: the bare call, a binding, and
/// a consumer that only accepts a real scalar. The last `str` row needs
/// nothing from `string_conversion.rs`, because the value it sees is an
/// ordinary `str`.
#[test]
fn the_other_two_conversions_of_a_cpython_object_are_admitted() {
    let dir = ScratchDir::new("foreign_int_str_admitted").expect("scratch");
    for body in [
        "import gc\n\nprint(int(gc))\n",
        "import gc\n\nx = int(gc)\nprint(x)\n",
        "import gc\n\nx = int(gc) + 1\nprint(x)\n",
        "import gc\n\nprint(str(gc))\n",
        "import gc\n\ns = str(gc)\nprint(s)\n",
        "import gc\n\ns = str(gc)\nprint(f\"[{s}]\")\n",
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

/// `int`/`str` on anything that is not a CPython object keep `C0001`
/// verbatim.
///
/// `bool_of_a_non_object_keeps_its_unchanged_refusal`'s claim for the two
/// names PR 4b adds, and the outside pin for fork 1's scope boundary: the
/// arms are `Ty::Object`-only, so `str(1)` is refused exactly as it was.
#[test]
fn int_and_str_of_a_non_object_keep_their_unchanged_refusal() {
    let dir = ScratchDir::new("foreign_int_str_non_object").expect("scratch");
    for body in [
        "print(int(1))\n",
        "print(int(1.5))\n",
        "print(int(\"3\"))\n",
        "print(str(1))\n",
        "print(str(1.5))\n",
        "xs = [1]\nprint(str(xs))\n",
    ] {
        let out = check(&dir, body);
        assert!(!out.status.success(), "{body} should still be refused");
        let text = format!("{}{}", stdout_of(&out), stderr_of(&out));
        assert!(text.contains("C0001"), "{body}: {text}");
    }
}

/// A user-defined `def int`/`def str` still wins over the builtin.
///
/// `a_user_defined_conversion_function_still_wins`'s claim for PR 4b's two
/// names. The `+ 1.0` is what discriminates -- it type-checks only if the
/// call resolved to the user function's `float` return rather than to the
/// builtin's `int`/`str`.
#[test]
fn a_user_defined_int_or_str_function_still_wins() {
    let dir = ScratchDir::new("foreign_int_str_shadowed").expect("scratch");
    for name in ["int", "str"] {
        let body = format!(
            "def {name}(x: float) -> float:\n\
             \x20   return x + 1.0\n\
             \n\
             print({name}(2.0) + 1.0)\n"
        );
        let out = check(&dir, &body);
        assert!(
            out.status.success(),
            "{name}: {}{}",
            stdout_of(&out),
            stderr_of(&out)
        );
    }
}

/// A user-defined `class int`/`class str` still wins over the builtin.
///
/// The class-shaped twin of the `def` guard, and `a_user_defined_conversion_
/// class_still_wins`'s claim for PR 4b's two names. Each new arm consults the
/// class table before admitting a `Ty::Object` argument, so the call resolves
/// as an instantiation and the program gets the shadowing constructor's own
/// parameter check -- which is precisely what `main` answered before PR 4b,
/// where the arms did not exist at all. Asserting the message rather than
/// just a non-zero exit is what pins "identical to `main`".
///
/// The constructor takes a `float` rather than an `int` deliberately: a class
/// named `int` shadows the *annotation* `int` in its own module, so `x: int`
/// would name the class being defined and draw an unrelated `C0001`.
#[test]
fn a_user_defined_int_or_str_class_still_wins() {
    let dir = ScratchDir::new("foreign_int_str_shadowed_class").expect("scratch");
    for name in ["int", "str"] {
        let body = format!(
            "import gc\n\
             \n\
             class {name}:\n\
             \x20   def __init__(self, x: float) -> None:\n\
             \x20       self.x = x\n\
             \n\
             v = {name}(gc)\n\
             print(v.x)\n"
        );
        let out = check(&dir, &body);
        assert!(
            !out.status.success(),
            "{name}: a class-shadowed conversion must be refused: {}{}",
            stdout_of(&out),
            stderr_of(&out)
        );
        let text = format!("{}{}", stdout_of(&out), stderr_of(&out));
        let message = format!("argument 1 of `{name}` expects `float`, got `object`");
        assert!(
            text.contains(&message),
            "{name}: expected `main`'s own refusal ({message}), got: {text}"
        );
    }
}

/// PR 4b's conversions are admitted inside a function body too (#1316).
#[test]
fn an_int_or_str_conversion_inside_a_function_body_is_admitted() {
    let dir = ScratchDir::new("foreign_int_str_in_function").expect("scratch");
    for name in ["int", "str"] {
        let body = format!(
            "import gc\n\
             \n\
             def f() -> int:\n\
             \x20   {name}(gc)\n\
             \x20   return 1\n\
             \n\
             print(f())\n"
        );
        let out = check(&dir, &body);
        assert!(out.status.success(), "{name}: {}", stdout_of(&out));
    }
}

/// `int(o)` and `str(o)` reach a real CPython interpreter and answer what
/// CPython itself would.
///
/// `sys.maxunicode` is an `int`, which `PyNumber_Long` returns unchanged and
/// `PyObject_Str` renders; `sys` itself has no integer conversion but does
/// have a `__str__`, which is what makes the second half of the table a test
/// of `PyObject_Str` rather than of a number's formatting. The expectation is
/// computed by the same interpreter, so the oracle stays CPython's.
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn the_other_two_conversions_reach_the_host() {
    let dir = ScratchDir::new("foreign_int_str_hosted").expect("scratch");
    build_ext(
        &dir,
        "pycc_int_str_mod",
        "import sys\n\
         \n\
         print(int(sys.maxunicode))\n\
         print(str(sys.maxunicode))\n\
         print(str(sys))\n",
    );
    let run = python(&dir, "import pycc_int_str_mod\n");
    assert!(
        run.status.success(),
        "stdout: {}\nstderr: {}",
        stdout_of(&run),
        stderr_of(&run)
    );
    let expected = python(
        &dir,
        "import sys\nprint(int(sys.maxunicode))\nprint(str(sys.maxunicode))\nprint(str(sys))\n",
    );
    assert_eq!(
        stdout_of(&run),
        stdout_of(&expected),
        "stderr: {}",
        stderr_of(&run)
    );
}

/// An `int(o)` whose result is outside pycc's inline-integer range raises
/// `OverflowError` in the host rather than silently truncating or growing a
/// bigint path.
///
/// `sys.maxsize` is `2**63 - 1` on every 64-bit build, which is outside
/// `[-2**62, 2**62-1]`, so this is the shim's own encode arm -- the same
/// refusal `pycc_ext_unpack_int_at` gives at the thunk boundary, citing
/// #1040. The trailing `print` proves the module body stopped rather than
/// merely reported.
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn an_out_of_range_int_conversion_raises_overflow_error_in_the_host() {
    let dir = ScratchDir::new("foreign_int_overflow_hosted").expect("scratch");
    build_ext(
        &dir,
        "pycc_int_overflow_mod",
        "import sys\n\nprint(int(sys.maxsize))\nprint(99)\n",
    );
    let run = python(
        &dir,
        "try:\n\
         \x20   import pycc_int_overflow_mod\n\
         except OverflowError as e:\n\
         \x20   print(\"OverflowError\", \"#1040\" in str(e))\n",
    );
    assert!(
        run.status.success(),
        "stdout: {}\nstderr: {}",
        stdout_of(&run),
        stderr_of(&run)
    );
    assert_eq!(
        stdout_of(&run),
        "OverflowError True\n",
        "stderr: {}",
        stderr_of(&run)
    );
}

/// `str(o)` copies CPython's bytes into a pycc `str` that outlives the
/// CPython temporary the conversion produced.
///
/// The §2 ordering, observed from the outside: `PyObject_Str` returns a *new*
/// reference and `PyUnicode_AsUTF8AndSize` points into that object's own
/// buffer, so the `pycc_rt_str_from_literal` copy has to complete before the
/// `Py_DECREF`. The converted value is the module's only reference to those
/// bytes -- CPython's own temporary is unreachable the moment the helper
/// returns -- and the two conversions plus the `f`-string below all read it
/// afterwards, so a release-then-copy inversion is a use-after-free this
/// test's output would show. `gc` is used rather than `sys` because a
/// built-in module's `repr` is short and fixed.
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn a_str_conversion_outlives_the_cpython_temporary_in_the_host() {
    let dir = ScratchDir::new("foreign_str_ownership_hosted").expect("scratch");
    build_ext(
        &dir,
        "pycc_str_ownership_mod",
        "import gc\n\
         \n\
         a = str(gc)\n\
         b = str(gc)\n\
         print(f\"[{a}][{b}]\")\n",
    );
    let run = python(&dir, "import pycc_str_ownership_mod\n");
    assert!(
        run.status.success(),
        "stdout: {}\nstderr: {}",
        stdout_of(&run),
        stderr_of(&run)
    );
    let expected = python(&dir, "import gc\nprint(f\"[{str(gc)}][{str(gc)}]\")\n");
    assert_eq!(
        stdout_of(&run),
        stdout_of(&expected),
        "stderr: {}",
        stderr_of(&run)
    );
}
