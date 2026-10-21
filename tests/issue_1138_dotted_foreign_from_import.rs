//! Part 1 of #1138: an unaliased, top-level `from <dotted module> import a`
//! of a CPython module -- one whose root is neither a project module nor a
//! project package -- binds each name to the CPython object, fetched with
//! CPython's own `IMPORT_NAME`/`IMPORT_FROM` semantics, exactly as #1278's
//! undotted form does (`tests/issue_1278_from_foreign_import.rs`).
//!
//! The plain dotted `import a.b` is admitted by Part 3 (#1381,
//! `tests/issue_1381_dotted_plain_import.rs`), and a name pycc resolves
//! by its spelling keeps the spelling
//! refusal, except the two buffer-carrier pairs #1380 (Part 2) admits
//! (`tests/issue_1380_buffer_carrier_from_import.rs`). The hosted tests are `#[ignore]`d, compare the
//! built extension against the host interpreter's own run of the same
//! source, and contribute no line coverage; the changed driver lines are
//! covered by the unit tests in `src/modules/tests.rs`.

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
    pycc_scratch::lf(&*String::from_utf8_lossy(&output.stdout)).replace("\r\n", "\n")
}

fn stderr_of(output: &Output) -> String {
    pycc_scratch::lf(&*String::from_utf8_lossy(&output.stderr)).replace("\r\n", "\n")
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
        .current_dir(dir)
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
fn check_accepts_a_dotted_foreign_from_import() {
    let dir = ScratchDir::new("dotted_from_check").expect("scratch");
    let output = check_with(
        &dir,
        "from json.decoder import JSONDecoder\nfrom xml.dom import minidom\n\
         print(str(JSONDecoder))\n",
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

/// The shapes outside Part 1 keep a `C0001`: the spelling refusal for a
/// spelling that is not one of #1380's exact carrier pairs (Part 2), and
/// the wildcard from form, whose refusal is now the from form's own message
/// rather than the module's, exactly as for an undotted module.
#[test]
fn the_shapes_outside_part_1_keep_their_c0001() {
    for (tag, body, needle) in [
        (
            // `numpy.typing` exports no `ndarray`; only the exact pairs
            // `numpy.ndarray` and `numpy.typing.NDArray` are carriers.
            "dotted_from_spelling",
            "from numpy.typing import ndarray\n",
            "binding the CPython object `numpy.typing.ndarray` to `ndarray`",
        ),
        (
            "dotted_from_wildcard",
            "from json.decoder import *\n",
            "`from ... import *` (wildcard import) is not supported yet",
        ),
    ] {
        assert_one_error(tag, body, &[], "C0001", needle);
    }
}

/// A dotted name under a project module or package is compiled into the
/// artifact, so it is not foreign: it keeps the module `C0001`.
#[test]
fn a_dotted_from_import_under_a_project_root_keeps_the_c0001() {
    let dir = ScratchDir::new("dotted_from_project_root").expect("scratch");
    write(&dir, "helper.py", "x = 1\n");
    let output = check_with(&dir, "from helper.sub import y\n", &[]);
    assert_eq!(output.status.code(), Some(1), "{}", stderr_of(&output));
    let rendered = stdout_of(&output);
    assert!(
        rendered.contains("import of module `helper.sub` is not supported yet"),
        "{rendered}"
    );
}

/// One statement is one `I0402`, quoting the statement as written.
#[test]
fn a_denied_dotted_from_import_is_one_diagnostic_quoting_the_statement() {
    assert_one_error(
        "dotted_from_deny",
        "from json.decoder import JSONDecoder\n",
        &["--interop-policy", "deny"],
        "I0402",
        "`from json.decoder import JSONDecoder` is a CPython-backed import",
    );
}

/// The allowlist classifies a submodule by its root: `json` admits
/// `json.decoder`, and `xml.dom` is refused naming the root `xml`.
#[test]
fn an_allowlist_classifies_a_dotted_from_import_by_its_root() {
    let dir = ScratchDir::new("dotted_from_allowlist").expect("scratch");
    std::fs::write(
        dir.join("pycc.toml"),
        "[project]\nname = \"p\"\nentry = \"m.py\"\npython = \"3.14\"\n\n\
         [interop]\npolicy = \"allowlist\"\nallow = [\"json\"]\n",
    )
    .expect("write pycc.toml");
    let admitted = check_with(&dir, "from json.decoder import JSONDecoder\n", &[]);
    assert_eq!(
        admitted.status.code(),
        Some(0),
        "{}{}",
        stdout_of(&admitted),
        stderr_of(&admitted)
    );
    let refused = check_with(&dir, "from xml.dom import minidom\n", &[]);
    assert_eq!(refused.status.code(), Some(1), "{}", stderr_of(&refused));
    let rendered = stdout_of(&refused);
    assert_eq!(rendered.matches("error[I0402]").count(), 1, "{rendered}");
    assert!(
        rendered.contains("its root `xml` is not in `[interop] allow`"),
        "{rendered}"
    );
}

/// A native build cannot embed `tkinter`: one `I0403` for the statement,
/// classified by the dotted module's root.
#[test]
fn a_native_build_refuses_a_dotted_from_import_of_an_excluded_root_once() {
    let dir = ScratchDir::new("dotted_from_native").expect("scratch");
    let output = pycc()
        .arg("build")
        .arg(write(&dir, "m.py", "from tkinter.ttk import Button\n"))
        .arg("-o")
        .arg(dir.join("m"))
        .output()
        .expect("pycc should spawn");
    assert_eq!(output.status.code(), Some(1), "{}", stderr_of(&output));
    let rendered = stderr_of(&output);
    assert_eq!(rendered.matches("error[I0403]").count(), 1, "{rendered}");
    assert!(
        rendered.contains("`from tkinter.ttk import Button` imports a standard-library module"),
        "{rendered}"
    );
}

/// A non-standard root still needs a lock in an embedded build; the lock
/// check names the root, not the dotted module.
#[test]
fn an_embedded_build_of_a_non_stdlib_dotted_from_import_needs_the_lock() {
    let dir = ScratchDir::new("dotted_from_lock").expect("scratch");
    let output = pycc()
        .arg("build")
        .arg(write(&dir, "m.py", "from numpy.linalg import norm\n"))
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
fn a_dotted_from_import_binds_the_cpython_object_in_the_host() {
    let out = assert_matches_cpython(
        "dotted_from_hosted",
        "pycc_dotted_from_json_mod",
        "from json.decoder import JSONDecoder, JSONDecodeError\n\
         print(str(JSONDecoder))\nprint(str(JSONDecodeError.__name__))\n",
    );
    assert_eq!(
        out,
        "<class 'json.decoder.JSONDecoder'>\nJSONDecodeError\nno error\n"
    );
}

/// A submodule that is not yet an attribute of its package is bound
/// through the fromlist import and the `sys.modules` fallback, as
/// `IMPORT_FROM` binds it.
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn a_dotted_from_import_binds_a_submodule_in_the_host() {
    let out = assert_matches_cpython(
        "dotted_from_submodule",
        "pycc_dotted_from_minidom_mod",
        "from xml.dom import minidom\nprint(str(minidom.__name__))\n",
    );
    assert_eq!(out, "xml.dom.minidom\nno error\n");
}

/// Each failure raises CPython's own error: a missing name on a module
/// whose `__name__` differs from the spelling (`os.path` is `posixpath` or
/// `ntpath`), a missing leaf, a non-package parent, and a missing root.
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn a_dotted_from_import_raises_cpythons_errors_in_the_host() {
    let missing_name = assert_matches_cpython(
        "dotted_from_missing_name",
        "pycc_dotted_from_missing_name_mod",
        "from os.path import nope\n",
    );
    assert!(
        missing_name.starts_with("ImportError cannot import name 'nope' from '"),
        "{missing_name}"
    );
    assert!(!missing_name.contains("'os.path'"), "{missing_name}");
    let missing_leaf = assert_matches_cpython(
        "dotted_from_missing_leaf",
        "pycc_dotted_from_missing_leaf_mod",
        "from json.nope import x\n",
    );
    assert_eq!(
        missing_leaf,
        "ModuleNotFoundError No module named 'json.nope' json.nope None None\n"
    );
    let not_a_package = assert_matches_cpython(
        "dotted_from_not_a_package",
        "pycc_dotted_from_not_a_package_mod",
        "from os.path.x import y\n",
    );
    assert_eq!(
        not_a_package,
        "ModuleNotFoundError No module named 'os.path.x'; 'os.path' is not a package os.path.x \
         None None\n"
    );
    let missing_root = assert_matches_cpython(
        "dotted_from_missing_root",
        "pycc_dotted_from_missing_root_mod",
        "from no_such_root_1138.sub import y\n",
    );
    assert_eq!(
        missing_root,
        "ModuleNotFoundError No module named 'no_such_root_1138' no_such_root_1138 None None\n"
    );
}

/// A plain (embedded) build runs the dotted from form too, matching
/// CPython's own output.
#[cfg(not(windows))]
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn an_embedded_build_runs_a_dotted_from_import_like_cpython() {
    let dir = ScratchDir::new("dotted_from_embedded").expect("scratch");
    let source = write(
        &dir,
        "m.py",
        "from json.decoder import JSONDecoder\nprint(str(JSONDecoder))\n",
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
    assert_eq!(stdout_of(&embedded), "<class 'json.decoder.JSONDecoder'>\n");
}
