//! v0.8 accept 2: the constructs library code falls back for most often
//! compile natively -- the `...` literal, `hash()` of a CPython object,
//! function-local imports, nested functions and lambdas.
//!
//! Each fixture under `tests/fixtures/v08_*.py` prints what it computes and
//! ends in an uncaught exception or not, and the oracle is `python3` (or
//! `PYCC_PYTHON`) running it: a `--non-strict` native build (bundling that
//! interpreter), and the same module built with `--ext --non-strict` and
//! imported, must print the same and fail the same way. `--report-fallbacks`
//! must name no function running in CPython.

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
    String::from_utf8_lossy(bytes).replace("\r\n", "\n")
}

fn last_line(bytes: &[u8]) -> String {
    text(bytes).lines().last().unwrap_or_default().to_string()
}

fn fixture(name: &str, dir: &Path, as_name: &str) -> PathBuf {
    let source = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(format!("{name}.py"));
    let path = dir.join(as_name);
    std::fs::copy(source, &path).expect("copy the fixture");
    path
}

fn assert_matches(ours: &Output, oracle: &Output) {
    assert_eq!(text(&ours.stdout), text(&oracle.stdout), "{}", text(&ours.stderr));
    assert_eq!(ours.status.code(), oracle.status.code(), "{}", text(&ours.stderr));
    assert_eq!(last_line(&ours.stderr), last_line(&oracle.stderr));
}

/// Builds `source` with `flags`, asserting it builds and that no function,
/// method or class runs in CPython (a statement such as an import may);
/// returns the report.
fn build(source: &Path, out: &Path, flags: &[&str]) -> String {
    let build = pycc()
        .args(["build", "--report-fallbacks"])
        .args(flags)
        .arg(source)
        .arg("-o")
        .arg(out)
        .output()
        .expect("pycc should spawn");
    let report = text(&build.stderr);
    assert!(build.status.success(), "{report}");
    for unit in ["fallback: function", "fallback: method", "fallback: class"] {
        assert!(!report.contains(unit), "no function should run in CPython:\n{report}");
    }
    report
}

/// The fixture as a `--non-strict` native program against `python3`.
fn native(name: &str) {
    let dir = ScratchDir::new(&format!("{name}_native")).expect("scratch");
    let source = fixture(name, &dir, "m.py");
    build(&source, &dir.join("app"), &["--non-strict"]);
    let ours = Command::new(dir.join("app")).output().expect("run the build");
    let oracle = python().arg(&source).output().expect("python should spawn");
    assert_matches(&ours, &oracle);
}

/// The fixture as a `--ext --non-strict` module imported by `python3`,
/// against importing its source.
fn ext(name: &str) {
    let compiled = ScratchDir::new(&format!("{name}_ext")).expect("scratch");
    let source = fixture(name, &compiled, "m.py");
    build(&source, &compiled.join("m"), &["--ext", "--non-strict"]);
    std::fs::remove_file(&source).expect("import the artifact, not the source");
    let plain = ScratchDir::new(&format!("{name}_ext_src")).expect("scratch");
    fixture(name, &plain, "m.py");
    let run = |dir: &Path| {
        python()
            .args(["-c", "import m"])
            .current_dir(dir)
            .output()
            .expect("python should spawn")
    };
    assert_matches(&run(&compiled), &run(&plain));
}

#[test]
fn ellipsis_native() {
    native("v08_ellipsis");
}

#[test]
fn ellipsis_ext() {
    ext("v08_ellipsis");
}

/// Strict mode: a bare `...` statement is a no-op anywhere, and the
/// literal as a value is the `Ellipsis` object in an `--ext` module; a
/// native program, which holds no CPython object, refuses the value.
#[test]
fn ellipsis_strict() {
    let dir = ScratchDir::new("v08_ellipsis_strict").expect("scratch");
    let source = dir.join("s.py");
    std::fs::write(
        &source,
        "def stub(x: int) -> None:\n    ...\n\n\
         def marker() -> object:\n    return ...\n\n\
         def is_marker(x: object) -> bool:\n    return x is ...\n",
    )
    .unwrap();
    build(&source, &dir.join("s"), &["--ext"]);
    let run = python()
        .args(["-c", "import s; print(s.stub(1), s.marker(), s.is_marker(...), s.is_marker(1))"])
        .current_dir(&*dir)
        .output()
        .expect("python should spawn");
    assert_eq!(text(&run.stdout), "None Ellipsis True False\n", "{}", text(&run.stderr));
    let native_source = dir.join("n.py");
    std::fs::write(&native_source, "def stub(x: int) -> None:\n    ...\n\n\ndef f() -> None:\n    x = ...\n").unwrap();
    let native = pycc()
        .args(["check"])
        .arg(&native_source)
        .output()
        .expect("pycc should spawn");
    let stderr = text(&native.stderr) + &text(&native.stdout);
    assert!(!native.status.success());
    assert!(stderr.contains("an `...` ellipsis literal used as a value"), "{stderr}");
    assert!(stderr.contains("n.py:6:"), "{stderr}");
    assert!(!stderr.contains("n.py:2:"), "the statement is a no-op: {stderr}");
}

#[test]
fn hash_object_native() {
    native("v08_hash_object");
}

#[test]
fn hash_object_ext() {
    ext("v08_hash_object");
}

#[test]
fn local_import_native() {
    native("v08_local_import");
}

#[test]
fn local_import_ext() {
    ext("v08_local_import");
}

/// Strict mode: a function-local import of a CPython module is typed (its
/// names are CPython objects), in an `--ext` module and in a native program
/// (which embeds CPython for it); repeated imports neither leak the module
/// nor the names they bind.
#[test]
fn local_import_strict_and_leak_free() {
    let dir = ScratchDir::new("v08_local_import_strict").expect("scratch");
    let source = dir.join("sl.py");
    std::fs::write(
        &source,
        "def dump(x: int) -> str:\n    import json\n    from json import dumps as d\n    \
         return str(json.dumps(x)) + str(d(x + 1))\n\n\n\
         def many(n: int) -> int:\n    total = 0\n    for i in range(n):\n        \
         import json\n        from json import dumps\n        total += 1\n    return total\n",
    )
    .unwrap();
    build(&source, &dir.join("sl"), &["--ext"]);
    let run = python()
        .args([
            "-c",
            "import sl, json, sys\nprint(sl.dump(3))\nr0 = sys.getrefcount(json)\n\
             r1 = sys.getrefcount(json.dumps)\nprint(sl.many(100000))\n\
             print(sys.getrefcount(json) - r0, sys.getrefcount(json.dumps) - r1)",
        ])
        .current_dir(&*dir)
        .output()
        .expect("python should spawn");
    assert_eq!(text(&run.stdout), "34\n100000\n0 0\n", "{}", text(&run.stderr));
    let program = dir.join("main.py");
    let mut text_source = std::fs::read_to_string(&source).unwrap();
    text_source.push_str("\nprint(dump(3), many(10))\n");
    std::fs::write(&program, text_source).unwrap();
    build(&program, &dir.join("app"), &[]);
    let ours = Command::new(dir.join("app")).output().expect("run the build");
    assert_eq!(text(&ours.stdout), "34 10\n", "{}", text(&ours.stderr));
}

/// A relative function-local import (`--foreign-relative-imports`) runs
/// after the module body, against the module's own package, in strict and
/// non-strict mode; a missing sibling raises CPython's `ImportError`.
#[test]
fn relative_local_import_resolves_against_the_package() {
    for flags in [&["--ext"][..], &["--ext", "--non-strict"][..]] {
        let dir = ScratchDir::new("v08_relative_local_import").expect("scratch");
        let pkg = dir.join("pkg");
        std::fs::create_dir(&pkg).unwrap();
        std::fs::write(pkg.join("__init__.py"), "").unwrap();
        std::fs::write(pkg.join("a.py"), "X = 5\n").unwrap();
        let source = pkg.join("m.py");
        std::fs::write(
            &source,
            "def f() -> int:\n    from .a import X\n    return int(X) + 1\n\n\n\
             def g() -> str:\n    try:\n        from .nope import Y\n    except ImportError as e:\n        \
             return \"ImportError: \" + str(e)\n    return \"imported\"\n",
        )
        .unwrap();
        let mut all = flags.to_vec();
        all.push("--foreign-relative-imports");
        build(&source, &pkg.join("m"), &all);
        std::fs::remove_file(&source).unwrap();
        let run = python()
            .args(["-c", "from pkg import m\nprint(m.f(), m.g())"])
            .current_dir(&*dir)
            .output()
            .expect("python should spawn");
        assert_eq!(
            text(&run.stdout),
            "6 ImportError: No module named 'pkg.nope'\n",
            "{flags:?}: {}",
            text(&run.stderr)
        );
    }
}
