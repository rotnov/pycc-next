//! A package whose every module -- `__init__.py` included -- is built as
//! its own `--ext --non-strict --foreign-relative-imports` extension module
//! and imported in place of the source by CPython (v0.3 accept 4, the shape
//! `bench/package_check.py` runs on a real PyPI package).
//!
//! The fixture `tests/fixtures/ext_package/textkit` covers what such a
//! package needs from the extension boundary: the package `__init__`
//! artifact named after its directory, re-exports through relative
//! imports, module variables and imports as module attributes, a
//! submodule's dotted `__name__`, the module docstring, class constants on
//! the class and its instances, a call-initialized class attribute, an
//! `int` stored into an inferred `float` attribute from Python, `__str__`
//! and `__repr__`, and the non-strict typing rules untyped methods need
//! (slice assignment on an object, a method with no `return`, a local first
//! bound to `None`, a nested helper called with an object and an `int`, a
//! nested helper returning a tuple or `None`, an exception built from an
//! object message, `chr` of a surrogate code point).
//!
//! The oracle is CPython itself: `driver.py` runs against the compiled tree
//! and against the sources, and the two outputs must be byte-identical.

use pycc_scratch::ScratchDir;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const MODULES: [&str; 3] = ["__init__", "version", "core"];

fn pycc() -> Command {
    Command::new(env!("CARGO_BIN_EXE_pycc"))
}

fn python() -> Command {
    Command::new(std::env::var_os("PYCC_PYTHON").unwrap_or_else(|| "python3".into()))
}

fn fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/ext_package")
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).replace("\r\n", "\n")
}

/// Copies the package into `dir/textkit`.
fn copy_package(dir: &Path) {
    let package = dir.join("textkit");
    std::fs::create_dir_all(&package).expect("create the package directory");
    for module in MODULES {
        let file = format!("{module}.py");
        std::fs::copy(fixture().join("textkit").join(&file), package.join(&file))
            .expect("copy a module");
    }
}

/// Runs the driver against the package under `dir`, from `dir`.
fn run_driver(dir: &Path) -> Output {
    python()
        .arg("-I")
        .arg(fixture().join("driver.py"))
        .arg(dir)
        .current_dir(dir)
        .output()
        .expect("python3 should spawn")
}

/// Builds every module of the package under `dir` in place, returning the
/// builds' `--report-fallbacks` output; the sources are removed only once
/// all are built, because a module's dotted name is read off the
/// `__init__.py` files around it.
fn build_package(dir: &Path) -> String {
    let package = dir.join("textkit");
    let mut report = String::new();
    for module in MODULES {
        let build = pycc()
            .arg("build")
            .arg(package.join(format!("{module}.py")))
            .arg("-o")
            .arg(package.join(module))
            .args([
                "--ext",
                "--non-strict",
                "--foreign-relative-imports",
                "--report-fallbacks",
            ])
            .output()
            .expect("pycc should spawn");
        assert!(
            build.status.success(),
            "{module}: {}{}",
            text(&pycc_scratch::lf(&build.stdout)),
            text(&pycc_scratch::lf(&build.stderr))
        );
        report.push_str(&text(&pycc_scratch::lf(&build.stderr)));
    }
    for module in MODULES {
        std::fs::remove_file(package.join(format!("{module}.py"))).expect("remove a source");
    }
    report
}

#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn a_package_built_module_by_module_matches_cpython() {
    let built = ScratchDir::new("package_ext_built").expect("scratch");
    let plain = ScratchDir::new("package_ext_plain").expect("scratch");
    copy_package(&built);
    copy_package(&plain);
    let report = build_package(&built);
    // Every function and class compiles; only `__init__`'s re-exporting
    // imports run in CPython too, as the CPython imports they are.
    assert!(!report.contains("fallback: function"), "{report}");
    assert!(!report.contains("fallback: class"), "{report}");
    for module in MODULES {
        let artifact = built.join("textkit").join(format!("{module}.abi3.so"));
        assert!(
            artifact.exists() || cfg!(windows),
            "{} is missing",
            artifact.display()
        );
    }
    let ours = run_driver(&built);
    let theirs = run_driver(&plain);
    assert_eq!(theirs.status.code(), Some(0), "{}", text(&pycc_scratch::lf(&theirs.stderr)));
    assert_eq!(text(&pycc_scratch::lf(&ours.stdout)), text(&pycc_scratch::lf(&theirs.stdout)), "{}", text(&pycc_scratch::lf(&ours.stderr)));
    assert_eq!(ours.status.code(), Some(0), "{}", text(&pycc_scratch::lf(&ours.stderr)));
}

/// A package `__init__` artifact needs a directory that names the package:
/// one whose name is not an identifier is refused, as before.
#[test]
fn a_package_init_outside_an_identifier_directory_is_refused() {
    let dir = ScratchDir::new("package_ext_bad_dir").expect("scratch");
    let package = dir.join("not-a-name");
    std::fs::create_dir_all(&package).expect("create");
    std::fs::write(package.join("__init__.py"), "X = 1\n").expect("write");
    let build = pycc()
        .arg("build")
        .arg(package.join("__init__.py"))
        .arg("-o")
        .arg(package.join("__init__"))
        .args(["--ext", "--non-strict"])
        .output()
        .expect("pycc should spawn");
    assert!(!build.status.success());
    let err = text(&pycc_scratch::lf(&build.stderr));
    assert!(err.contains("derives the module name `__init__`"), "{err}");
}
