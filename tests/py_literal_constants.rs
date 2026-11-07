//! v0.12: a `str` literal a module only hands to CPython is made as a
//! CPython `str` directly (`crates/pycc_codegen/src/py_literal.rs`): the
//! same text, interned where CPython interns a code constant, and one
//! object per text of the module, as CPython's merged constants. The
//! oracle is importing the module's source.

use pycc_scratch::ScratchDir;
use std::path::Path;
use std::process::Command;

fn python() -> Command {
    Command::new(std::env::var_os("PYCC_PYTHON").unwrap_or_else(|| "python3".into()))
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(&pycc_scratch::lf(bytes)).replace("\r\n", "\n")
}

#[test]
fn literals_handed_to_cpython_match_its_constants() {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/py_literal_constants.py");
    let compiled = ScratchDir::new("py_literal_ext").expect("scratch");
    std::fs::copy(&fixture, compiled.join("m.py")).expect("copy the fixture");
    let build = Command::new(env!("CARGO_BIN_EXE_pycc"))
        .args(["build", "--ext", "--non-strict", "--select", "all"])
        .arg(compiled.join("m.py"))
        .arg("-o")
        .arg(compiled.join("m"))
        .output()
        .expect("pycc should spawn");
    assert!(build.status.success(), "{}", text(&build.stderr));
    std::fs::remove_file(compiled.join("m.py")).expect("import the artifact, not the source");
    let plain = ScratchDir::new("py_literal_src").expect("scratch");
    std::fs::copy(&fixture, plain.join("m.py")).expect("copy the fixture");
    let run = |dir: &Path| {
        python()
            .args(["-B", "-c", "import m"])
            .current_dir(dir)
            .output()
            .expect("python should spawn")
    };
    let (ours, oracle) = (run(&compiled), run(&plain));
    assert!(
        text(&oracle.stdout).starts_with("True True True\n"),
        "the oracle runs the fixture: {}",
        text(&oracle.stdout)
    );
    assert_eq!(text(&ours.stdout), text(&oracle.stdout), "{}", text(&ours.stderr));
    assert_eq!(ours.status.code(), Some(0), "{}", text(&ours.stderr));
}
