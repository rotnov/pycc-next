//! Native and fallback code share one stdin/stdout. Whenever a CPython
//! interpreter is present (a non-strict build that embeds CPython for
//! fallback code, or an `--ext` module in its host), compiled `print`,
//! `input()`, `sys.stdin.readline()`/`read()` and `sys.stdout.write()` go
//! through CPython's own `sys.stdin`/`sys.stdout` (`pycc_rt::io`), so two
//! buffers never steal each other's input lines or reorder output.
//!
//! The oracle is `python3` (or `PYCC_PYTHON`) running the fixture with the
//! same piped stdin; stdout is a pipe too, so CPython block-buffers it.

use pycc_scratch::ScratchDir;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

const FIXTURE: &str = "non_strict_shared_io";

fn pycc() -> Command {
    Command::new(env!("CARGO_BIN_EXE_pycc"))
}

fn python() -> Command {
    Command::new(std::env::var_os("PYCC_PYTHON").unwrap_or_else(|| "python3".into()))
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).replace("\r\n", "\n")
}

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

fn stdin_bytes() -> Vec<u8> {
    std::fs::read(fixtures().join(format!("{FIXTURE}.stdin"))).expect("read the stdin fixture")
}

fn copy_fixture(dir: &Path, as_name: &str) -> PathBuf {
    let path = dir.join(as_name);
    std::fs::copy(fixtures().join(format!("{FIXTURE}.py")), &path).expect("copy the fixture");
    path
}

/// Runs `command` with the fixture's stdin piped in.
fn run_with_stdin(mut command: Command) -> Output {
    let mut child = command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn");
    child
        .stdin
        .take()
        .expect("stdin")
        .write_all(&stdin_bytes())
        .expect("write stdin");
    child.wait_with_output().expect("wait")
}

fn oracle() -> Output {
    let dir = ScratchDir::new("shared_io_oracle").expect("scratch");
    let source = copy_fixture(&dir, "m.py");
    let mut command = python();
    command.arg(source);
    let output = run_with_stdin(command);
    assert!(output.status.success(), "{}", text(&output.stderr));
    output
}

fn assert_same(ours: &Output, oracle: &Output) {
    assert_eq!(
        text(&ours.stdout),
        text(&oracle.stdout),
        "{}",
        text(&ours.stderr)
    );
    assert_eq!(ours.status.code(), oracle.status.code());
}

#[test]
fn native_and_fallback_io_interleave_like_cpython_in_an_embedded_build() {
    let dir = ScratchDir::new("shared_io_native").expect("scratch");
    let source = copy_fixture(&dir, "m.py");
    let build = pycc()
        .args(["build", "--non-strict", "--report-fallbacks"])
        .arg(&source)
        .arg("-o")
        .arg(dir.join("app"))
        .output()
        .expect("pycc should spawn");
    assert!(build.status.success(), "{}", text(&build.stderr));
    let report = text(&build.stderr);
    for name in ["ask", "echo_rest"] {
        assert!(
            report.contains(&format!("fallback: function `{name}` at ")),
            "{name} should fall back:\n{report}"
        );
    }
    let ours = run_with_stdin(Command::new(dir.join("app")));
    assert_same(&ours, &oracle());
}

#[test]
fn native_and_fallback_io_interleave_like_cpython_in_an_ext_module() {
    let dir = ScratchDir::new("shared_io_ext").expect("scratch");
    let source = copy_fixture(&dir, "shared_io_mod.py");
    let build = pycc()
        .args(["build", "--non-strict", "--ext"])
        .arg(&source)
        .arg("-o")
        .arg(dir.join("shared_io_mod"))
        .output()
        .expect("pycc should spawn");
    assert!(build.status.success(), "{}", text(&build.stderr));
    std::fs::remove_file(&source).expect("import the artifact, not the source");
    // No `-u`: the host's block-buffered `sys.stdout` is the only buffer.
    let mut host = python();
    host.args(["-c", "import shared_io_mod"]).current_dir(&*dir);
    assert_same(&run_with_stdin(host), &oracle());
}
