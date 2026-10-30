//! A module-level type variable (`T = TypeVar("T")`) is made once, as
//! CPython makes it, when its `TypeVar` import runs in CPython in a
//! `--non-strict` build: compiled code no longer calls `TypeVar` beside the
//! CPython copy of the assignment, and reads the one object Python code
//! sees. Found as 39 type variables against CPython's 22 importing pydantic.
//!
//! The oracle is `python3` (or `PYCC_PYTHON`): `v011_type_vars_driver.py`
//! imports `v011_type_vars_api.py` built `--ext --non-strict` and as source.

use pycc_scratch::ScratchDir;
use std::path::Path;
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

fn fixtures(dir: &Path) {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    std::fs::copy(root.join("v011_type_vars_api.py"), dir.join("api.py")).expect("copy");
    std::fs::copy(root.join("v011_type_vars_driver.py"), dir.join("driver.py")).expect("copy");
}

fn run(dir: &Path) -> Output {
    python()
        .arg("driver.py")
        .current_dir(dir)
        .output()
        .expect("python should spawn")
}

#[test]
fn a_type_variable_is_made_once() {
    let compiled = ScratchDir::new("v011_type_vars_ext").expect("scratch");
    fixtures(&compiled);
    let build = pycc()
        .args(["build", "--ext", "--non-strict", "--report-fallbacks", "api.py", "-o", "api"])
        .current_dir(&*compiled)
        .output()
        .expect("pycc should spawn");
    let report = text(&build.stderr);
    assert!(build.status.success(), "{report}");
    assert!(
        report.contains("`TypeVar` runs in CPython, so the type variable is made there"),
        "{report}"
    );
    std::fs::remove_file(compiled.join("api.py")).expect("import the artifact, not the source");
    let plain = ScratchDir::new("v011_type_vars_src").expect("scratch");
    fixtures(&plain);
    let oracle = run(&plain);
    assert!(
        text(&oracle.stdout).contains("made ['T', 'U']"),
        "the oracle runs the driver: {}{}",
        text(&oracle.stdout),
        text(&oracle.stderr)
    );
    let ours = run(&compiled);
    assert_eq!(text(&ours.stdout), text(&oracle.stdout), "{}", text(&ours.stderr));
    assert_eq!(ours.status.code(), oracle.status.code());
}
