//! Non-strict `--ext`: `super(C, self)` in a method of a class CPython
//! creates is CPython's `super`, called from the compiled body, instead of
//! the method falling back. The oracle is CPython running the fixture.

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
fn an_explicit_super_in_a_lifted_method_compiles() {
    let dir = ScratchDir::new("explicit_super").expect("scratch");
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/explicit_super.py");
    std::fs::copy(&source, dir.join("supers.py")).expect("copy the fixture");
    let build = Command::new(env!("CARGO_BIN_EXE_pycc"))
        .args(["build", "--ext", "--non-strict", "--report-fallbacks"])
        .arg(dir.join("supers.py"))
        .arg("-o")
        .arg(dir.join("supers"))
        .output()
        .expect("pycc should spawn");
    let report = text(&pycc_scratch::lf(&build.stderr));
    assert!(build.status.success(), "{report}");
    assert!(!report.contains("method `Child."), "{report}");
    let oracle = python()
        .arg(&source)
        .output()
        .expect("python3 should spawn");
    std::fs::remove_file(dir.join("supers.py")).expect("remove the source");
    let ours = python()
        .args(["-c", "import supers\nsupers.main()\n"])
        .current_dir(&*dir)
        .output()
        .expect("python3 should spawn");
    assert_eq!(
        text(&pycc_scratch::lf(&ours.stdout)),
        text(&pycc_scratch::lf(&oracle.stdout)),
        "{}",
        text(&pycc_scratch::lf(&ours.stderr))
    );
}
