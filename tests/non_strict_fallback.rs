//! Non-strict per-function fallback (#12): a top-level function or statement
//! pycc cannot compile yet runs as Python source in CPython, sharing the
//! module namespace with the compiled code.
//!
//! The oracle is `python3` (or `PYCC_PYTHON`) running the fixture itself.
//! The native arm builds an embedded executable bundling that interpreter;
//! the `--ext` arm imports the compiled module into it.

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

fn fixture(dir: &Path, as_name: &str) -> PathBuf {
    let source =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/non_strict_fallback.py");
    let path = dir.join(as_name);
    std::fs::copy(source, &path).expect("copy the fixture");
    path
}

/// The same run as CPython's: stdout, exit status and the exception line.
fn assert_matches(ours: &Output, oracle: &Output) {
    assert_eq!(
        text(&ours.stdout),
        text(&oracle.stdout),
        "{}",
        text(&ours.stderr)
    );
    assert_eq!(ours.status.code(), oracle.status.code());
    assert_eq!(last_line(&ours.stderr), last_line(&oracle.stderr));
}

/// Which top-level definitions fell back, from `--report-fallbacks`.
fn fell_back(report: &str, name: &str) -> bool {
    report
        .lines()
        .any(|line| line.starts_with(&format!("fallback: function `{name}` at ")))
}

fn assert_report(report: &str) {
    for name in ["boom", "bump"] {
        assert!(
            fell_back(report, name),
            "{name} should fall back:\n{report}"
        );
    }
    // `guarded` returns an `int` or a `str`: an object in an `--ext` module;
    // in a native build its `def` runs in CPython (`lifted:`), its body
    // compiled.
    for name in ["scale", "fails", "guarded", "describe"] {
        assert!(!fell_back(report, name), "{name} should compile:\n{report}");
    }
    // A lambda used as a value: CPython creates it, its body is compiled.
    assert!(
        report.contains("lifted: lambda `describe.<locals>.<lambda>`"),
        "{report}"
    );
}

#[test]
fn compiled_and_fallback_code_match_cpython_in_a_native_build() {
    let dir = ScratchDir::new("fallback_native").expect("scratch");
    let source = fixture(&dir, "m.py");
    let build = pycc()
        .args(["build", "--non-strict", "--report-fallbacks"])
        .arg(&source)
        .arg("-o")
        .arg(dir.join("app"))
        .output()
        .expect("pycc should spawn");
    assert!(build.status.success(), "{}", text(&build.stderr));
    assert_report(&text(&build.stderr));
    let ours = Command::new(dir.join("app"))
        .output()
        .expect("run the build");
    let oracle = python().arg(&source).output().expect("python should spawn");
    assert_eq!(oracle.status.code(), Some(1));
    assert_matches(&ours, &oracle);
}

#[test]
fn compiled_and_fallback_code_match_cpython_in_an_ext_module() {
    let compiled = ScratchDir::new("fallback_ext").expect("scratch");
    let source = fixture(&compiled, "fbm.py");
    let build = pycc()
        .args(["build", "--non-strict", "--ext", "--report-fallbacks"])
        .arg(&source)
        .arg("-o")
        .arg(compiled.join("fbm"))
        .output()
        .expect("pycc should spawn");
    assert!(build.status.success(), "{}", text(&build.stderr));
    assert_report(&text(&build.stderr));
    std::fs::remove_file(&source).expect("import the artifact, not the source");
    let plain = ScratchDir::new("fallback_ext_src").expect("scratch");
    fixture(&plain, "fbm.py");
    // Compiled `print` writes through the host's `sys.stdout`, so the
    // block-buffered pipe keeps program order without `-u`.
    let run = |dir: &Path| {
        python()
            .args(["-c", "import fbm"])
            .current_dir(dir)
            .output()
            .expect("python should spawn")
    };
    assert_matches(&run(&compiled), &run(&plain));
}

/// A public function fallback code calls, whose inferred signature the
/// CPython boundary cannot carry (`-> list[int]`), falls back itself rather
/// than stopping the build with the export planner's `C0003`.
#[test]
fn a_function_the_boundary_cannot_publish_falls_back() {
    let dir = ScratchDir::new("fallback_unpublishable").expect("scratch");
    let source = dir.join("m.py");
    std::fs::write(
        &source,
        "import sys\nI=lambda:[*map(int,sys.stdin.readline().split())]\nP=print\n\
         for _ in [0]*I()[0]:\n\tn,=I();p=I()\n\tP(n,p)\n",
    )
    .expect("write");
    let input = "1\n2\n3 4\n";
    let build = pycc()
        .args(["build", "--non-strict", "--report-fallbacks"])
        .arg(&source)
        .arg("-o")
        .arg(dir.join("app"))
        .output()
        .expect("pycc should spawn");
    assert!(build.status.success(), "{}{}", text(&build.stdout), text(&build.stderr));
    let run = |command: &mut Command| {
        let mut child = command
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .expect("spawn");
        use std::io::Write as _;
        child.stdin.take().expect("stdin").write_all(input.as_bytes()).expect("feed stdin");
        child.wait_with_output().expect("wait")
    };
    let ours = run(&mut Command::new(dir.join("app")));
    let oracle = run(python().arg(&source));
    assert_matches(&ours, &oracle);
}

/// A module-level fallback block binds globals compiled code reads, and a
/// compiled function the block calls is the published one.
#[test]
fn a_module_level_fallback_block_shares_globals_with_compiled_code() {
    let dir = ScratchDir::new("fallback_block").expect("scratch");
    let source = dir.join("m.py");
    std::fs::write(
        &source,
        "def inc(x: int) -> int:\n    return x + 1\n\n\
         pairs = [(k, inc(k)) for k in range(3) if (lambda q: q != 1)(k)]\n\
         n = len(pairs)\n\
         print(n, pairs)\n\
         n = inc(n)\n\
         print(n)\n",
    )
    .expect("write");
    let build = pycc()
        .args(["build", "--non-strict", "--report-fallbacks"])
        .arg(&source)
        .arg("-o")
        .arg(dir.join("app"))
        .output()
        .expect("pycc should spawn");
    assert!(build.status.success(), "{}", text(&build.stderr));
    let report = text(&build.stderr);
    assert!(report.contains("fallback: statement at"), "{report}");
    assert!(!fell_back(&report, "inc"), "{report}");
    let ours = Command::new(dir.join("app"))
        .output()
        .expect("run the build");
    let oracle = python().arg(&source).output().expect("python should spawn");
    assert_matches(&ours, &oracle);
}

#[test]
fn strict_mode_still_refuses_what_non_strict_runs_in_cpython() {
    let dir = ScratchDir::new("fallback_strict").expect("scratch");
    let source = dir.join("m.py");
    std::fs::write(
        &source,
        "def f(x: int) -> int:\n    g = lambda: x\n    h = g\n    return h()\n\nprint(f(2))\n",
    )
    .expect("write");
    let strict = pycc().arg("check").arg(&source).output().expect("spawn");
    assert_eq!(strict.status.code(), Some(1));
    assert!(
        text(&strict.stdout).contains("error[C0001]"),
        "{}",
        text(&strict.stdout)
    );
    let lenient = pycc()
        .args(["check", "--non-strict", "--report-fallbacks"])
        .arg(&source)
        .output()
        .expect("spawn");
    assert!(lenient.status.success(), "{}", text(&lenient.stdout));
    assert!(
        text(&lenient.stderr).contains("lifted: lambda `f.<locals>.<lambda>`"),
        "{}",
        text(&lenient.stderr)
    );
}

/// A syntax error is never a fallback.
#[test]
fn a_syntax_error_is_reported_in_non_strict_mode() {
    let dir = ScratchDir::new("fallback_syntax").expect("scratch");
    let source = dir.join("m.py");
    std::fs::write(&source, "def f(:\n    pass\n").expect("write");
    let output = pycc()
        .args(["check", "--non-strict"])
        .arg(&source)
        .output()
        .expect("spawn");
    assert_eq!(output.status.code(), Some(1));
    assert!(
        text(&output.stdout).contains("error[L0001]"),
        "{}",
        text(&output.stdout)
    );
}
