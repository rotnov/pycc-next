//! Part 2 of #1026 (PR 2b of #1081): calling a method on a CPython object.
//!
//! `tests/issue_1080_foreign_object.rs` owns what a foreign `import` binds
//! and the refusal table around it, including the one row this PR flips
//! (`numpy.sqrt(2.0)`, refused as `T0043` by PR 2a and accepted here). This
//! file owns the capability itself: what the checker still refuses about a
//! call, that PR 2a's positional bound is inherited unchanged, and -- in the
//! `#[ignore]`d hosted tests at the bottom -- that a real CPython
//! interpreter observes the call's effect and its exceptions.
//!
//! The hosted tests contribute no line coverage (CI's coverage job runs
//! `llvm-cov` without `--include-ignored`); they are run by the Tier-1
//! `native-build-test` leg's `cargo test --workspace -- --include-ignored`.
//! Every new Rust line is covered by the non-ignored tests here and by the
//! unit tests in `crates/pycc_codegen/src/foreign_call.rs`.

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

/// Every argument type the shim has a `pycc_ext_obj_pack_*` helper for is
/// admitted, and zero arguments are too.
///
/// The zero-argument row is the acceptance shape (`gc.disable()`); the
/// other five are the marshalling surface. Asserted through `pycc check`
/// rather than a build so the row set stays readable and fast -- the
/// emitted code for each packer is asserted at the IR level in
/// `crates/pycc_codegen/src/foreign_call/tests.rs`.
#[test]
fn every_marshallable_argument_type_is_admitted() {
    let dir = ScratchDir::new("foreign_call_arg_types").expect("scratch");
    for body in [
        "import gc\n\ngc.disable()\n",
        "import gc\n\ngc.set_threshold(700)\n",
        "import json\n\njson.dumps(2.0)\n",
        "import gc\n\ngc.set_debug(True)\n",
        "import json\n\njson.dumps(\"x\")\n",
        // A second CPython object, packed by `pycc_ext_obj_pack_object`
        // since Part 2a of #1371.
        "import json\nimport gc\n\ngc.set_debug(json.dumps)\n",
        // Several arguments at once, and a name rather than a literal.
        "import gc\n\nx: int = 1\ngc.set_threshold(700, x)\n",
    ] {
        let output = check(&dir, body);
        assert_eq!(
            output.status.code(),
            Some(0),
            "{body}: {}",
            stdout_of(&output)
        );
    }
}

/// An argument whose type has no boundary representation is refused with
/// `I0404`, naming the type.
///
/// A second CPython object is no longer among them: since Part 2a of #1371
/// it is packed by `pycc_ext_obj_pack_object` (the `json.dumps` row of
/// `every_marshallable_argument_type_is_admitted` above, and
/// `tests/issue_1371_object_calls.rs`). The refusal of every other type is
/// what makes `foreign_pack::packer_for`'s panic a front-end-defect
/// assertion rather than a reachable failure.
#[test]
fn an_unmarshallable_argument_is_refused_with_i0404() {
    let dir = ScratchDir::new("foreign_call_bad_arg").expect("scratch");
    for (body, ty) in [
        ("import gc\n\ngc.set_debug([1])\n", "list[int]"),
        // A `None` argument is admitted since Part 8 of #1371.
        ("import gc\n\ngc.set_debug({\"a\": 1})\n", "dict[str, int]"),
    ] {
        let output = check(&dir, body);
        assert_eq!(
            output.status.code(),
            Some(1),
            "{body}: {}",
            stderr_of(&output)
        );
        let rendered = stdout_of(&output);
        assert!(rendered.contains("error[I0404]"), "{body}: {rendered}");
        assert!(
            rendered.contains(&format!(
                "passing a `{ty}` argument to a CPython object's method"
            )),
            "{body}: {rendered}"
        );
    }
}

/// A call inside a function body is admitted since #1316, and a call above
/// the `import` is still `T0021`.
///
/// #1316 gave every pycc function a failure edge
/// (`crates/pycc_codegen/src/foreign_fail.rs`): a failed call there is
/// bridged to a pycc exception and branches to the innermost handler, so
/// the positional bound PR 2b relied on no longer applies to a function
/// body. The source-order bound still does.
#[test]
fn a_method_call_inherits_the_source_order_bound() {
    let dir = ScratchDir::new("foreign_call_position").expect("scratch");

    let output = check(
        &dir,
        "import gc\n\n\ndef off() -> None:\n    gc.disable()\n",
    );
    assert_eq!(output.status.code(), Some(0), "{}", stdout_of(&output));

    let output = check(&dir, "gc.disable()\n\nimport gc\n");
    assert_eq!(output.status.code(), Some(1), "{}", stderr_of(&output));
    let rendered = stdout_of(&output);
    assert!(rendered.contains("error[T0021]"), "{rendered}");
    assert!(rendered.contains("`gc` is not defined"), "{rendered}");
}

/// A plain `pycc build` still refuses an import it cannot embed with
/// `I0403` before any of this is reachable: `tkinter` needs Tcl/Tk from
/// outside the interpreter, which an embedded executable does not bundle
/// (D-248 rule 1). A standard-library call such as `gc.disable()` builds
/// embedded instead; see the hosted test below. A third-party root is
/// bundled from `pycc.lock` (#1242).
#[test]
fn a_native_build_of_a_method_call_is_still_refused_with_i0403() {
    let dir = ScratchDir::new("foreign_call_native").expect("scratch");
    let output = pycc()
        .arg("build")
        .arg(source(&dir, "import tkinter\n\ntkinter.mainloop()\n"))
        .arg("-o")
        .arg(dir.join("m"))
        .output()
        .expect("pycc should spawn");
    assert_eq!(output.status.code(), Some(1), "{}", stdout_of(&output));
    // `build` renders diagnostics to stderr where `check` renders to stdout
    // (`src/frontend.rs`'s `render_all`).
    let rendered = stderr_of(&output);
    assert!(rendered.contains("error[I0403]"), "{rendered}");
}

/// Builds `body` as an extension module named `module` inside `dir`.
///
/// The output path carries **no** suffix, exactly as `issue_1080_foreign_object.rs`'s
/// own helper writes it: `pycc build --ext` appends the one its target
/// triple calls for. Spelling `.so` here builds on macOS and then fails the
/// `--ext` name contract on Windows, where the module name would come out
/// as `<module>.so` and is not an identifier.
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

/// **The acceptance test for #1081.** `import gc` / `gc.disable()`, built
/// as an extension module, and the host observes `gc.isenabled()` go
/// `False` merely by importing it.
///
/// This is the capability PR 2a deliberately did not ship: every program 2a
/// admitted did nothing observable with the CPython object it loaded. Here
/// the compiled module body reaches into the hosting interpreter and
/// changes its state.
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn a_zero_argument_method_call_has_its_effect_in_the_host() {
    let dir = ScratchDir::new("foreign_call_gc_hosted").expect("scratch");
    build_ext(&dir, "pycc_gc_disable_mod", "import gc\n\ngc.disable()\n");
    let run = python(
        &dir,
        "import gc\n\
         assert gc.isenabled(), 'the host starts with gc enabled'\n\
         import pycc_gc_disable_mod\n\
         assert not gc.isenabled(), 'importing the artifact must have disabled gc'\n",
    );
    assert!(
        run.status.success(),
        "stdout: {}\nstderr: {}",
        stdout_of(&run),
        stderr_of(&run)
    );
}

/// The argument half of the same claim: an `int` argument really reaches
/// CPython with its value intact.
///
/// `gc.set_threshold` is chosen because its effect is readable back through
/// `gc.get_threshold()`, so this asserts the marshalled *value* rather than
/// only that a call happened.
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn an_int_argument_reaches_the_host_with_its_value() {
    let dir = ScratchDir::new("foreign_call_threshold_hosted").expect("scratch");
    build_ext(
        &dir,
        "pycc_gc_threshold_mod",
        "import gc\n\ngc.set_threshold(12345)\n",
    );
    let run = python(
        &dir,
        "import gc\n\
         import pycc_gc_threshold_mod\n\
         assert gc.get_threshold()[0] == 12345, gc.get_threshold()\n",
    );
    assert!(
        run.status.success(),
        "stdout: {}\nstderr: {}",
        stdout_of(&run),
        stderr_of(&run)
    );
}

/// A missing method surfaces as CPython's own `AttributeError`, and the
/// module body stops there.
///
/// The same edge PR 2a's `a_failed_attribute_lookup_raises_attribute_error_in_the_host`
/// asserts for a load, now for a call: `pycc_ext_obj_call` returns `NULL`
/// with CPython's exception set and `foreign_call::emit`'s `NULL` check
/// returns `-1` from the `Py_mod_exec` slot. The `print` below the call is
/// what proves the body stopped rather than merely reported.
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn a_missing_method_raises_attribute_error_in_the_host() {
    let dir = ScratchDir::new("foreign_call_missing_hosted").expect("scratch");
    build_ext(
        &dir,
        "pycc_missing_method_mod",
        "import gc\n\ngc.pycc_no_such_method_1081()\nprint(\"ran past the call\")\n",
    );
    let run = python(
        &dir,
        "try:\n\
         \x20   import pycc_missing_method_mod\n\
         except AttributeError as e:\n\
         \x20   assert 'pycc_no_such_method_1081' in str(e), str(e)\n\
         else:\n\
         \x20   raise AssertionError('the method call should have failed')\n",
    );
    assert!(
        run.status.success(),
        "stdout: {}\nstderr: {}",
        stdout_of(&run),
        stderr_of(&run)
    );
    assert!(
        !stdout_of(&run).contains("ran past the call"),
        "the module body must stop at the failed call: {}",
        stdout_of(&run)
    );
}

/// A call inside a module-scope loop does not grow the stack per iteration.
///
/// A top-level `for` is not a function body, so the call is emitted in
/// `pycc_ext_module_exec` -- and an `alloca`
/// emitted at the call site is only reclaimed when `pycc_ext_module_exec`
/// returns. Twenty million iterations segfaulted the hosting interpreter
/// until `alloca_in_entry_block` hoisted the argument array; the loop below
/// is the same shape with an argument, so it also proves the packers are
/// driven once per iteration against a single reused slot.
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn a_call_in_a_module_scope_loop_does_not_exhaust_the_stack() {
    let dir = ScratchDir::new("foreign_call_loop_hosted").expect("scratch");
    build_ext(
        &dir,
        "pycc_loop_call_mod",
        "import gc\n\nfor i in range(20000000):\n    gc.set_threshold(500)\n",
    );
    let run = python(
        &dir,
        "import pycc_loop_call_mod\n\
         import gc\n\
         assert gc.get_threshold()[0] == 500, gc.get_threshold()\n\
         print(\"survived the loop\")\n",
    );
    assert!(
        run.status.success(),
        "a module-scope loop must not exhaust the stack -- stdout: {}\nstderr: {}",
        stdout_of(&run),
        stderr_of(&run)
    );
    assert!(
        stdout_of(&run).contains("survived the loop"),
        "{}",
        stdout_of(&run)
    );
}

/// A method that *raises* surfaces its own exception, not an
/// `AttributeError` and not a `SystemError`.
///
/// `json.loads("not json")` also exercises the `str` packer end to end: the
/// `ValueError` only happens if the argument really arrived as that text.
/// (`math` is not a foreign module -- `pycc_std::resolve_module` answers
/// `StdModule::Math` for it, so `import math` never produces a
/// `Ty::Object`; every fixture here uses a module pycc has no native
/// implementation of.)
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn a_raising_method_surfaces_its_own_exception_in_the_host() {
    let dir = ScratchDir::new("foreign_call_raises_hosted").expect("scratch");
    build_ext(
        &dir,
        "pycc_raising_method_mod",
        "import json\n\njson.loads(\"not json\")\nprint(\"ran past the call\")\n",
    );
    let run = python(
        &dir,
        "try:\n\
         \x20   import pycc_raising_method_mod\n\
         except ValueError as e:\n\
         \x20   assert 'Expecting value' in str(e), str(e)\n\
         else:\n\
         \x20   raise AssertionError('the method call should have raised')\n",
    );
    assert!(
        run.status.success(),
        "stdout: {}\nstderr: {}",
        stdout_of(&run),
        stderr_of(&run)
    );
    assert!(
        !stdout_of(&run).contains("ran past the call"),
        "the module body must stop at the raising call: {}",
        stdout_of(&run)
    );
}

/// CPython resolves a call's callable before it evaluates the arguments, so
/// a missing method raises `AttributeError` even when an argument
/// expression would itself raise.
///
/// The regression pin for the review finding on PR 2b of #1081: while the
/// shim performed the attribute lookup itself, the generated code had
/// already evaluated every argument by the time the lookup ran, and this
/// program raised `ZeroDivisionError` instead. Only a hosted run can see
/// the difference -- both spellings build and both stop the module body.
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn a_missing_method_raises_before_a_failing_argument_does() {
    let dir = ScratchDir::new("foreign_call_order_hosted").expect("scratch");
    build_ext(
        &dir,
        "pycc_call_order_mod",
        "import gc

d: int = 0
gc.pycc_missing_1081(1 // d)
",
    );
    let run = python(
        &dir,
        "try:\n\
         \x20   import pycc_call_order_mod\n\
         except AttributeError as e:\n\
         \x20   assert 'pycc_missing_1081' in str(e), str(e)\n\
         except ZeroDivisionError as e:\n\
         \x20   raise AssertionError('arguments were evaluated before the lookup: ' + str(e))\n\
         else:\n\
         \x20   raise AssertionError('the missing method should have raised')\n",
    );
    assert!(
        run.status.success(),
        "stdout: {}\nstderr: {}",
        stdout_of(&run),
        stderr_of(&run)
    );
}

/// A `str` argument crosses as an independent CPython `str` with the right
/// contents, and the artifact stays usable afterwards.
///
/// The packer *borrows* its `PyStrObj` rather than consuming it (unlike the
/// `str` *result* packer, which consumes -- `docs/RUNTIME.md` owns the
/// distinction). Passing the same name to two calls is what would expose a
/// premature release: under a consuming packer the second call would read a
/// freed string.
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn a_str_argument_may_be_passed_twice_without_a_premature_release() {
    let dir = ScratchDir::new("foreign_call_str_hosted").expect("scratch");
    build_ext(
        &dir,
        "pycc_str_arg_mod",
        "import warnings\n\ns: str = \"pycc-1081\"\n\
         warnings.filterwarnings(\"ignore\", s)\n\
         warnings.filterwarnings(\"ignore\", s)\n\n\
         def answer() -> int:\n    return 42\n",
    );
    let run = python(
        &dir,
        "import warnings\n\
         import pycc_str_arg_mod as m\n\
         assert m.answer() == 42, m.answer()\n\
         assert any('pycc-1081' in str(f[1]) for f in warnings.filters), warnings.filters\n",
    );
    assert!(
        run.status.success(),
        "stdout: {}\nstderr: {}",
        stdout_of(&run),
        stderr_of(&run)
    );
}

/// An `int` argument outside the D-141 inline range reaches the host as an
/// `OverflowError` naming the range, and the module body stops there.
///
/// This is the one refusal the *type* checker cannot make: `n` is an `int`
/// like any other, and only its run-time word tells `pycc_rt_ext_int_classify`
/// it is a heap bigint. `pycc_ext_obj_pack_int` therefore raises rather than
/// truncating or aborting, on the same edge a failed call takes -- the shim
/// sees a `NULL` argument, skips the vectorcall entirely, and hands the
/// module-exec slot its `-1`. `#1040` is the issue
/// that would widen the boundary to a real bigint; until it lands this arm is
/// the documented boundary behavior, not a defect.
///
/// The C shim is outside `cargo llvm-cov`'s denominator, so this test is the
/// only thing that exercises the arm at all.
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn a_bigint_int_argument_raises_overflow_error_in_the_host() {
    let dir = ScratchDir::new("foreign_call_bigint_hosted").expect("scratch");
    build_ext(
        &dir,
        "pycc_bigint_arg_mod",
        // 2**62 -- the first value the inline range excludes.
        "import gc\n\nn: int = 4611686018427387904\ngc.set_threshold(n)\nprint(\"ran past the call\")\n",
    );
    let run = python(
        &dir,
        "try:\n\
         \x20   import pycc_bigint_arg_mod\n\
         except OverflowError as e:\n\
         \x20   assert '2**62' in str(e), str(e)\n\
         \x20   assert '#1040' in str(e), str(e)\n\
         else:\n\
         \x20   raise AssertionError('the bigint argument should have been refused')\n",
    );
    assert!(
        run.status.success(),
        "stdout: {}\nstderr: {}",
        stdout_of(&run),
        stderr_of(&run)
    );
    assert!(
        !stdout_of(&run).contains("ran past the call"),
        "the module body must stop at the refused call: {}",
        stdout_of(&run)
    );
}

/// Since #1223 a plain `pycc build` of a standard-library method call
/// builds an embedded executable that really calls into CPython:
/// `gc.disable()` then `gc.isenabled()` observes the effect. Since #1286 it
/// runs on the Windows leg too (a stub `m` plus `m.pycc\`, D-253); the
/// comparison applies D-082's `\r\n` normalization so either writer passes.
#[test]
#[ignore = "needs CPython 3.14.7 as python3.14 or PYCC_PYTHON; run with --include-ignored"]
fn a_standard_library_method_call_builds_embedded_and_runs() {
    let dir = ScratchDir::new("foreign_call_embedded").expect("scratch");
    let output = pycc()
        .arg("build")
        .arg(source(
            &dir,
            "import gc\n\ngc.disable()\nif gc.isenabled():\n    print(\"on\")\nelse:\n    print(\"off\")\n",
        ))
        .arg("-o")
        .arg(dir.join("m"))
        .output()
        .expect("pycc should spawn");
    assert!(output.status.success(), "{}", stderr_of(&output));
    assert!(dir.join("m.pycc").join("PYCC-BUNDLE").is_file());
    let run = Command::new(dir.join("m"))
        .output()
        .expect("the embedded binary runs");
    assert_eq!(run.status.code(), Some(0), "{}", stderr_of(&run));
    assert_eq!(stdout_of(&run).replace("\r\n", "\n"), "off\n");
}
