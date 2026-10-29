//! v0.10 accept 5: compiled subclasses of compiled classes of another
//! module of a package built whole are native. `tests/fixtures/
//! v010_subclasses/pks` derives classes from another module's (`Square`
//! over `base.Shape`, `Rect` over `Square`, `Doubled` over `base.Plain`):
//! each is compiled over the other module's layout, its class record
//! (`pycc_codegen`'s `class_record`) carrying a vtable the other module's
//! dispatchers read (`pycc_mir::dispatch`), so a method compiled in `base`
//! calling `self.kind()` on a `Square` runs `Square.kind`. `driver.py`
//! exercises them from Python -- overrides called from base methods across
//! modules, `super()` chains through the other module's bodies,
//! `isinstance`, the MRO, attributes, `copy.copy`, a Python subclass of a
//! compiled subclass, a subclass in a third module (run by CPython, the
//! documented gap) -- against the sources and against the compiled tree,
//! and probes for leaks.

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
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/v010_subclasses")
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

/// Builds `dir/pks` whole with `args` besides; the report.
fn build_whole(dir: &Path, args: &[&str]) -> String {
    let package = dir.join("pks");
    let build = pycc()
        .arg("build")
        .arg(&package)
        .arg("-o")
        .arg(package.join("__init__"))
        .args(["--ext", "--non-strict", "--report-fallbacks"])
        .args(args)
        .output()
        .expect("pycc should spawn");
    let report = text(&build.stderr);
    assert!(build.status.success(), "{}{report}", text(&build.stdout));
    report
}

fn run(driver: &Path, tree: &Path) -> Output {
    python()
        .arg("-I")
        .arg(driver)
        .arg(tree)
        .current_dir(tree)
        .output()
        .expect("python3 should spawn")
}

fn subclasses_match_cpython(release: bool) {
    let built = ScratchDir::new("v010_subclasses_built").expect("scratch");
    let plain = ScratchDir::new("v010_subclasses_plain").expect("scratch");
    copy_tree(&fixtures().join("pks"), &built.join("pks"));
    copy_tree(&fixtures().join("pks"), &plain.join("pks"));
    let report = build_whole(&built, if release { &["--release"] } else { &[] });
    // The subclasses of another module's classes are compiled: no class
    // statement of `shapes` runs in CPython, and both of `base`'s classes
    // are read natively there.
    for class in ["Square", "Rect", "Doubled"] {
        assert!(!report.contains(&format!("class `{class}` at")), "{report}");
    }
    for class in ["pks.base.Shape", "pks.base.Plain"] {
        assert!(
            report.contains(&format!("native class: instances of `{class}` are read natively")),
            "{report}"
        );
    }
    // A subclass of a subclass another module compiled runs in CPython
    // over its layout (`docs/DEVIATIONS.md`).
    assert!(report.contains("class `Cube` at"), "{report}");
    remove_sources(&built.join("pks"));
    let driver = fixtures().join("driver.py");
    let theirs = run(&driver, &plain);
    assert_eq!(theirs.status.code(), Some(0), "{}", text(&theirs.stderr));
    let ours = run(&driver, &built);
    assert_eq!(text(&ours.stdout), text(&theirs.stdout), "{}", text(&ours.stderr));
    assert_eq!(ours.status.code(), Some(0), "{}", text(&ours.stderr));
    assert!(text(&ours.stdout).contains("leaks none"));
}

#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn native_subclasses_across_modules_match_cpython() {
    subclasses_match_cpython(false);
}

#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn native_subclasses_across_modules_match_cpython_release() {
    subclasses_match_cpython(true);
}

/// An override whose signature is not the base's (`area` returning a
/// `float` over an `int` one) gets no vtable entry: the base module's
/// compiled callers reach it through CPython, and its result is converted
/// to the declared type, raising `TypeError` for another -- as for a
/// Python subclass's override (`docs/DEVIATIONS.md`). Its own callers run
/// it natively.
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn an_override_of_another_signature_is_reached_through_cpython() {
    let dir = ScratchDir::new("v010_subclasses_signature").expect("scratch");
    let package = dir.join("pks");
    std::fs::create_dir_all(&package).expect("create the package");
    std::fs::write(package.join("__init__.py"), "").expect("write");
    std::fs::write(
        package.join("base.py"),
        "class Shape:\n    def __init__(self, size: int) -> None:\n        self.size = size\n\n    \
         def area(self) -> int:\n        return self.size\n\n    \
         def show(self) -> str:\n        return str(self.area())\n",
    )
    .expect("write");
    std::fs::write(
        package.join("odd.py"),
        "from .base import Shape\n\n\nclass Odd(Shape):\n    def area(self) -> float:\n        \
         return self.size / 2\n\n\ndef half(o: Odd) -> float:\n    return o.area()\n",
    )
    .expect("write");
    let report = build_whole(&dir, &[]);
    assert!(!report.contains("class `Odd` at"), "{report}");
    remove_sources(&package);
    let out = python()
        .args([
            "-c",
            "from pks import odd\no = odd.Odd(3)\nprint(odd.half(o), o.area())\n\
             try:\n    o.show()\nexcept TypeError as e:\n    print('TypeError', e)\n",
        ])
        .current_dir(&*dir)
        .output()
        .expect("python3 should spawn");
    assert_eq!(
        text(&out.stdout),
        "1.5 1.5\nTypeError return value must be int, not float\n",
        "{}",
        text(&out.stderr)
    );
}
