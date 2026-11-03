//! v0.12: a published class is made once, when its statement runs, instead
//! of as a template made at module init plus the class made from it.
//! `tests/fixtures/v012_class_once/pko` has classes made in one step
//! (`class Plain:`, `class Explicit(object):`, a decorated class, a
//! subclass in its module and subclasses in another module) and classes
//! still made from their template (`Holder(Generic[T])`, whose base is
//! evaluated from its source, and `Kept(Holder)`, whose base is not a class
//! made in one step). `driver.py` reads what `type.__new__` makes of a
//! class statement -- the name, `__qualname__`, `__module__`, the MRO,
//! `__firstlineno__`, `__static_attributes__`, the namespace entries --
//! and exercises `isinstance`/`issubclass`, `repr`, pickling and copying,
//! class attributes, and Python subclasses (`__set_name__`,
//! `__init_subclass__` with keywords, overrides compiled callers run),
//! against the sources and against the package built whole.

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
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/v012_class_once")
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

fn run(driver: &Path, tree: &Path) -> Output {
    python()
        .arg("-I")
        .arg(driver)
        .arg(tree)
        .current_dir(tree)
        .output()
        .expect("python3 should spawn")
}

fn classes_match_cpython(release: bool) {
    let built = ScratchDir::new("v012_class_once_built").expect("scratch");
    let plain = ScratchDir::new("v012_class_once_plain").expect("scratch");
    copy_tree(&fixtures().join("pko"), &built.join("pko"));
    copy_tree(&fixtures().join("pko"), &plain.join("pko"));
    let package = built.join("pko");
    let build = pycc()
        .arg("build")
        .arg(&package)
        .arg("-o")
        .arg(package.join("__init__"))
        .args(["--ext", "--non-strict", "--report-fallbacks"])
        .args(if release { &["--release"][..] } else { &[] })
        .output()
        .expect("pycc should spawn");
    let report = text(&build.stderr);
    assert!(build.status.success(), "{}{report}", text(&build.stdout));
    // The undecorated classes of `pko.base` are compiled and made once. A
    // decorated class and the subclasses `pko.derived` derives across the
    // module boundary run in CPython, from the classes made once.
    for class in ["Plain", "Explicit", "Child", "Holder", "Kept"] {
        assert!(!report.contains(&format!("class `{class}` at")), "{report}");
    }
    for class in ["Counter", "Far", "Farther"] {
        assert!(report.contains(&format!("class `{class}` at")), "{report}");
    }
    remove_sources(&package);
    let driver = fixtures().join("driver.py");
    let theirs = run(&driver, &plain);
    assert_eq!(theirs.status.code(), Some(0), "{}", text(&theirs.stderr));
    let ours = run(&driver, &built);
    assert_eq!(text(&ours.stdout), text(&theirs.stdout), "{}", text(&ours.stderr));
    assert_eq!(ours.status.code(), Some(0), "{}", text(&ours.stderr));
    // The classes made in one step are compiled classes with no template:
    // the root every compiled class derives from comes right after the
    // class's compiled bases.
    let probe = python()
        .args([
            "-I",
            "-c",
            "import sys\nsys.path.insert(0, '.')\nfrom pko import base, derived\n\
             for c in (base.Plain, base.Child, derived.Farther, base.Kept):\n    \
             print([k.__module__ + '.' + k.__qualname__ for k in c.__mro__])\n",
        ])
        .current_dir(&*built)
        .output()
        .expect("python3 should spawn");
    assert_eq!(
        text(&probe.stdout),
        "['pko.base.Plain', 'pycc.compiled', 'builtins.object']\n\
         ['pko.base.Child', 'pko.base.Plain', 'pycc.compiled', 'builtins.object']\n\
         ['pko.derived.Farther', 'pko.base.Child', 'pko.base.Plain', 'pycc.compiled', \
         'builtins.object']\n\
         ['pko.base.Kept', 'pko.base.Kept', 'pko.base.Holder', 'pko.base.Holder', \
         'pycc.compiled', 'typing.Generic', 'builtins.object']\n",
        "{}",
        text(&probe.stderr)
    );
}

#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn classes_made_once_match_cpython() {
    classes_match_cpython(false);
}

#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn classes_made_once_match_cpython_release() {
    classes_match_cpython(true);
}
