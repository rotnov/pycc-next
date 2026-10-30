//! v0.11 accept 3: importing a compiled package does less work than it did
//! without changing what Python code observes. `tests/fixtures/v011_import/
//! pkimp` is built whole; `driver.py` imports it and prints what Python
//! code can see of it -- the attributes the import adds to `sys`, its
//! published classes (made from their templates when their statements run:
//! no arguments, `object`, a base of the module, `Generic[T]`, class-body
//! `None` defaults on slots), a class CPython runs, a Python subclass, and
//! compiled frames in a traceback and `sys._getframe` -- against the
//! sources and against the compiled tree, which must print the same.

use pycc_scratch::ScratchDir;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn pycc() -> Command {
    Command::new(env!("CARGO_BIN_EXE_pycc"))
}

fn python() -> Command {
    Command::new(std::env::var_os("PYCC_PYTHON").unwrap_or_else(|| "python3".into()))
}

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/v011_import")
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(&pycc_scratch::lf(bytes)).replace("\r\n", "\n")
}

fn copy_tree(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).expect("create a directory");
    for entry in std::fs::read_dir(from).expect("read the fixture") {
        let entry = entry.expect("an entry");
        let target = to.join(entry.file_name());
        if entry.file_type().expect("its type").is_dir() {
            copy_tree(&entry.path(), &target);
        } else {
            std::fs::copy(entry.path(), &target).expect("copy a file");
        }
    }
}

fn remove_sources(dir: &Path) {
    for entry in std::fs::read_dir(dir).expect("read the tree") {
        let path = entry.expect("an entry").path();
        if path.is_dir() {
            remove_sources(&path);
        } else if path.extension().is_some_and(|ext| ext == "py") {
            std::fs::remove_file(&path).expect("remove a source");
        }
    }
}

/// Builds `dir/pkimp` whole; the report.
fn build_whole(dir: &Path) -> String {
    let package = dir.join("pkimp");
    let build = pycc()
        .arg("build")
        .arg(&package)
        .arg("-o")
        .arg(package.join("__init__"))
        .args(["--ext", "--non-strict", "--report-fallbacks", "--release"])
        .output()
        .expect("pycc should spawn");
    let report = text(&build.stderr);
    assert!(build.status.success(), "{}{report}", text(&build.stdout));
    report
}

fn run(driver: &Path, tree: &Path) -> Output {
    python()
        .arg("-I")
        // Every warning shown, so one the import raises (a C type made
        // with no module) shows in the stderr compared below.
        .args(["-W", "always"])
        .arg(driver)
        .arg(tree)
        .current_dir(tree)
        .output()
        .expect("python3 should spawn")
}

#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn importing_the_compiled_package_is_observed_as_importing_its_sources() {
    let built = ScratchDir::new("v011_import_built").expect("scratch");
    let plain = ScratchDir::new("v011_import_plain").expect("scratch");
    copy_tree(&fixtures().join("pkimp"), &built.join("pkimp"));
    copy_tree(&fixtures().join("pkimp"), &plain.join("pkimp"));
    let report = build_whole(&built);
    // The classes the driver reads as published are published; the one
    // with a keyword argument runs in CPython.
    for class in ["Plain", "Boxed", "Loader"] {
        assert!(
            report.contains(&format!("native class: instances of `pkimp.classes.{class}`")),
            "{report}"
        );
    }
    assert!(report.contains("class `Abstract` at"), "{report}");
    remove_sources(&built.join("pkimp"));
    let driver = fixtures().join("driver.py");
    let theirs = run(&driver, &plain);
    assert_eq!(theirs.status.code(), Some(0), "{}", text(&theirs.stderr));
    let ours = run(&driver, &built);
    assert_eq!(text(&ours.stdout), text(&theirs.stdout), "{}", text(&ours.stderr));
    assert_eq!(ours.status.code(), Some(0), "{}", text(&ours.stderr));
    assert_eq!(text(&ours.stderr), text(&theirs.stderr));
}
