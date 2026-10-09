//! Part 3 of #1026 (PR 3b of #1082): a subscript *load* on a CPython
//! object.
//!
//! `tests/issue_1080_foreign_object.rs` owns what a foreign `import` binds
//! and the refusal table around it; `tests/issue_1081_foreign_method_call.rs`
//! owns the method call and `tests/issue_1082_foreign_len_and_truth.rs`
//! owns `len` and truth testing. This file owns the one operation PR 3b
//! adds: `o[k]` for a key that is an `int`, `float`, `bool` or `str`.
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

fn stdout_of(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn stderr_of(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
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

/// Every admitted key type type-checks, in the one statement position the
/// result may occupy.
///
/// The result is a CPython object, and `check_assignment` still refuses
/// binding one to a name, so a discarded load and a `len` of the load are
/// the two shapes a whole program can actually contain today.
#[test]
fn a_subscript_load_with_each_admitted_key_type_is_admitted() {
    let dir = ScratchDir::new("foreign_subscript_admitted").expect("scratch");
    for body in [
        "import gc\n\ngc.garbage[0]\n",
        "import gc\n\ngc.garbage[1.5]\n",
        "import gc\n\ngc.garbage[True]\n",
        "import gc\n\ngc.garbage[\"k\"]\n",
        "import gc\n\nprint(len(gc.garbage[0]))\n",
        "import gc\n\nif gc.garbage[0]:\n    print(1)\n",
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

/// The neighbouring shapes PR 3b deliberately left refused, each
/// with the diagnostic that owns it.
///
/// A key with no `pycc_ext_obj_pack_*` helper is the new `I0404`; a
/// *store* is `C0001` from HIR lowering, and so is a `del` of an item. (A
/// slice *load* was a `T0033` row here until Part 2b of #1371 admitted it,
/// `tests/issue_1371_object_membership_slice.rs`, and a `del` of a slice a
/// `C0001` row until Part 2c did, `tests/issue_1371_object_slice_del.rs`.) Splitting them out this
/// way is what pins that admitting the load did not widen any of them. Binding the
/// result to a module-level name was a fourth row until #1325 admitted it
/// (`tests/issue_1325_foreign_binding.rs`).
#[test]
fn the_neighbouring_subscript_shapes_keep_their_own_refusals() {
    let dir = ScratchDir::new("foreign_subscript_refused").expect("scratch");
    for (body, code, phrase) in [
        (
            "import gc\n\ngc.garbage[None]\n",
            "I0404",
            "indexing a CPython object with a `None` key",
        ),
        (
            "import gc\n\ngc.garbage[0] = 1\n",
            "C0001",
            "only assigning to a bare-name subscript target",
        ),
        (
            "import gc\n\ndel gc.garbage[0]\n",
            "C0001",
            "a `del` of a subscript",
        ),
    ] {
        let out = check(&dir, body);
        assert!(!out.status.success(), "{body}: {}", stdout_of(&out));
        let text = format!("{}{}", stdout_of(&out), stderr_of(&out));
        assert!(text.contains(code), "{body}: {text}");
        assert!(text.contains(phrase), "{body}: {text}");
    }
}

/// The load is admitted inside a function body since #1316, which gave
/// every pycc function a failure edge for a raising `PyObject_GetItem` to
/// take (`crates/pycc_codegen/src/foreign_fail.rs`).
#[test]
fn a_subscript_load_is_admitted_in_a_function_body() {
    let dir = ScratchDir::new("foreign_subscript_positional").expect("scratch");
    let out = check(
        &dir,
        "import gc\n\n\ndef _n() -> int:\n    return len(gc.garbage[0])\n\n\nprint(_n())\n",
    );
    assert!(out.status.success(), "{}", stdout_of(&out));
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

/// Writes the host-side helper module `dir/host_only/pycc_sub_helper.py`.
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
        helper_dir.join("pycc_sub_helper.py"),
        "mapping = {\"k\": \"value\"}\n\
         items = [\"a\", \"bb\", \"ccc\"]\n",
    )
    .expect("write the helper module");
}

/// **The acceptance test for PR 3b.** A built extension module reads a
/// real mapping by a `str` key and a real sequence by an `int` key.
///
/// Both results are consumed by `len`, which is what proves the shim
/// returned the actual element rather than something merely non-`NULL`:
/// the two lengths differ, so one hard-coded answer cannot satisfy both.
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn a_subscript_load_reaches_the_host_with_both_key_kinds() {
    let dir = ScratchDir::new("foreign_subscript_hosted").expect("scratch");
    write_helper(&dir);
    build_ext(
        &dir,
        "pycc_subscript_mod",
        "import pycc_sub_helper\n\
         \n\
         print(len(pycc_sub_helper.mapping[\"k\"]))\n\
         print(len(pycc_sub_helper.items[1]))\n",
    );
    let run = python(
        &dir,
        "import sys\n\
         sys.path.insert(0, 'host_only')\n\
         import pycc_subscript_mod\n",
    );
    assert!(
        run.status.success(),
        "stdout: {}\nstderr: {}",
        stdout_of(&run),
        stderr_of(&run)
    );
    assert_eq!(stdout_of(&run), "5\n2\n", "stderr: {}", stderr_of(&run));
}

/// A missing key surfaces CPython's own `KeyError`, and the module body
/// stops there.
///
/// The one new `EXT_MODULE_EXEC_FAILED` edge `foreign_call::emit_subscript`
/// emits, end to end: `PyObject_GetItem` raises, `pycc_ext_obj_getitem`
/// returns `NULL` with the exception set, and the `Py_mod_exec` slot
/// returns `-1` without clearing it. The absent `99` below is what proves
/// the body stopped rather than merely reported.
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn a_missing_key_raises_key_error_in_the_host() {
    let dir = ScratchDir::new("foreign_subscript_missing_hosted").expect("scratch");
    write_helper(&dir);
    build_ext(
        &dir,
        "pycc_subscript_missing_mod",
        "import pycc_sub_helper\n\
         \n\
         print(len(pycc_sub_helper.mapping[\"absent\"]))\n\
         print(99)\n",
    );
    let run = python(
        &dir,
        "import sys\n\
         sys.path.insert(0, 'host_only')\n\
         try:\n\
         \x20   import pycc_subscript_missing_mod\n\
         except KeyError as e:\n\
         \x20   print('KeyError', e)\n\
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
        stdout_of(&run).starts_with("KeyError"),
        "{}",
        stdout_of(&run)
    );
    assert!(!stdout_of(&run).contains("99"), "{}", stdout_of(&run));
}
