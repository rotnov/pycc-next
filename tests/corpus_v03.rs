//! v0.3 accept 3 (the real-world corpus): what the corpus needed natively.
//!
//! * The solver types the result of an operator over two concrete operands
//!   at once, so a builtin method or call over it (`a.count(m - 1)`) types
//!   the parameter it is passed to; a list display of such results is a
//!   concrete list.
//! * An empty list whose producer reads an unannotated function's parameter
//!   or return is typed once the solver has pinned that signature.
//! * An empty list bound to a name whose element nothing reads, produces or
//!   compares is a `list[int]`: no operation can observe its element type.
//! * A strict-mode return the solver cannot type (a returned tuple display)
//!   is typed from the function's body once its parameters are, and the
//!   solver runs again with it (`constraints::return_fallback`).
//! * `exit`/`quit` and `from sys import exit` are `sys.exit`; a module-level
//!   `X = sys.stdin` (or `sys.stdout`) reads `X` as the stream itself.
//! * Strict mode, like non-strict mode, does not compile an unannotated
//!   definition nothing in the entry module mentions
//!   (`corpus_round7::strict_mode_does_not_compile_dead_unannotated_definitions`).
//!
//! The oracle is CPython: each fixture is built, run and compared with
//! `python3` running the file -- stdout, the uncaught exception's last line
//! and the exit status.

use pycc_scratch::ScratchDir;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

fn pycc() -> Command {
    Command::new(env!("CARGO_BIN_EXE_pycc"))
}

fn python() -> Command {
    Command::new(std::env::var_os("PYCC_PYTHON").unwrap_or_else(|| "python3".into()))
}

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).replace("\r\n", "\n")
}

fn last_line(bytes: &[u8]) -> String {
    text(bytes).lines().last().unwrap_or_default().to_string()
}

/// Runs `command` with fixture `name`'s `.stdin` file (if any) as stdin.
fn run_with_stdin(mut command: Command, name: &str) -> Output {
    let stdin = fixture(&name.replace(".py", ".stdin"));
    let input = std::fs::File::open(&stdin).map_or_else(|_| Stdio::null(), Stdio::from);
    command.stdin(input).output().expect("the program runs")
}

/// Builds fixture `name` (plus `flags`) with `--release`, runs it and
/// compares with CPython. Returns the build's stderr.
fn matches_cpython(name: &str, flags: &[&str]) -> String {
    let dir = ScratchDir::new("corpus_v03").expect("scratch");
    let out = dir.join("prog");
    let build = pycc()
        .arg("build")
        .arg(fixture(name))
        .arg("-o")
        .arg(&out)
        .arg("--release")
        .args(flags)
        .output()
        .expect("pycc should spawn");
    assert!(
        build.status.success(),
        "{}{}",
        text(&build.stdout),
        text(&build.stderr)
    );
    let ours = run_with_stdin(Command::new(&out), name);
    let mut oracle = python();
    oracle.arg(fixture(name));
    let theirs = run_with_stdin(oracle, name);
    assert_eq!(
        text(&ours.stdout),
        text(&theirs.stdout),
        "stdout differs ({name})"
    );
    assert_eq!(last_line(&ours.stderr), last_line(&theirs.stderr), "{name}");
    assert_eq!(ours.status.code(), theirs.status.code(), "{name}");
    text(&build.stderr)
}

#[test]
fn inference_strict() {
    matches_cpython("corpus_v03_inference.py", &[]);
}

#[test]
fn inference_non_strict_without_fallback() {
    let report = matches_cpython(
        "corpus_v03_inference.py",
        &["--non-strict", "--report-fallbacks"],
    );
    assert!(!report.contains("fallback:"), "{report}");
}

/// `exit`/`quit` and `from sys import exit` are CPython's `sys.exit`, run
/// through the embedded interpreter.
#[test]
#[ignore = "needs CPython 3.13+ with a shared libpython (PYCC_PYTHON, default python3)"]
fn exit_is_sys_exit() {
    for flags in [&[][..], &["--non-strict"][..]] {
        matches_cpython("corpus_v03_exit.py", flags);
        matches_cpython("corpus_v03_exit_from_sys.py", flags);
    }
}

#[test]
fn a_stream_alias_is_the_stream() {
    matches_cpython("corpus_v03_stream_alias.py", &[]);
    let report = matches_cpython(
        "corpus_v03_stream_alias.py",
        &["--non-strict", "--report-fallbacks"],
    );
    assert!(!report.contains("fallback:"), "{report}");
}
