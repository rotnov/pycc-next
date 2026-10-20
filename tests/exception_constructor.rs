//! An exception class with a constructor of its own (`class MissingRef(
//! Exception)` with an `__init__` keeping a field) in a non-strict `--ext`
//! module: pycc raises a compiled exception by its type and message only,
//! so the class runs in CPython, and the functions raising and catching it
//! stay compiled (`src/fallback.rs`'s `exception_with_constructor`), as
//! does `isinstance` of a caught exception against a builtin exception
//! class. The oracle is CPython running the fixture.

use pycc_scratch::ScratchDir;
use std::path::Path;
use std::process::Command;

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).replace("\r\n", "\n")
}

fn python() -> Command {
    Command::new(std::env::var_os("PYCC_PYTHON").unwrap_or_else(|| "python3".into()))
}

#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn functions_raising_an_exception_class_with_a_constructor_stay_compiled() {
    let dir = ScratchDir::new("exception_constructor").expect("scratch");
    let source =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/exception_constructor.py");
    std::fs::copy(&source, dir.join("excmod.py")).expect("copy the fixture");
    let build = Command::new(env!("CARGO_BIN_EXE_pycc"))
        .args(["build", "--ext", "--non-strict", "--report-fallbacks"])
        .arg(dir.join("excmod.py"))
        .arg("-o")
        .arg(dir.join("excmod"))
        .output()
        .expect("pycc should spawn");
    let report = text(&build.stderr);
    assert!(build.status.success(), "{report}");
    assert!(
        report.contains("class `MissingRef` at ")
            && report.contains("an exception class with a constructor of its own"),
        "{report}"
    );
    for function in ["find", "lookup_len", "kinds", "main"] {
        assert!(!report.contains(&format!("function `{function}`")), "{report}");
    }
    let oracle = python()
        .arg(&source)
        .output()
        .expect("python3 should spawn");
    std::fs::remove_file(dir.join("excmod.py")).expect("remove the source");
    let ours = python()
        .args(["-c", "import excmod; excmod.main()"])
        .current_dir(&*dir)
        .output()
        .expect("python3 should spawn");
    assert_eq!(text(&ours.stdout), text(&oracle.stdout), "{}", text(&ours.stderr));
    assert!(ours.status.success());
}
