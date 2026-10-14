//! Multi-module programs (v0.3 accept 1): a program of several `.py` files
//! and packages compiles as one unit, natively (strict and `--non-strict`)
//! and as an `--ext` module, and behaves as CPython 3.13 runs the same
//! files: the same stdout, the same uncaught exception, module bodies run
//! once in import order.
//!
//! The oracle fixtures are directory trees under
//! `tests/fixtures/multi_module/`, each with a `main.py` entry; every
//! expected output is produced by running CPython on the same tree in the
//! same test. The refusals are pinned on scratch trees below.

use pycc_scratch::ScratchDir;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn pycc() -> Command {
    Command::new(env!("CARGO_BIN_EXE_pycc"))
}

fn python() -> Command {
    let mut command =
        Command::new(std::env::var_os("PYCC_PYTHON").unwrap_or_else(|| "python3".into()));
    // Never leave `__pycache__` directories in the fixture tree.
    command.env("PYTHONDONTWRITEBYTECODE", "1");
    command
}

fn fixture(case: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/multi_module")
        .join(case)
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).replace("\r\n", "\n")
}

fn rendered(output: &Output) -> String {
    format!("{}{}", text(&output.stdout), text(&output.stderr))
}

/// The last line of a traceback: the exception CPython reports.
fn exception_line(stderr: &[u8]) -> String {
    text(stderr).lines().last().unwrap_or_default().to_string()
}

/// Builds `case`'s entry natively (with `extra` flags) and checks stdout,
/// the exit status and the reported exception against CPython.
fn native_matches_cpython(case: &str, extra: &[&str]) {
    let dir = ScratchDir::new(&format!("multi_module_{case}")).expect("scratch");
    let entry = fixture(case).join("main.py");
    let binary = dir.join("prog");
    let build = pycc()
        .arg("build")
        .arg(&entry)
        .arg("-o")
        .arg(&binary)
        .args(extra)
        .output()
        .expect("pycc should spawn");
    assert!(
        build.status.success(),
        "{case} {extra:?}: {}",
        rendered(&build)
    );
    let compiled = Command::new(&binary).output().expect("the program runs");
    let oracle = python()
        .arg(&entry)
        .current_dir(fixture(case))
        .output()
        .expect("python3 should spawn");
    assert_eq!(
        text(&compiled.stdout),
        text(&oracle.stdout),
        "{case} {extra:?}"
    );
    assert_eq!(
        compiled.status.code(),
        oracle.status.code(),
        "{case} {extra:?}"
    );
    if !oracle.status.success() {
        assert_eq!(
            exception_line(&compiled.stderr),
            exception_line(&oracle.stderr),
            "{case} {extra:?}"
        );
    }
}

/// Builds `case`'s entry as an `--ext` module, imports it from CPython (in
/// a directory holding only the artifact) and compares with importing the
/// source tree.
fn ext_matches_cpython(case: &str) {
    let dir = ScratchDir::new(&format!("multi_module_ext_{case}")).expect("scratch");
    let build = pycc()
        .arg("build")
        .arg(fixture(case).join("main.py"))
        .arg("-o")
        .arg(dir.join("main"))
        .arg("--ext")
        .output()
        .expect("pycc should spawn");
    assert!(build.status.success(), "{case}: {}", rendered(&build));
    let compiled = python()
        .args(["-c", "import main"])
        .current_dir(&*dir)
        .output()
        .expect("python3 should spawn");
    let oracle = python()
        .args(["-c", "import main"])
        .current_dir(fixture(case))
        .output()
        .expect("python3 should spawn");
    assert_eq!(text(&compiled.stdout), text(&oracle.stdout), "{case}");
    assert_eq!(compiled.status.code(), oracle.status.code(), "{case}");
    if !oracle.status.success() {
        assert_eq!(
            exception_line(&compiled.stderr),
            exception_line(&oracle.stderr),
            "{case}"
        );
    }
}

/// Modules and packages: `import m`, `import pkg.sub.mod as x`, `from pkg
/// import submodule`, relative imports (`.`, `..`), `__init__.py` bodies,
/// `m.f(...)`, `m.CONST`, `m.Class(...)`, a module global another module
/// rebinds through a function (`m.COUNT` reads it live, `from m import
/// COUNT` copied it), two modules defining the same name, a module imported
/// by two others running once, each module's own `__name__`.
#[test]
#[ignore = "requires a CPython 3.13+ oracle (PYCC_PYTHON, default python3)"]
fn a_program_of_modules_and_packages_matches_cpython() {
    native_matches_cpython("program", &[]);
}

#[test]
#[ignore = "requires a CPython 3.13+ oracle (PYCC_PYTHON, default python3)"]
fn a_program_of_modules_and_packages_matches_cpython_in_non_strict_mode() {
    native_matches_cpython("program", &["--non-strict"]);
}

#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn a_program_of_modules_and_packages_matches_cpython_as_an_extension() {
    ext_matches_cpython("program");
}

/// An exception class one module defines, raised there and not caught:
/// CPython reports it by its module-qualified name.
#[test]
#[ignore = "requires a CPython 3.13+ oracle (PYCC_PYTHON, default python3)"]
fn an_uncaught_exception_from_another_module_matches_cpython() {
    native_matches_cpython("raises", &[]);
    native_matches_cpython("raises", &["--non-strict"]);
}

#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn an_uncaught_exception_from_another_module_matches_cpython_as_an_extension() {
    ext_matches_cpython("raises");
}

/// Classes across modules: a subclass of another module's class (reached
/// as `base.Animal` and as a from-imported name), dispatch, `isinstance`
/// against `m.Class`, class attributes read through `m.Class.attr`.
#[test]
#[ignore = "requires a CPython 3.13+ oracle (PYCC_PYTHON, default python3)"]
fn classes_across_modules_match_cpython() {
    native_matches_cpython("classes", &[]);
    native_matches_cpython("classes", &["--non-strict"]);
}

#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn classes_across_modules_match_cpython_as_an_extension() {
    ext_matches_cpython("classes");
}

fn write(root: &Path, relative: &str, contents: &str) -> PathBuf {
    let path = root.join(relative);
    std::fs::create_dir_all(path.parent().expect("a parent")).expect("mkdir");
    std::fs::write(&path, contents).expect("write");
    path
}

/// `pycc check` of `main.py` in a scratch tree of `files`, which must fail:
/// the rendered diagnostics.
fn check_fails(files: &[(&str, &str)]) -> String {
    let dir = ScratchDir::new("multi_module_refusals").expect("scratch");
    for (path, contents) in files {
        write(&dir, path, contents);
    }
    let output = pycc()
        .arg("check")
        .arg(dir.join("main.py"))
        .output()
        .expect("pycc should spawn");
    assert!(!output.status.success(), "{files:?} was accepted");
    rendered(&output)
}

#[test]
fn an_unresolved_relative_import_is_reported_against_the_importer() {
    let text = check_fails(&[
        ("pkg/__init__.py", ""),
        ("pkg/a.py", "from .missing import f\n"),
        ("main.py", "import pkg.a\n"),
    ]);
    assert!(
        text.contains("error[T0021]: no module named `.missing`"),
        "{text}"
    );
    assert!(text.contains("a.py:1:1"), "{text}");
}

#[test]
fn a_name_a_module_does_not_define_is_reported() {
    let text = check_fails(&[
        ("helper.py", "def f() -> int:\n    return 1\n"),
        ("main.py", "from helper import g\n"),
    ]);
    assert!(text.contains("error[T0021]: module `helper` (`"), "{text}");
    assert!(text.contains("has no top-level name `g`"), "{text}");
    let text = check_fails(&[
        ("helper.py", "def f() -> int:\n    return 1\n"),
        ("main.py", "import helper\n\nprint(helper.g())\n"),
    ]);
    assert!(
        text.contains("error[T0021]: module `helper` has no attribute `g`"),
        "{text}"
    );
    assert!(text.contains("main.py:3:7"), "{text}");
}

#[test]
fn a_module_used_as_a_value_is_refused() {
    let text = check_fails(&[
        ("helper.py", "X = 1\n"),
        ("main.py", "import helper\n\nprint(helper)\n"),
    ]);
    assert!(
        text.contains("error[C0001]: the module `helper` used as a value is not supported yet"),
        "{text}"
    );
}

#[test]
fn assigning_another_modules_attribute_is_refused() {
    let text = check_fails(&[
        ("helper.py", "X = 1\n"),
        ("main.py", "import helper\n\nhelper.X = 2\n"),
    ]);
    assert!(
        text.contains("error[C0001]: assigning to `helper.X`, an attribute of the module `helper`"),
        "{text}"
    );
}

#[test]
fn rebinding_an_imported_name_is_refused() {
    let text = check_fails(&[
        ("helper.py", "def f() -> int:\n    return 1\n"),
        ("main.py", "from helper import f\n\nf = 3\n"),
    ]);
    assert!(
        text.contains("error[C0001]: rebinding `f`, which a project import binds"),
        "{text}"
    );
}

#[test]
fn a_circular_import_is_refused_with_the_cycle() {
    let text = check_fails(&[
        ("a.py", "import b\n"),
        ("b.py", "import a\n"),
        ("main.py", "import a\n"),
    ]);
    assert!(text.contains("error[E0108]: import cycle:"), "{text}");
    assert!(text.contains("a.py` -> `"), "{text}");
}

#[test]
fn a_project_import_inside_a_block_is_refused() {
    let text = check_fails(&[
        ("helper.py", "X = 1\n"),
        ("main.py", "if True:\n    import helper\n"),
    ]);
    assert!(
        text.contains("importing the project module `helper` inside an `if`/`try` block"),
        "{text}"
    );
}

/// A diagnostic names another module's class as the program spells it
/// where it is defined, never by the compiler's internal spelling.
#[test]
fn diagnostics_spell_another_modules_names_as_python_does() {
    let text = check_fails(&[
        (
            "geometry.py",
            "class Point:\n    def __init__(self, x: int) -> None:\n        self.x = x\n",
        ),
        (
            "main.py",
            "import geometry\n\n\ndef f(p: geometry.Point) -> int:\n    return p\n",
        ),
    ]);
    assert!(text.contains("expected `int`, found `Point`"), "{text}");
    assert!(!text.contains("0m8geometry"), "{text}");
}
