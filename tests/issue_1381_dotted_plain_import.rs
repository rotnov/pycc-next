//! Part 3 of #1138 (#1381): a plain dotted `import a.b` of a CPython
//! module -- one whose root is neither a project module nor a project
//! package -- binds the root `a`, and `import a.b as c` binds the leaf
//! `a.b` to `c`, each fetched with CPython's own `IMPORT_NAME` (and, for
//! the aliased form, `IMPORT_FROM` walk) semantics, at top level and in a
//! module-level `if`/`try` body alike.
//!
//! The non-ignored tests drive `pycc check` and `pycc build`'s gates; the
//! hosted tests are `#[ignore]`d, compare the built extension against the
//! host interpreter's own run of the same source, and contribute no line
//! coverage. The changed Rust lines are covered by the unit tests in
//! `crates/pycc_hir/src/import/tests/dotted.rs`, `src/modules/tests.rs` and
//! `crates/pycc_codegen/src/foreign_import/tests.rs`.

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
fn check_accepts_a_plain_dotted_foreign_import() {
    let dir = ScratchDir::new("dotted_plain_check").expect("scratch");
    let output = check_with(
        &dir,
        "import xml.dom.minidom\nimport email.utils as eu\nimport os\nimport os.path\n\
         if True:\n    import re._parser as sre_parse\n    print(str(sre_parse))\n\
         try:\n    import json.decoder\nexcept ImportError:\n    json = None\n\
         print(str(xml))\nprint(str(eu))\n",
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

/// The shapes Part 3 does not admit keep a `C0001`: a leaf bound under its
/// own root's name, a root pycc resolves by its spelling, and two
/// different modules bound to one name.
#[test]
fn the_shapes_outside_part_3_keep_their_c0001() {
    for (tag, body, needle) in [
        (
            "dotted_plain_as_root",
            "import xml.dom as xml\n",
            "binding the CPython module `xml.dom` to `xml`, the name of its own top-level \
             package, is not supported yet",
        ),
        (
            "dotted_plain_spelling_root",
            "import typing.sub\n",
            "binding the CPython module `typing.sub` to `typing`, a name pycc resolves by its \
             spelling",
        ),
        (
            "dotted_plain_shadow",
            "import xml.dom as x\nimport xml.sax as x\n",
            "shadowing a foreign import",
        ),
    ] {
        assert_one_error(tag, body, &[], "C0001", needle);
    }
}

/// A dotted name under a project module or package is compiled into the
/// artifact, so it is not foreign: it keeps the module `C0001`.
#[test]
fn a_plain_dotted_import_under_a_project_root_keeps_the_c0001() {
    let dir = ScratchDir::new("dotted_plain_project_root").expect("scratch");
    write(&dir, "helper.py", "x = 1\n");
    let output = check_with(&dir, "import helper.sub\n", &[]);
    assert_eq!(output.status.code(), Some(1), "{}", stderr_of(&output));
    let rendered = stdout_of(&output);
    assert!(
        rendered.contains("import of module `helper.sub` is not supported yet"),
        "{rendered}"
    );
}

/// One statement is one `I0402`, quoting the statement as written.
#[test]
fn a_denied_plain_dotted_import_is_one_diagnostic_quoting_the_statement() {
    assert_one_error(
        "dotted_plain_deny",
        "import xml.dom\n",
        &["--interop-policy", "deny"],
        "I0402",
        "`import xml.dom` is a CPython-backed import",
    );
}

/// The allowlist classifies a plain dotted import by its root: `json`
/// admits `json.decoder`, and `xml.dom` is refused naming the root `xml`.
#[test]
fn an_allowlist_classifies_a_plain_dotted_import_by_its_root() {
    let dir = ScratchDir::new("dotted_plain_allowlist").expect("scratch");
    std::fs::write(
        dir.join("pycc.toml"),
        "[project]\nname = \"p\"\nentry = \"m.py\"\npython = \"3.14\"\n\n\
         [interop]\npolicy = \"allowlist\"\nallow = [\"json\"]\n",
    )
    .expect("write pycc.toml");
    let admitted = check_with(&dir, "import json.decoder as d\n", &[]);
    assert_eq!(
        admitted.status.code(),
        Some(0),
        "{}{}",
        stdout_of(&admitted),
        stderr_of(&admitted)
    );
    let refused = check_with(&dir, "import xml.dom\n", &[]);
    assert_eq!(refused.status.code(), Some(1), "{}", stderr_of(&refused));
    let rendered = stdout_of(&refused);
    assert_eq!(rendered.matches("error[I0402]").count(), 1, "{rendered}");
    assert!(
        rendered.contains("its root `xml` is not in `[interop] allow`"),
        "{rendered}"
    );
    assert!(
        rendered.contains("`import xml.dom` is a CPython-backed import"),
        "{rendered}"
    );
    // The aliased form is quoted as written, alias included.
    let aliased = check_with(&dir, "import xml.dom as d\n", &[]);
    assert_eq!(aliased.status.code(), Some(1), "{}", stderr_of(&aliased));
    let rendered = stdout_of(&aliased);
    assert_eq!(rendered.matches("error[I0402]").count(), 1, "{rendered}");
    assert!(
        rendered.contains("`import xml.dom as d` is a CPython-backed import"),
        "{rendered}"
    );
}

/// A native build cannot embed `tkinter`: one `I0403` for the statement,
/// classified by the dotted module's root.
#[test]
fn a_native_build_refuses_a_plain_dotted_import_of_an_excluded_root_once() {
    let dir = ScratchDir::new("dotted_plain_native").expect("scratch");
    let output = pycc()
        .arg("build")
        .arg(write(&dir, "m.py", "import tkinter.ttk\n"))
        .arg("-o")
        .arg(dir.join("m"))
        .output()
        .expect("pycc should spawn");
    assert_eq!(output.status.code(), Some(1), "{}", stderr_of(&output));
    let rendered = stderr_of(&output);
    assert_eq!(rendered.matches("error[I0403]").count(), 1, "{rendered}");
    assert!(
        rendered.contains("`import tkinter.ttk` imports a standard-library module"),
        "{rendered}"
    );
    // The aliased form is quoted as written, alias included.
    let aliased = pycc()
        .arg("build")
        .arg(write(&dir, "a.py", "import tkinter.ttk as t\n"))
        .arg("-o")
        .arg(dir.join("a"))
        .output()
        .expect("pycc should spawn");
    assert_eq!(aliased.status.code(), Some(1), "{}", stderr_of(&aliased));
    let rendered = stderr_of(&aliased);
    assert_eq!(rendered.matches("error[I0403]").count(), 1, "{rendered}");
    assert!(
        rendered.contains("`import tkinter.ttk as t` imports a standard-library module"),
        "{rendered}"
    );
}

/// A non-standard root still needs a lock in an embedded build; the lock
/// check names the root, not the dotted module.
#[test]
fn an_embedded_build_of_a_non_stdlib_plain_dotted_import_needs_the_lock() {
    let dir = ScratchDir::new("dotted_plain_lock").expect("scratch");
    let output = pycc()
        .arg("build")
        .arg(write(&dir, "m.py", "import numpy.linalg\n"))
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

/// `import a.b.c` binds the root package, with every submodule loaded.
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn an_unaliased_dotted_import_binds_the_root_in_the_host() {
    let out = assert_matches_cpython(
        "dotted_plain_root",
        "pycc_dotted_plain_root_mod",
        "import xml.dom.minidom\nprint(str(xml.__name__))\n\
         print(str(xml.dom.minidom.__name__))\n",
    );
    assert_eq!(out, "xml\nxml.dom.minidom\nno error\n");
}

/// `import a.b as c` binds the leaf, at top level and in a module-level
/// `if` body (`import re._parser as sre_parse`, the frontier shape
/// `docs/TESTING.md` records).
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn an_aliased_dotted_import_binds_the_leaf_in_the_host() {
    let out = assert_matches_cpython(
        "dotted_plain_leaf",
        "pycc_dotted_plain_leaf_mod",
        "import email.utils as eu\nprint(str(eu.__name__))\n\
         if True:\n    import re._parser as sre_parse\n    print(str(sre_parse.__name__))\n",
    );
    assert_eq!(out, "email.utils\nre._parser\nno error\n");
}

/// Two imports binding one root each run their own import: after
/// `import xml.dom` then `import xml.sax`, `xml.sax` is loaded.
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn sibling_dotted_imports_each_load_their_submodule_in_the_host() {
    let out = assert_matches_cpython(
        "dotted_plain_siblings",
        "pycc_dotted_plain_siblings_mod",
        "import xml.dom\nimport xml.sax\nprint(str(xml.sax.__name__))\n",
    );
    assert_eq!(out, "xml.sax\nno error\n");
}

/// A missing leaf in a `try`/`except ImportError` body takes the handler,
/// and at top level raises CPython's own error, for the root and the leaf
/// binding alike.
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn a_missing_dotted_leaf_raises_cpythons_error_in_the_host() {
    let bridged = assert_matches_cpython(
        "dotted_plain_bridge",
        "pycc_dotted_plain_bridge_mod",
        "try:\n    import json.nope\nexcept ImportError:\n    print('fallback')\n",
    );
    assert_eq!(bridged, "fallback\nno error\n");
    for (tag, body) in [
        ("dotted_plain_missing_root_form", "import json.nope\n"),
        ("dotted_plain_missing_leaf_form", "import json.nope as n\n"),
    ] {
        let raised = assert_matches_cpython(tag, &format!("pycc_{tag}_mod"), body);
        assert_eq!(
            raised,
            "ModuleNotFoundError No module named 'json.nope' json.nope None None\n"
        );
    }
}

/// A plain (embedded) build runs the dotted plain forms too, matching
/// CPython's own output.
#[cfg(not(windows))]
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn an_embedded_build_runs_a_plain_dotted_import_like_cpython() {
    let dir = ScratchDir::new("dotted_plain_embedded").expect("scratch");
    let source = write(
        &dir,
        "m.py",
        "import xml.dom.minidom\nimport email.utils as eu\n\
         print(str(xml.dom.minidom.__name__))\nprint(str(eu.__name__))\n",
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
    assert_eq!(stdout_of(&embedded), "xml.dom.minidom\nemail.utils\n");
}
