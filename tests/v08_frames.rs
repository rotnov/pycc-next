//! v0.8 accept 3: compiled functions run in real frames.
//!
//! `tests/fixtures/v08_frames_ext.py` is built with `--ext --non-strict` and
//! imported by `tests/fixtures/v08_frames_driver.py`; the oracle is the same
//! driver importing the source. They must print the same: `sys._getframe`
//! chains through compiled and Python callers (`f_code`, `f_lineno`,
//! `f_back`, `f_globals`, a module frame's `f_locals`), `inspect.stack()`,
//! tracebacks of exceptions raised in and propagating through compiled
//! frames (file, line, function and source line of each entry, causes, a
//! traceback formatted inside a compiled handler), `warnings.warn`'s
//! `stacklevel`, module globals rebound from Python, a private function
//! published, and constant memory over a churn of raises and frame walks.

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

fn fixture(name: &str, dir: &Path) -> PathBuf {
    let source = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(format!("{name}.py"));
    let path = dir.join(format!("{name}.py"));
    std::fs::copy(source, &path).expect("copy the fixture");
    path
}

fn run_driver(dir: &Path, module_dir: &Path) -> Output {
    let driver = fixture("v08_frames_driver", dir);
    python()
        .arg(driver)
        .arg(module_dir)
        .output()
        .expect("python should spawn")
}

#[test]
#[ignore = "requires a CPython 3.13 interpreter with headers (PYCC_PYTHON, default python3)"]
fn compiled_frames_behave_as_cpython_frames() {
    let compiled = ScratchDir::new("v08_frames_ext").expect("scratch");
    let plain = ScratchDir::new("v08_frames_py").expect("scratch");
    let source = fixture("v08_frames_ext", &compiled);
    fixture("v08_frames_ext", &plain);
    let build = pycc()
        .args(["build", "--ext", "--non-strict"])
        .arg(&source)
        .arg("-o")
        .arg(compiled.join("v08_frames_ext"))
        .output()
        .expect("pycc should spawn");
    assert!(build.status.success(), "{}", text(&build.stderr));
    let ours = run_driver(&compiled, &compiled);
    let oracle = run_driver(&plain, &plain);
    assert_eq!(
        text(&ours.stdout),
        text(&oracle.stdout),
        "{}",
        text(&ours.stderr)
    );
    assert_eq!(
        ours.status.code(),
        oracle.status.code(),
        "{}",
        text(&ours.stderr)
    );
    assert!(text(&ours.stdout).contains("memory constant"));
}

/// A native program with fallback code runs in an embedded CPython: its
/// uncaught exception's traceback is CPython's, compiled frames included.
#[test]
#[ignore = "requires a CPython 3.13 interpreter with a shared libpython (PYCC_PYTHON, default python3)"]
fn an_embedded_programs_traceback_matches_cpython() {
    let dir = ScratchDir::new("v08_frames_embedded").expect("scratch");
    let source = dir.join("app.py");
    std::fs::write(
        &source,
        "import math\n\n\ndef root(x):\n    return math.sqrt(x)\n\n\n\
         def run(xs):\n    return [root(x) for x in xs]\n\n\nprint(run([4.0, -1.0]))\n",
    )
    .unwrap();
    let build = pycc()
        .current_dir(&*dir)
        .args(["build", "--non-strict", "app.py", "-o"])
        .arg(dir.join("app"))
        .output()
        .expect("pycc should spawn");
    assert!(build.status.success(), "{}", text(&build.stderr));
    let ours = Command::new(dir.join("app"))
        .current_dir(&*dir)
        .output()
        .unwrap();
    let oracle = python().current_dir(&*dir).arg("app.py").output().unwrap();
    assert_eq!(ours.status.code(), oracle.status.code());
    assert_eq!(text(&ours.stderr), text(&oracle.stderr));
}
