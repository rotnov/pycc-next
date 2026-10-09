//! #1278: an unaliased, top-level `from <undotted module> import a, b` of a
//! CPython module binds each name to the CPython object `module.<name>`,
//! fetched with CPython's own `IMPORT_NAME`/`IMPORT_FROM` semantics, so
//! `from itertools import product` behaves as it does under CPython. Part 1
//! of #1138 extends the channel to a dotted module
//! (`tests/issue_1138_dotted_foreign_from_import.rs`).
//!
//! Every hosted test compares against the host interpreter's own run of the
//! same source rather than against a transcribed expectation where the two
//! can differ (a file path in a message). The hosted tests are `#[ignore]`d
//! and contribute no line coverage; the Tier-1 `native-build-test` leg runs
//! them with `cargo test --workspace -- --include-ignored`. The lines this
//! change needs covered are covered by the non-ignored tests here and by the
//! unit tests in `crates/pycc_hir/src/import/tests/from_foreign.rs`,
//! `crates/pycc_hir/src/program/tests.rs`, `src/foreign_import.rs`,
//! `src/interop_policy/tests.rs` and `crates/pycc_codegen/src/foreign_import.rs`.

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

fn check_with(dir: &Path, body: &str, extra: &[&str]) -> Output {
    pycc()
        .arg("check")
        .args(extra)
        .arg(write(dir, "m.py", body))
        .output()
        .expect("pycc should spawn")
}

/// `pycc check` of `body` fails with exactly one `code` diagnostic whose
/// text contains `needle`.
fn assert_one_error(tag: &str, body: &str, extra: &[&str], code: &str, needle: &str) {
    let dir = ScratchDir::new(tag).expect("scratch");
    let output = check_with(&dir, body, extra);
    assert_eq!(output.status.code(), Some(1), "{}", stderr_of(&output));
    let rendered = stdout_of(&output);
    assert_eq!(rendered.matches("error[").count(), 1, "{rendered}");
    assert!(rendered.contains(&format!("error[{code}]")), "{rendered}");
    assert!(rendered.contains(needle), "{rendered}");
}

#[test]
fn check_accepts_a_foreign_from_import() {
    let dir = ScratchDir::new("from_foreign_check").expect("scratch");
    let output = check_with(
        &dir,
        "from itertools import product, chain\nprint(str(product.__name__))\n",
        &[],
    );
    assert_eq!(
        output.status.code(),
        Some(0),
        "{}{}",
        stdout_of(&output),
        stderr_of(&output)
    );
}

/// A name pycc already resolves by its spelling keeps a `C0001`: binding it
/// to a CPython object would change what every later use of it means.
#[test]
fn a_builtin_or_marker_spelling_is_refused() {
    assert_one_error(
        "from_foreign_builtin",
        "from builtins import range\n",
        &[],
        "C0001",
        "binding the CPython object `builtins.range` to `range`",
    );
    assert_one_error(
        "from_foreign_marker",
        // `from numpy import ndarray` is a buffer-carrier import since
        // #1380; the other module's spelling is not.
        "from numpy import NDArray\n",
        &[],
        "C0001",
        "binding the CPython object `numpy.NDArray` to `NDArray`",
    );
}

#[test]
fn the_shapes_outside_this_channel_keep_their_c0001() {
    for (tag, body, needle) in [
        (
            "from_foreign_alias",
            "from itertools import product as p\n",
            "`from ... import x as y` aliasing is not supported yet",
        ),
        (
            "from_foreign_wildcard",
            "from itertools import *\n",
            "`from ... import *` (wildcard import) is not supported yet",
        ),
        (
            // A module-level `if`/`try` body admits it since #1383
            // (`tests/issue_1383_block_from_import.rs`); a function body
            // does not.
            "from_foreign_function",
            "def f() -> None:\n    from itertools import product\n",
            "an `import` inside a function or block body",
        ),
        (
            "from_foreign_shadow",
            "import copy\nfrom copy import copy\n",
            "shadowing a foreign import is not supported yet",
        ),
    ] {
        assert_one_error(tag, body, &[], "C0001", needle);
    }
}

/// The bound name is an opaque object like any other foreign binding: a
/// function body may read it (#1316) and, since Part 1 of #1333, bind it to
/// a local, while a module-level read above the import is unbound.
#[test]
fn the_bound_name_follows_the_foreign_object_rules() {
    let dir = ScratchDir::new("from_foreign_fn_bind").expect("scratch");
    let output = check_with(
        &dir,
        "from itertools import product\n\n\ndef f() -> None:\n    print(str(product))\n    p = product\n",
        &[],
    );
    assert_eq!(
        output.status.code(),
        Some(0),
        "{}{}",
        stdout_of(&output),
        stderr_of(&output)
    );
    assert_one_error(
        "from_foreign_early_read",
        "print(str(product))\nfrom itertools import product\n",
        &[],
        "T0021",
        "name `product` is not defined",
    );
}

/// One statement is one `I0402`, quoting the statement as written, however
/// many names it imports.
#[test]
fn a_denied_from_import_is_one_diagnostic_quoting_the_statement() {
    assert_one_error(
        "from_foreign_deny",
        "from json import dumps, loads\n",
        &["--interop-policy", "deny"],
        "I0402",
        "`from json import dumps, loads` is a CPython-backed import",
    );
}

/// A native build cannot embed `tkinter`: one `I0403` for the statement.
#[test]
fn a_native_build_refuses_a_from_import_of_an_excluded_root_once() {
    let dir = ScratchDir::new("from_foreign_native").expect("scratch");
    let output = pycc()
        .arg("build")
        .arg(write(&dir, "m.py", "from tkinter import Tk, Label\n"))
        .arg("-o")
        .arg(dir.join("m"))
        .output()
        .expect("pycc should spawn");
    assert_eq!(output.status.code(), Some(1), "{}", stderr_of(&output));
    let rendered = stderr_of(&output);
    assert_eq!(rendered.matches("error[I0403]").count(), 1, "{rendered}");
    assert!(
        rendered.contains("`from tkinter import Tk, Label` imports a standard-library module"),
        "{rendered}"
    );
}

/// A non-standard root still needs a lock in an embedded build (#1242); the
/// from form classifies by its module exactly as `import numpy` does.
#[test]
fn an_embedded_build_of_a_non_stdlib_from_import_needs_the_lock() {
    let dir = ScratchDir::new("from_foreign_lock").expect("scratch");
    let output = pycc()
        .arg("build")
        .arg(write(&dir, "m.py", "from numpy import array\n"))
        .arg("-o")
        .arg(dir.join("m"))
        .output()
        .expect("pycc should spawn");
    assert_eq!(output.status.code(), Some(2), "{}", stderr_of(&output));
    let rendered = stderr_of(&output);
    assert!(
        rendered.contains("imports `numpy` from outside the standard library"),
        "{rendered}"
    );
    assert!(rendered.contains("run `pycc lock"), "{rendered}");
}

/// A project module's own from-import is admitted, and importing a project
/// name next to it still works; re-exporting the foreign name is refused.
#[test]
fn a_project_module_s_from_import_links_but_is_not_re_exported() {
    let dir = ScratchDir::new("from_foreign_two_modules").expect("scratch");
    write(&dir, "dep.py", "from itertools import product\nX = 1\n");
    let admitted = pycc()
        .arg("check")
        .arg(write(
            &dir,
            "main.py",
            "from dep import X\nfrom itertools import product\nprint(X)\n",
        ))
        .output()
        .expect("pycc should spawn");
    assert_eq!(admitted.status.code(), Some(0), "{}", stdout_of(&admitted));
    let refused = pycc()
        .arg("check")
        .arg(write(&dir, "main.py", "from dep import product\n"))
        .output()
        .expect("pycc should spawn");
    assert_eq!(refused.status.code(), Some(1));
    assert!(
        stdout_of(&refused).contains(
            "dep.py` binds `product` to the CPython object `itertools.product`; re-exporting a \
             foreign import across project modules is not supported yet"
        ),
        "{}",
        stdout_of(&refused)
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

/// Runs `target` and prints what it raised -- the type, `str(e)`, `.name`,
/// `.path` and `.name_from` -- after whatever the module body printed.
fn raised_report(target: &str) -> String {
    format!(
        "import runpy\n\
         try:\n\
         \x20   {target}\n\
         except ImportError as e:\n\
         \x20   print(type(e).__name__, str(e), e.name, e.path, getattr(e, 'name_from', None))\n\
         else:\n\
         \x20   print('no error')\n"
    )
}

/// Builds `body`, then compares the extension's import against CPython's
/// own run of the same source: stdout, including the raised report, must
/// match byte for byte. Returns that stdout.
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
fn a_from_import_binds_the_cpython_object_in_the_host() {
    let out = assert_matches_cpython(
        "from_foreign_hosted",
        "pycc_from_itertools_mod",
        "from itertools import product, chain\n\
         print(str(product))\nprint(str(product.__name__))\nprint(str(chain.__name__))\n",
    );
    assert_eq!(
        out,
        "<class 'itertools.product'>\nproduct\nchain\nno error\n"
    );
}

/// A dependency module's from-import runs when the linked program executes
/// the dependency's body, as in CPython: `dep.py` is lark's own shape
/// (`lark/utils.py` is a dependency module), and the entry observes the
/// foreign object only through a native `str` the dependency exports.
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn a_dependency_module_s_from_import_runs_in_the_host() {
    let dir = ScratchDir::new("from_foreign_two_modules_hosted").expect("scratch");
    write(
        &dir,
        "dep.py",
        "from itertools import product\n\
         nm = str(product.__name__)\n\n\n\
         def name() -> str:\n    return nm\n",
    );
    build_ext(
        &dir,
        "pycc_from_two_modules_mod",
        "from dep import name\nprint(name())\n",
    );
    let compiled = python(&dir, &raised_report("import pycc_from_two_modules_mod"));
    assert_ok(&compiled);
    let oracle = python(&dir, &raised_report("runpy.run_path('m.py')"));
    assert_ok(&oracle);
    assert_eq!(stdout_of(&compiled), stdout_of(&oracle));
    assert_eq!(stdout_of(&compiled), "product\nno error\n");
}

#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn a_from_import_of_functions_binds_each_in_the_host() {
    let out = assert_matches_cpython(
        "from_foreign_copy",
        "pycc_from_copy_mod",
        "from copy import copy, deepcopy\n\
         print(str(copy.__name__))\nprint(str(deepcopy.__name__))\n",
    );
    assert_eq!(out, "copy\ndeepcopy\nno error\n");
}

/// A package's submodule is bound through the fromlist import and the
/// `sys.modules` fallback, as `IMPORT_FROM` binds it.
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn a_from_import_binds_a_package_submodule_in_the_host() {
    let out = assert_matches_cpython(
        "from_foreign_submodule",
        "pycc_from_xml_mod",
        "from xml import dom\nprint(str(dom.__name__))\n",
    );
    assert_eq!(out, "xml.dom\nno error\n");
}

/// A missing name raises CPython's own `ImportError`: the message, `.name`,
/// `.path` and `.name_from` all match, for a module without a file
/// (`unknown location`) and for one with a file.
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn a_missing_name_raises_cpythons_import_error_in_the_host() {
    let builtin = assert_matches_cpython(
        "from_foreign_missing_builtin",
        "pycc_from_missing_builtin_mod",
        "from itertools import nope\n",
    );
    assert_eq!(
        builtin,
        "ImportError cannot import name 'nope' from 'itertools' (unknown location) itertools \
         None nope\n"
    );
    let with_file = assert_matches_cpython(
        "from_foreign_missing_file",
        "pycc_from_missing_file_mod",
        "from json import nope\n",
    );
    assert!(
        with_file.starts_with("ImportError cannot import name 'nope' from 'json' ("),
        "{with_file}"
    );
    assert!(with_file.trim_end().ends_with(" nope"), "{with_file}");
}

/// A later missing name fails the statement after the earlier names bound,
/// and the statements around it run in source order.
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn a_later_missing_name_fails_in_source_order_in_the_host() {
    let out = assert_matches_cpython(
        "from_foreign_missing_later",
        "pycc_from_missing_later_mod",
        "print('before')\nfrom itertools import product, nope\nprint('after')\n",
    );
    assert!(out.starts_with("before\nImportError "), "{out}");
    assert!(!out.contains("after"), "{out}");
}

#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn a_missing_module_raises_module_not_found_error_in_the_host() {
    let out = assert_matches_cpython(
        "from_foreign_no_module",
        "pycc_from_no_module_mod",
        "from no_such_mod_1278 import x\n",
    );
    assert_eq!(
        out,
        "ModuleNotFoundError No module named 'no_such_mod_1278' no_such_mod_1278 None None\n"
    );
}

/// A plain (embedded) build compiles its module with `ext` set, so the from
/// form runs there too, matching CPython's own output.
#[cfg(not(windows))]
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn an_embedded_build_runs_a_from_import_like_cpython() {
    let dir = ScratchDir::new("from_foreign_embedded").expect("scratch");
    let source = write(
        &dir,
        "m.py",
        "from itertools import product\nprint(str(product))\n",
    );
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
    assert_eq!(stdout_of(&embedded), "<class 'itertools.product'>\n");
}
