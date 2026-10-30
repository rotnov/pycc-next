//! Exception classes with members of their own (v0.10 accept 5). In a
//! `--non-strict` `--ext` module CPython makes each from its statement over
//! a PEP 697 template (`src/ext/module/exc_templates.c`), its methods
//! compiled, and the attributes they store from typed parameters live in a
//! native layout the compiled methods read and store natively
//! (`src/fallback/exc_layout.rs`). `tests/fixtures/v010_exceptions_host.py`
//! uses them from Python -- construction, `args`, `str`/`repr` with and
//! without `super().__init__`, overrides, a layout over another, raising
//! and catching across the boundary, tracebacks, `__dict__`, `del`,
//! pickling and copying, Python subclasses, causes and exception groups --
//! against the module built from `tests/fixtures/v010_exceptions.py` and
//! against its source. `tests/fixtures/v010_exc_pkg/pke`, built whole,
//! derives exception classes from another module's: over one without a
//! layout (a layout of its own over it) and over a layout class (that
//! class's layout).

use pycc_scratch::ScratchDir;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn pycc() -> Command {
    Command::new(env!("CARGO_BIN_EXE_pycc"))
}

fn python() -> Command {
    Command::new(std::env::var_os("PYCC_PYTHON").unwrap_or_else(|| "python3".into()))
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(&pycc_scratch::lf(bytes)).replace("\r\n", "\n")
}

fn fixture(name: &str, dir: &Path, as_name: &str) -> PathBuf {
    let source = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(format!("{name}.py"));
    let path = dir.join(as_name);
    std::fs::copy(source, &path).expect("copy the fixture");
    path
}

fn build(dir: &Path, flags: &[&str]) {
    let source = fixture("v010_exceptions", dir, "m.py");
    let build = pycc()
        .args(["build", "--ext", "--non-strict"])
        .args(flags)
        .arg(&source)
        .arg("-o")
        .arg(dir.join("m"))
        .output()
        .expect("pycc should spawn");
    assert!(build.status.success(), "{}", text(&build.stderr));
    std::fs::remove_file(&source).expect("import the artifact, not the source");
}

fn run(dir: &Path, script: &str) -> Output {
    python()
        .args(["-c", script])
        .current_dir(dir)
        .output()
        .expect("python should spawn")
}

fn matches_cpython(flags: &[&str]) {
    let compiled = ScratchDir::new("v010_exceptions").expect("scratch");
    build(&compiled, flags);
    fixture("v010_exceptions_host", &compiled, "host.py");
    let plain = ScratchDir::new("v010_exceptions_src").expect("scratch");
    fixture("v010_exceptions", &plain, "m.py");
    fixture("v010_exceptions_host", &plain, "host.py");
    let script = "exec(open('host.py').read())";
    let ours = run(&compiled, script);
    let theirs = run(&plain, script);
    assert_eq!(theirs.status.code(), Some(0), "{}", text(&theirs.stderr));
    assert_eq!(text(&ours.stdout), text(&theirs.stdout), "{}", text(&ours.stderr));
    assert_eq!(ours.status.code(), Some(0), "{}", text(&ours.stderr));
}

#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn exception_classes_with_a_layout_match_cpython() {
    matches_cpython(&[]);
}

#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn exception_classes_with_a_layout_match_cpython_release() {
    matches_cpython(&["--release"]);
}

/// The attributes are native: not in `__dict__`, the template in the MRO,
/// a store of another type kept there as CPython keeps it in `__dict__`,
/// and the compiled functions raising and catching the classes compiled.
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn the_layout_is_native() {
    let dir = ScratchDir::new("v010_exceptions_native").expect("scratch");
    let source = fixture("v010_exceptions", &dir, "m.py");
    let build = pycc()
        .args(["build", "--ext", "--non-strict", "--report-fallbacks"])
        .arg(&source)
        .arg("-o")
        .arg(dir.join("m"))
        .output()
        .expect("pycc should spawn");
    let report = text(&build.stderr);
    assert!(build.status.success(), "{report}");
    for function in ["fail", "run", "churn", "probe"] {
        assert!(!report.contains(&format!("function `{function}`")), "{report}");
    }
    std::fs::remove_file(&source).expect("import the artifact");
    let out = run(
        &dir,
        "import m\ne = m.Deeper(1, 'w', 2)\n\
         print('code' in e.__dict__, 'depth' in e.__dict__, \
         [c.__name__ for c in type(e).__mro__ if c.__module__ == 'pycc'])\n\
         e.code = 'x'\nprint(repr(e.code), e.__dict__)\n\
         f = m.NoFile(2, 'p')\nprint('path' in f.__dict__, str(f), f.errno)\n\
         print([[c for c in k.__mro__ if c.__module__ == 'pycc'] \
         for k in (m.Metered, m.Plated, m.Both, m.KeyOS, m.Lazy, m.Reduced)])\n",
    );
    assert_eq!(
        text(&out.stdout),
        "False False ['exception_layout', 'exception_layout']\n\
         'x' {}\n\
         True [Errno 2] no such thing 2\n\
         [[], [], [], [], [], []]\n",
        "{}",
        text(&out.stderr)
    );
}

/// A layout class reached through an alias, and a class statement over the
/// alias (`tests/fixtures/v010_exc_alias.py`): no class of the module gets a
/// layout, and the module behaves as its source does.
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn an_aliased_class_gets_no_layout() {
    let script = "import m\nprint(m.run(3), m.plain(4), [c.__name__ for c in m.Through.__mro__], \
                  [[c for c in k.__mro__ if c.__module__ == 'pycc'] for k in (m.Aliased, m.Plain)])\n\
                  class Again(m.Alias, KeyError):\n    pass\nprint(Again(1).code if hasattr(Again(1), 'code') \
                  else 'none', [c.__name__ for c in Again.__mro__][:3])\n";
    let compiled = ScratchDir::new("v010_exc_alias").expect("scratch");
    let source = fixture("v010_exc_alias", &compiled, "m.py");
    let build = pycc()
        .args(["build", "--ext", "--non-strict"])
        .arg(&source)
        .arg("-o")
        .arg(compiled.join("m"))
        .output()
        .expect("pycc should spawn");
    assert!(build.status.success(), "{}", text(&build.stderr));
    std::fs::remove_file(&source).expect("import the artifact");
    let plain = ScratchDir::new("v010_exc_alias_src").expect("scratch");
    fixture("v010_exc_alias", &plain, "m.py");
    let ours = run(&compiled, script);
    let theirs = run(&plain, script);
    assert_eq!(theirs.status.code(), Some(0), "{}", text(&theirs.stderr));
    assert_eq!(text(&ours.stdout), text(&theirs.stdout), "{}", text(&ours.stderr));
    assert!(text(&ours.stdout).contains("[[], []]"), "{}", text(&ours.stdout));
}

/// Raising, catching, pickling and dropping the exceptions frees what they
/// allocate: an object and its native instance hold each other, a cycle the
/// collector frees.
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn exceptions_with_a_layout_do_not_leak() {
    let dir = ScratchDir::new("v010_exceptions_leaks").expect("scratch");
    build(&dir, &[]);
    let out = run(
        &dir,
        "import ctypes, gc, pickle, m\nlib = ctypes.CDLL(m.__file__)\n\
         live = lambda: (gc.collect(), lib.pycc_rt_str_live_objects(), \
         lib.pycc_rt_instance_live_objects())[1:]\n\
         def work():\n    m.churn(40)\n    e = m.Owned(3, 'w' * 3)\n    e.describe()\n    \
         e.bump(1)\n    m.Deeper(1, 'd', 2).describe()\n    try:\n        m.fail(15)\n    \
         except m.Owned as caught:\n        caught.describe()\n    \
         pickle.loads(pickle.dumps(m.Coded(4)))\n    del e.code\n\
         for _ in range(50):\n    work()\nbefore = live()\nfor _ in range(500):\n    work()\n\
         print('leaks', 'none' if live() == before else (before, live()))\n",
    );
    assert_eq!(text(&out.stdout), "leaks none\n", "{}", text(&out.stderr));
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

fn package_matches_cpython(release: bool) {
    let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/v010_exc_pkg");
    let built = ScratchDir::new("v010_exc_pkg_built").expect("scratch");
    let plain = ScratchDir::new("v010_exc_pkg_plain").expect("scratch");
    copy_tree(&fixtures.join("pke"), &built.join("pke"));
    copy_tree(&fixtures.join("pke"), &plain.join("pke"));
    let package = built.join("pke");
    let build = pycc()
        .arg("build")
        .arg(&package)
        .arg("-o")
        .arg(package.join("__init__"))
        .args(["--ext", "--non-strict", "--report-fallbacks"])
        .args(if release { &["--release"][..] } else { &[][..] })
        .output()
        .expect("pycc should spawn");
    let report = text(&build.stderr);
    assert!(build.status.success(), "{report}");
    for function in ["raise_sub", "catch"] {
        assert!(!report.contains(&format!("function `{function}` at")), "{report}");
    }
    for entry in std::fs::read_dir(&package).expect("read the tree") {
        let path = entry.expect("an entry").path();
        if path.extension().is_some_and(|ext| ext == "py") {
            std::fs::remove_file(&path).expect("remove a source");
        }
    }
    let driver = fixtures.join("driver.py");
    let run_driver = |tree: &Path| {
        python()
            .arg("-I")
            .arg(&driver)
            .arg(tree)
            .current_dir(tree)
            .output()
            .expect("python3 should spawn")
    };
    let theirs = run_driver(&plain);
    assert_eq!(theirs.status.code(), Some(0), "{}", text(&theirs.stderr));
    let ours = run_driver(&built);
    assert_eq!(text(&ours.stdout), text(&theirs.stdout), "{}", text(&ours.stderr));
    assert_eq!(ours.status.code(), Some(0), "{}", text(&ours.stderr));
    assert!(text(&ours.stdout).contains("leaks none"));
    // `KeyMissing`, over another module's exception class without a layout,
    // has one of its own; `Sub`, over another module's layout class, keeps
    // that class's (its own attribute in `__dict__`, `docs/DEVIATIONS.md`).
    let out = python()
        .arg("-I")
        .arg("-c")
        .arg(
            "import sys; sys.path.insert(0, '.')\nimport pke\n\
             k = pke.KeyMissing('a', 'b')\ns = pke.Sub(1, 'w', 'e')\n\
             print('key' in k.__dict__, 'code' in s.__dict__, 'extra' in s.__dict__)\n\
             print('rate' in pke.common.Rated(1).__dict__, 'left' in pke.common.Lefty(1).__dict__, \
             'pin' in pke.common.Pinned(1).__dict__)\n",
        )
        .current_dir(&*built)
        .output()
        .expect("python3 should spawn");
    assert_eq!(text(&out.stdout), "False False True\nTrue True True\n", "{}", text(&out.stderr));
}

#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn exception_classes_over_another_module_s_match_cpython() {
    package_matches_cpython(false);
}

#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn exception_classes_over_another_module_s_match_cpython_release() {
    package_matches_cpython(true);
}
