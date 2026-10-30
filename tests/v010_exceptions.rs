//! Exception classes with members of their own (v0.10 accept 5), in a
//! `--non-strict` `--ext` module or a package built whole: CPython makes each
//! from its statement, its methods compiled (`lift`), so the compiled module
//! behaves as its source does. `tests/fixtures/v010_exceptions_host.py` uses
//! them from Python -- construction, `args`, `str`/`repr` with and without
//! `super().__init__`, overrides, a class over another, raising and catching
//! across the boundary, tracebacks, `__dict__`, `del`, pickling and
//! copying, Python subclasses (several bases, a reducing mixin, one freed
//! with its last object), causes and exception groups -- against the module
//! built from `tests/fixtures/v010_exceptions.py` and against its source.
//! `tests/fixtures/v010_exc_pkg/pke`, built whole, derives exception classes
//! from another module's. The remaining fixtures are the shapes that once
//! defeated v0.10's native exception layouts (an alias, nested and
//! conditional class statements, `exec`, a rebound name, an imported name
//! spelled like a builtin, wildcard imports, subclass cycles across modules,
//! a name imported twice, a function-local import): they stay as
//! regressions of the plain path.

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

fn run(dir: &Path, script: &str) -> Output {
    python()
        .args(["-c", script])
        .current_dir(dir)
        .output()
        .expect("python should spawn")
}

/// Builds fixture `name` as `m` in `dir` (`--ext --non-strict`, plus
/// `flags`), leaving the artifact alone: the build's stderr.
fn build(name: &str, dir: &Path, flags: &[&str]) -> String {
    let source = fixture(name, dir, "m.py");
    let build = pycc()
        .args(["build", "--ext", "--non-strict"])
        .args(flags)
        .arg(&source)
        .arg("-o")
        .arg(dir.join("m"))
        .output()
        .expect("pycc should spawn");
    let report = text(&build.stderr);
    assert!(build.status.success(), "{report}");
    std::fs::remove_file(&source).expect("import the artifact, not the source");
    report
}

/// Builds fixture `name` as `m` (`--ext --non-strict`, plus `flags`), runs
/// `script` in CPython against it and against its source, and checks the
/// outputs match. Each of `beside` (a fixture, the file name it takes) lies
/// next to the module in both runs, uncompiled.
fn single_module_matches(name: &str, beside: &[(&str, &str)], flags: &[&str], script: &str) {
    let compiled = ScratchDir::new(&format!("{name}_built")).expect("scratch");
    build(name, &compiled, flags);
    let plain = ScratchDir::new(&format!("{name}_src")).expect("scratch");
    fixture(name, &plain, "m.py");
    for (file, as_name) in beside {
        fixture(file, &compiled, as_name);
        fixture(file, &plain, as_name);
    }
    let theirs = run(&plain, script);
    assert_eq!(theirs.status.code(), Some(0), "{}", text(&theirs.stderr));
    let ours = run(&compiled, script);
    assert_eq!(text(&ours.stdout), text(&theirs.stdout), "{}", text(&ours.stderr));
    assert_eq!(ours.status.code(), Some(0), "{}", text(&ours.stderr));
}

/// The host script of `tests/fixtures/v010_exceptions_host.py`.
const HOST: &str = "exec(open('host.py').read())";

fn exceptions_match_cpython(flags: &[&str]) {
    single_module_matches(
        "v010_exceptions",
        &[("v010_exceptions_host", "host.py")],
        flags,
        HOST,
    );
}

#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn exception_classes_match_cpython() {
    exceptions_match_cpython(&[]);
}

#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn exception_classes_match_cpython_release() {
    exceptions_match_cpython(&["--release"]);
}

/// The functions raising and catching the classes stay compiled.
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn the_functions_raising_them_are_compiled() {
    let dir = ScratchDir::new("v010_exceptions_report").expect("scratch");
    let report = build("v010_exceptions", &dir, &["--report-fallbacks"]);
    for function in ["fail", "run", "churn", "probe"] {
        assert!(!report.contains(&format!("function `{function}`")), "{report}");
    }
}

/// Raising, catching, pickling and dropping the exceptions frees what they
/// allocate.
#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn the_exceptions_do_not_leak() {
    let dir = ScratchDir::new("v010_exceptions_leaks").expect("scratch");
    build("v010_exceptions", &dir, &[]);
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

#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn an_aliased_class_matches_cpython() {
    single_module_matches(
        "v010_exc_alias",
        &[],
        &[],
        "import m\nprint(m.run(3), m.plain(4), [c.__name__ for c in m.Through.__mro__])\n\
         class Again(m.Alias, KeyError):\n    pass\nprint(Again(1).code if hasattr(Again(1), 'code') \
         else 'none', [c.__name__ for c in Again.__mro__][:3])\n",
    );
}

#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn unexamined_class_statements_match_cpython() {
    single_module_matches(
        "v010_exc_nested",
        &[],
        &[],
        "import m\nC = m.factory()\nprint(C(3).code, m.Cond(4).code, m.Guard(5).code, \
         m.Dec(6).code, m.REGISTRY[0].__name__, m.kept(7))\n",
    );
}

#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn a_rebound_class_name_matches_cpython() {
    single_module_matches(
        "v010_exc_rebound",
        &[],
        &[],
        "import m\nprint(m.make(3), [c.__name__ for c in m.B.__mro__])\n",
    );
}

#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn a_class_made_by_exec_matches_cpython() {
    single_module_matches(
        "v010_exc_exec",
        &[],
        &[],
        "import m\nM = m.made()\nprint(M(8).code, [c.__name__ for c in M.__mro__])\n",
    );
}

#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn an_imported_builtin_spelling_matches_cpython() {
    single_module_matches(
        "v010_exc_shadow_builtin",
        &[("v010_exc_helper", "helper.py")],
        &[],
        "import m\nprint(m.make(3), m.Err(4).code, m.Err(5).stored)\n",
    );
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

/// Builds package `package` of fixture directory `dir` whole (`--ext
/// --non-strict`, plus `flags`), its sources then removed: the build's
/// stderr and the built tree.
fn build_package(dir: &str, package: &str, flags: &[&str]) -> (String, ScratchDir) {
    let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures").join(dir);
    let built = ScratchDir::new(&format!("{dir}_built")).expect("scratch");
    copy_tree(&fixtures.join(package), &built.join(package));
    let tree = built.join(package);
    let build = pycc()
        .arg("build")
        .arg(&tree)
        .arg("-o")
        .arg(tree.join("__init__"))
        .args(["--ext", "--non-strict"])
        .args(flags)
        .output()
        .expect("pycc should spawn");
    let report = text(&build.stderr);
    assert!(build.status.success(), "{report}");
    for entry in std::fs::read_dir(&tree).expect("read the tree") {
        let path = entry.expect("an entry").path();
        if path.extension().is_some_and(|ext| ext == "py") {
            std::fs::remove_file(&path).expect("remove a source");
        }
    }
    (report, built)
}

/// Builds package `package` of fixture directory `dir` whole, runs `script`
/// in CPython against it and against its sources, and checks the outputs
/// match.
fn package_matches(dir: &str, package: &str, script: &str) {
    let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures").join(dir);
    let (_, built) = build_package(dir, package, &[]);
    let plain = ScratchDir::new(&format!("{dir}_plain")).expect("scratch");
    copy_tree(&fixtures.join(package), &plain.join(package));
    let run_in = |at: &Path| {
        python()
            .args(["-I", "-c", &format!("import sys; sys.path.insert(0, '.')\n{script}")])
            .current_dir(at)
            .output()
            .expect("python3 should spawn")
    };
    let theirs = run_in(&plain);
    assert_eq!(theirs.status.code(), Some(0), "{}", text(&theirs.stderr));
    let ours = run_in(&built);
    assert_eq!(text(&ours.stdout), text(&theirs.stdout), "{}", text(&ours.stderr));
    assert_eq!(ours.status.code(), Some(0), "{}", text(&ours.stderr));
}

/// `tests/fixtures/v010_exc_pkg`'s driver against the package built whole
/// and against its sources, with its leak probe; the functions raising and
/// catching the classes stay compiled.
fn exception_package_matches_cpython(flags: &[&str]) {
    let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/v010_exc_pkg");
    let mut all = vec!["--report-fallbacks"];
    all.extend_from_slice(flags);
    let (report, built) = build_package("v010_exc_pkg", "pke", &all);
    for function in ["raise_sub", "catch"] {
        assert!(!report.contains(&format!("function `{function}` at")), "{report}");
    }
    let plain = ScratchDir::new("v010_exc_pkg_plain").expect("scratch");
    copy_tree(&fixtures.join("pke"), &plain.join("pke"));
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
}

#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn exception_classes_over_another_module_s_match_cpython() {
    exception_package_matches_cpython(&[]);
}

#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn exception_classes_over_another_module_s_match_cpython_release() {
    exception_package_matches_cpython(&["--release"]);
}

#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn a_wildcard_imported_class_matches_cpython() {
    package_matches(
        "v010_exc_star",
        "pkw",
        "import pkw\nprint(pkw.coded(2), pkw.combine(3), pkw.Combined(4).show(), \
         [c.__name__ for c in pkw.Combined.__mro__])\n",
    );
}

#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn a_subclass_cycle_across_modules_matches_cpython() {
    package_matches(
        "v010_exc_cycle",
        "pkc",
        "import pkc\nprint(pkc.make(3), [c.__name__ for c in pkc.C.__mro__])\n",
    );
}

#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn a_name_imported_twice_matches_cpython() {
    package_matches(
        "v010_exc_rebind",
        "pkd",
        "import pkd\nprint(pkd.make(3), [c.__name__ for c in pkd.B.__mro__], pkd.b.A.__name__)\n",
    );
}

#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn a_function_local_import_matches_cpython() {
    package_matches(
        "v010_exc_local",
        "pkl",
        "import pkl\nprint(pkl.make(3), [c.__name__ for c in pkl.B.__mro__], \
         pkl.other().__name__)\n",
    );
}
