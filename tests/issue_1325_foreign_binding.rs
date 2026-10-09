//! #1325: a module-level name bound to a CPython object value -- `x =
//! product("ab", "c")` after `from itertools import product` -- holds the
//! object itself, and a bare-name `for t in x:` iterates it through
//! CPython's own iterator protocol, so the items and any raised exception
//! are CPython's own.
//!
//! Every hosted test compares against the host interpreter's own run of the
//! same source. The hosted tests are `#[ignore]`d and contribute no line
//! coverage; the Tier-1 `native-build-test` leg runs them with
//! `cargo test --workspace -- --include-ignored`. The changed lines are
//! covered by the non-ignored tests here and by the unit tests in
//! `crates/pycc_types/src/foreign/binding_tests.rs`,
//! `crates/pycc_mir/src/tests/obj_bind.rs` and
//! `crates/pycc_codegen/src/tests/object_binding.rs`.

use pycc_scratch::ScratchDir;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn pycc() -> Command {
    Command::new(env!("CARGO_BIN_EXE_pycc"))
}

fn host_python() -> Command {
    Command::new(std::env::var_os("PYCC_PYTHON").unwrap_or_else(|| "python3".into()))
}

fn stdout_of(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).replace("\r\n", "\n")
}

fn stderr_of(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).replace("\r\n", "\n")
}

/// Writes `body` to `dir/<file>` and returns the path.
fn write(dir: &Path, file: &str, body: &str) -> PathBuf {
    let path = dir.join(file);
    std::fs::write(&path, body).expect("write the fixture source");
    path
}

fn check_with(dir: &Path, body: &str) -> Output {
    pycc()
        .arg("check")
        .arg(write(dir, "m.py", body))
        .output()
        .expect("pycc should spawn")
}

/// `pycc check` of `body` fails with exactly one `code` diagnostic whose
/// text contains `needle`.
fn assert_one_error(tag: &str, body: &str, code: &str, needle: &str) {
    let dir = ScratchDir::new(tag).expect("scratch");
    let output = check_with(&dir, body);
    assert_eq!(output.status.code(), Some(1), "{}", stderr_of(&output));
    let rendered = stdout_of(&output);
    assert_eq!(rendered.matches("error[").count(), 1, "{rendered}");
    assert!(rendered.contains(&format!("error[{code}]")), "{rendered}");
    assert!(rendered.contains(needle), "{rendered}");
}

/// The success program: bind a call result, iterate it by bare name, loop
/// again over the now-exhausted iterator (which prints nothing), alias it
/// to a second name that a truth test reads, then rebind the name to a
/// second producer and iterate that.
const SUCCESS: &str = "from itertools import product\n\
    x = product(\"ab\", \"c\")\n\
    for t in x:\n    print(str(t))\n\
    for t in x:\n    print(str(t))\n\
    y = x\n\
    print(bool(y))\n\
    x = product(\"d\", \"ef\")\n\
    for t in x:\n    print(str(t))\n";

/// What `SUCCESS` prints under CPython.
const SUCCESS_OUT: &str = "('a', 'c')\n('b', 'c')\nTrue\n('d', 'e')\n('d', 'f')\n";

#[test]
fn check_accepts_a_module_level_object_binding_and_loop() {
    let dir = ScratchDir::new("obj_bind_check").expect("scratch");
    for body in [
        SUCCESS,
        "import sys\nfor q in sys:\n    pass\n",
        "import sys\np = sys.path\nfor q in p:\n    pass\n",
    ] {
        let output = check_with(&dir, body);
        assert_eq!(
            output.status.code(),
            Some(0),
            "{body}{}{}",
            stdout_of(&output),
            stderr_of(&output)
        );
    }
}

/// Rebinding a native name to an object is the ordinary redefinition
/// refusal (the other order boxes the native value since #1475,
/// `tests/issue_1475_object_boxing.rs`), and a comprehension over the bound
/// name keeps its `I0404`. The function-body binding this test used to
/// refuse is admitted since Part 1 of #1333; see
/// `tests/issue_1333_foreign_in_function.rs`.
#[test]
fn the_shapes_outside_the_admitted_binding_are_refused() {
    assert_one_error(
        "obj_bind_retype",
        "from itertools import product\nx = 1\nx = product(\"ab\")\n",
        "T0023",
        "cannot assign `object` to `x`, previously inferred as `int`",
    );
    assert_one_error(
        "obj_bind_comprehension",
        "from itertools import product\nx = product(\"ab\")\nys = [t for t in x]\n",
        "I0404",
        "using `x`, which is bound to a CPython object",
    );
}

/// Builds `body` as the extension module `module` in `dir` (the source
/// stays at `dir/m.py`, where the oracle runs it too).
fn build_ext(dir: &Path, module: &str, body: &str) {
    let build = pycc()
        .arg("build")
        .arg(write(dir, "m.py", body))
        .arg("-o")
        .arg(dir.join(module))
        .arg("--ext")
        .output()
        .expect("pycc should spawn");
    assert!(build.status.success(), "{}", stderr_of(&build));
}

fn python(dir: &Path, script: &str) -> Output {
    host_python()
        .arg("-c")
        .arg(script)
        .current_dir(dir)
        .output()
        .expect("python3 should spawn")
}

fn assert_ok(run: &Output) {
    assert!(
        run.status.success(),
        "stdout: {}\nstderr: {}",
        stdout_of(run),
        stderr_of(run)
    );
}

/// Runs `target` and prints what it raised -- any exception, by type and
/// `str(e)` -- after whatever the module body printed.
fn raised_report(target: &str) -> String {
    format!(
        "import runpy\n\
         try:\n\
         \x20   {target}\n\
         except Exception as e:\n\
         \x20   print(type(e).__name__, e)\n\
         else:\n\
         \x20   print('no error')\n"
    )
}

/// Builds `body`, then asserts the extension's import report matches
/// CPython's own run of the same source, and returns it.
fn assert_matches_cpython(tag: &str, module: &str, body: &str) -> String {
    let dir = ScratchDir::new(tag).expect("scratch");
    build_ext(&dir, module, body);
    let compiled = python(&dir, &raised_report(&format!("import {module}")));
    assert_ok(&compiled);
    let oracle = python(&dir, &raised_report("runpy.run_path('m.py')"));
    assert_ok(&oracle);
    assert_eq!(stdout_of(&compiled), stdout_of(&oracle));
    stdout_of(&compiled)
}

#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn a_bound_object_iterates_like_cpython_in_the_host() {
    let out = assert_matches_cpython("obj_bind_hosted", "pycc_obj_bind_mod", SUCCESS);
    assert_eq!(out, format!("{SUCCESS_OUT}no error\n"));
}

/// A raising producer surfaces CPython's own exception from the import,
/// before the name is ever bound.
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn a_raising_binding_raises_cpythons_exception_in_the_host() {
    let out = assert_matches_cpython(
        "obj_bind_raises",
        "pycc_obj_bind_raises_mod",
        "from itertools import product\nx = product(1, 2)\n",
    );
    assert_eq!(out, "TypeError 'int' object is not iterable\n");
}

/// The bare-name loop is admitted by type, so iterating a bare module
/// compiles and raises CPython's own `TypeError` at run time.
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn iterating_a_bare_module_raises_cpythons_exception_in_the_host() {
    let out = assert_matches_cpython(
        "obj_bind_module_iter",
        "pycc_obj_bind_module_iter_mod",
        "import sys\nfor q in sys:\n    pass\n",
    );
    assert_eq!(out, "TypeError 'module' object is not iterable\n");
}

/// A dependency module's `object` binding is importable by the entry
/// module and iterates there like CPython. `dep.py` is compiled into the
/// extension: the oracle runs first, then `dep.py` and any `__pycache__`
/// are removed before the extension is imported, so a run-time import of
/// `dep` by CPython would fail with `ModuleNotFoundError` instead of
/// producing the same output.
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn a_dependency_module_s_object_binding_iterates_in_the_host() {
    let dir = ScratchDir::new("obj_bind_two_modules").expect("scratch");
    let dep = write(
        &dir,
        "dep.py",
        "from itertools import product\nx = product(\"ab\", \"c\")\n",
    );
    let body = "from dep import x\nfor t in x:\n    print(str(t))\n";
    build_ext(&dir, "pycc_obj_bind_two_mod", body);
    let oracle = host_python()
        .arg("m.py")
        .current_dir(&*dir)
        .output()
        .expect("python3 should spawn");
    assert_ok(&oracle);
    std::fs::remove_file(&dep).expect("remove dep.py");
    let cache = dir.join("__pycache__");
    if cache.exists() {
        std::fs::remove_dir_all(&cache).expect("remove __pycache__");
    }
    let compiled = python(&dir, "import pycc_obj_bind_two_mod\n");
    assert_ok(&compiled);
    assert_eq!(stdout_of(&compiled), stdout_of(&oracle));
    assert_eq!(stdout_of(&compiled), "('a', 'c')\n('b', 'c')\n");
}

/// A plain (embedded) build compiles its module with `ext` set, so the
/// binding and loop run there too, matching CPython's own output.
#[cfg(not(windows))]
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn an_embedded_build_binds_and_iterates_like_cpython() {
    let dir = ScratchDir::new("obj_bind_embedded").expect("scratch");
    let source = write(&dir, "m.py", SUCCESS);
    let build = pycc()
        .arg("build")
        .arg(&source)
        .arg("-o")
        .arg(dir.join("app"))
        .output()
        .expect("pycc should spawn");
    assert!(build.status.success(), "{}", stderr_of(&build));
    let embedded = Command::new(dir.join("app"))
        .output()
        .expect("the embedded binary runs");
    assert_ok(&embedded);
    let oracle = host_python()
        .arg(&source)
        .output()
        .expect("python3 should spawn");
    assert_ok(&oracle);
    assert_eq!(stdout_of(&embedded), stdout_of(&oracle));
    assert_eq!(stdout_of(&embedded), SUCCESS_OUT);
}
