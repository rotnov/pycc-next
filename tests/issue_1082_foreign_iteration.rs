//! Part 3 of #1026 (PR 3c of #1082): `for` iteration over a CPython
//! object.
//!
//! `tests/issue_1080_foreign_object.rs` owns what a foreign `import`
//! binds and the refusal table around it;
//! `tests/issue_1081_foreign_method_call.rs` owns the method call,
//! `tests/issue_1082_foreign_len_and_truth.rs` owns `len` and truth
//! testing and `tests/issue_1082_foreign_subscript.rs` owns the subscript
//! load. This file owns the one operation PR 3c adds: `for x in o.attr:`
//! and `for x in o.method(...):`, the two iterable shapes admitted.
//!
//! The non-ignored tests here stop at `pycc check`, so they need no
//! CPython on `PATH`. The `#[ignore]`d hosted tests at the bottom
//! contribute no line coverage (CI's coverage job runs `llvm-cov` without
//! `--include-ignored`); every new Rust line is covered by the unit tests
//! in `crates/pycc_types/src/foreign/tests.rs`,
//! `crates/pycc_mir/src/tests/import.rs` and
//! `crates/pycc_codegen/src/foreign_call.rs`.

use pycc_scratch::ScratchDir;
use std::path::Path;
use std::process::{Command, Output};

fn pycc() -> Command {
    Command::new(env!("CARGO_BIN_EXE_pycc"))
}

/// Captured output with CRLF line endings folded to `\n`.
///
/// A single captured stream here can carry both conventions at once: the
/// compiled extension module writes `\n` verbatim, while the host CPython
/// driver's own `print` goes through CPython's text layer, which translates
/// `\n` to `\r\n` on Windows. `a_raising_iterator_stops_the_module_body`
/// compares a string spanning both printers, so a byte-exact `\n`
/// expectation is unsatisfiable there on `windows-latest`. Normalizing in
/// these two helpers rather than at the one assertion that happened to fail
/// covers every assertion in this file: the others pass today only because
/// their expected text does not straddle the interpreter's own output.
///
/// The sibling hosted suites (`tests/issue_1080_foreign_object.rs`,
/// `tests/issue_1081_foreign_method_call.rs`,
/// `tests/issue_1082_foreign_len_and_truth.rs`,
/// `tests/issue_1082_foreign_subscript.rs`) each define a bare
/// `String::from_utf8_lossy` pair and normalize nothing, so there was no
/// existing convention to follow; this is the first. Whether a
/// pycc-compiled `print` should translate line endings the way CPython's
/// does is a separate compiler-compatibility question, tracked on its own.
fn normalize_newlines(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).replace("\r\n", "\n")
}

fn stdout_of(output: &Output) -> String {
    normalize_newlines(&output.stdout)
}

fn stderr_of(output: &Output) -> String {
    normalize_newlines(&output.stderr)
}

fn check(dir: &Path, body: &str) -> Output {
    let src = dir.join("m.py");
    std::fs::write(&src, body).expect("write the fixture source");
    pycc()
        .arg("check")
        .arg(&src)
        .output()
        .expect("pycc should spawn")
}

/// Both admitted iterable shapes type-check at module scope.
///
/// The loop variable is a CPython object, and `check_assignment` still
/// refuses binding one to a name, so `len(x)` and a bare `pass` are the
/// body shapes a whole program can actually contain today.
#[test]
fn both_admitted_iterable_shapes_are_admitted() {
    let dir = ScratchDir::new("foreign_iteration_admitted").expect("scratch");
    for body in [
        "import gc\n\nfor x in gc.garbage:\n    pass\n",
        "import gc\n\nfor x in gc.garbage:\n    print(len(x))\n",
        "import gc\n\nfor x in gc.get_objects():\n    pass\n",
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

/// The iterable shapes PR 3c deliberately leaves out, each with the
/// diagnostic that owns it.
///
/// A subscript iterable (`for x in o[k]:`) and a tuple iterable are both
/// `pycc_hir`'s pre-existing `C0001`, word for word -- the two new routes
/// were added as guards *ahead* of the `let ... else` bindings that
/// produce them, so a regression that moved either diagnostic into the
/// type checker fails here. A bare foreign name is an `Expr::Name`
/// iterable; since #1325 it checks clean like any other `object` name, and
/// CPython's own `TypeError` answers a module object at run time.
#[test]
fn the_deferred_iterable_shapes_keep_their_own_refusals() {
    let dir = ScratchDir::new("foreign_iteration_refused").expect("scratch");
    for (body, code, phrase) in [
        (
            "import gc\n\nfor x in gc.garbage[0]:\n    pass\n",
            "C0001",
            "got a subscript expression (`obj[key]`) as the iterable",
        ),
        (
            "for x in (1, 2):\n    pass\n",
            "C0001",
            "got a tuple as the iterable",
        ),
    ] {
        let out = check(&dir, body);
        assert!(!out.status.success(), "{body}: {}", stdout_of(&out));
        let text = format!("{}{}", stdout_of(&out), stderr_of(&out));
        assert!(text.contains(code), "{body}: {text}");
        assert!(text.contains(phrase), "{body}: {text}");
    }
    let out = check(&dir, "import gc\n\nfor x in gc:\n    pass\n");
    assert!(out.status.success(), "{}", stdout_of(&out));
}

/// The loop stays module-body only, and the loop variable is only
/// *maybe* bound once the loop ends.
///
/// #1316 admits reading a foreign name inside a function body, but not
/// iterating it. Part 1 of #1333 admits a function-local `object`
/// binding, but a function-body `for` over an object is Part 2 (#1363),
/// so the loop stays refused there. And a loop that runs zero times never writes
/// its variable's slot, so a read after the loop is `T0041` rather than a
/// load of whatever the slot happened to hold.
#[test]
fn the_loop_inherits_the_positional_bound_and_binds_only_maybe() {
    let dir = ScratchDir::new("foreign_iteration_bounds").expect("scratch");
    for (body, code) in [
        (
            "import gc\n\n\ndef _n() -> int:\n    for x in gc.garbage:\n        print(len(x))\n    return 1\n\n\nprint(_n())\n",
            "I0404",
        ),
        (
            "import gc\n\nfor x in gc.garbage:\n    pass\n\nprint(len(x))\n",
            "T0041",
        ),
    ] {
        let out = check(&dir, body);
        assert!(!out.status.success(), "{body}: {}", stdout_of(&out));
        let text = format!("{}{}", stdout_of(&out), stderr_of(&out));
        assert!(text.contains(code), "{body}: {text}");
    }
}

/// Builds `body` as an extension module named `module` inside `dir`.
///
/// The output path carries **no** suffix: `pycc build --ext` appends the
/// one its target triple calls for.
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

/// Writes the host-side helper module `dir/host_only/pycc_iter_helper.py`.
///
/// It lives in a **subdirectory** of the scratch dir rather than beside
/// the fixture source: a `.py` next to the source is resolved as a
/// *project* import, which is still `C0001`, so it would never reach the
/// foreign-object path these tests exist to exercise. Putting it out of
/// the driver's reach and onto the host's `sys.path` at run time is what
/// makes the import foreign at compile time and resolvable at run time.
fn write_helper(dir: &Path) {
    let helper_dir = dir.join("host_only");
    std::fs::create_dir_all(&helper_dir).expect("create the helper directory");
    std::fs::write(
        helper_dir.join("pycc_iter_helper.py"),
        "items = [\"a\", \"bb\", \"ccc\"]\n\
         not_iterable = 1\n\
         \n\
         \n\
         def raising():\n\
         \x20   yield \"a\"\n\
         \x20   raise ValueError(\"boom\")\n",
    )
    .expect("write the helper module");
}

/// **The acceptance test for PR 3c.** A built extension module iterates a
/// real host list and sees every element.
///
/// The body consumes each item with `len`, and the three lengths differ,
/// so one hard-coded answer cannot satisfy all three -- that is what
/// proves the header block really wrote `*out` on each `1` from
/// `pycc_ext_obj_iter_next`. The trailing `99` proves the `0` return
/// (clean exhaustion) left the module body running rather than taking the
/// failure edge.
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn a_for_loop_over_a_host_sequence_sees_every_element() {
    let dir = ScratchDir::new("foreign_iteration_hosted").expect("scratch");
    write_helper(&dir);
    build_ext(
        &dir,
        "pycc_iter_mod",
        "import pycc_iter_helper\n\
         \n\
         for x in pycc_iter_helper.items:\n\
         \x20   print(len(x))\n\
         print(99)\n",
    );
    let run = python(
        &dir,
        "import sys\n\
         sys.path.insert(0, 'host_only')\n\
         import pycc_iter_mod\n",
    );
    assert!(
        run.status.success(),
        "stdout: {}\nstderr: {}",
        stdout_of(&run),
        stderr_of(&run)
    );
    assert_eq!(
        stdout_of(&run),
        "1\n2\n3\n99\n",
        "stderr: {}",
        stderr_of(&run)
    );
}

/// A non-iterable iterable surfaces CPython's own `TypeError`, from the
/// first of PR 3c's two `EXT_MODULE_EXEC_FAILED` edges.
///
/// `PyObject_GetIter` raises, `pycc_ext_obj_get_iter` returns `NULL` with
/// the exception set, and the `Py_mod_exec` slot returns `-1` without
/// clearing it. The absent `99` is what proves the body stopped rather
/// than merely reported.
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn a_non_iterable_raises_type_error_in_the_host() {
    let dir = ScratchDir::new("foreign_iteration_type_error_hosted").expect("scratch");
    write_helper(&dir);
    build_ext(
        &dir,
        "pycc_iter_type_error_mod",
        "import pycc_iter_helper\n\
         \n\
         for x in pycc_iter_helper.not_iterable:\n\
         \x20   print(len(x))\n\
         print(99)\n",
    );
    let run = python(
        &dir,
        "import sys\n\
         sys.path.insert(0, 'host_only')\n\
         try:\n\
         \x20   import pycc_iter_type_error_mod\n\
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

/// An iterator that raises *mid-iteration* surfaces its own exception,
/// from the second of PR 3c's two `EXT_MODULE_EXEC_FAILED` edges.
///
/// This is the case the three-valued `pycc_ext_obj_iter_next` exists for:
/// `PyIter_Next` answers `NULL` both at clean exhaustion and on an error,
/// and only `PyErr_Occurred()` tells the two apart. The generator yields
/// one item first, so the `1\n` below proves the loop really ran a
/// trip before the `-1` edge was taken, and the absent `99` proves the
/// error was not mistaken for exhaustion.
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn a_raising_iterator_stops_the_module_body() {
    let dir = ScratchDir::new("foreign_iteration_raising_hosted").expect("scratch");
    write_helper(&dir);
    build_ext(
        &dir,
        "pycc_iter_raising_mod",
        "import pycc_iter_helper\n\
         \n\
         for x in pycc_iter_helper.raising():\n\
         \x20   print(len(x))\n\
         print(99)\n",
    );
    let run = python(
        &dir,
        "import sys\n\
         sys.path.insert(0, 'host_only')\n\
         try:\n\
         \x20   import pycc_iter_raising_mod\n\
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
    assert_eq!(
        stdout_of(&run),
        "1\nValueError boom\n",
        "stderr: {}",
        stderr_of(&run)
    );
}
