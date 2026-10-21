//! A read of a name bound on some paths only compiles in strict mode too:
//! every global and non-parameter local carries a run-time initialized
//! flag, so the read raises CPython's `UnboundLocalError` / `NameError`
//! exactly where CPython does (`pycc_types::non_strict::admits_maybe_read`).
//! A name CPython would read something else for when unbound (a builtin,
//! `__name__`), a `del`/`except ... as` name and a buffer keep the static
//! `T0041`.

use pycc_scratch::ScratchDir;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn pycc() -> Command {
    Command::new(env!("CARGO_BIN_EXE_pycc"))
}

fn oracle() -> std::ffi::OsString {
    std::env::var_os("PYCC_PYTHON").unwrap_or_else(|| "python3".into())
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).replace("\r\n", "\n")
}

fn fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/maybe_unbound_runtime.py")
}

fn build_and_run(dir: &Path, source: &Path, flags: &[&str]) -> Output {
    let binary = dir.join(format!("app{}", flags.len()));
    let build = pycc()
        .arg("build")
        .arg(source)
        .arg("-o")
        .arg(&binary)
        .arg("--release")
        .args(flags)
        .output()
        .expect("pycc should spawn");
    assert!(
        build.status.success(),
        "pycc build {flags:?} failed:\n{}{}",
        text(&pycc_scratch::lf(&build.stdout)),
        text(&pycc_scratch::lf(&build.stderr))
    );
    Command::new(&binary).output().expect("the binary runs")
}

fn last_line(bytes: &[u8]) -> String {
    text(bytes).trim_end().lines().last().unwrap_or("").to_string()
}

#[test]
fn possibly_unbound_reads_raise_like_cpython() {
    let dir = ScratchDir::new("maybe_unbound_fixture").expect("scratch");
    let expected = Command::new(oracle()).arg(fixture()).output().expect("python3");
    assert!(expected.status.success(), "{}", text(&pycc_scratch::lf(&expected.stderr)));
    for flags in [&[][..], &["--non-strict"][..]] {
        let run = build_and_run(&dir, &fixture(), flags);
        assert!(run.status.success(), "{flags:?}: {}", text(&pycc_scratch::lf(&run.stderr)));
        assert_eq!(text(&pycc_scratch::lf(&run.stdout)), text(&pycc_scratch::lf(&expected.stdout)), "{flags:?}");
    }
}

/// An uncaught one ends the program as CPython's does: the same stdout,
/// exit status `1` and final exception line.
#[test]
fn an_uncaught_unbound_read_ends_the_program_like_cpython() {
    let dir = ScratchDir::new("maybe_unbound_uncaught").expect("scratch");
    for (name, source) in [
        (
            "local",
            "def f(n: int) -> int:\n    if n > 0:\n        y = n\n    return y\n\n\nprint(f(1))\nprint(f(0))\n",
        ),
        ("global", "for i in range(2):\n    print(i)\nfor j in range(0):\n    pass\nprint(i, j)\n"),
    ] {
        let source_path = dir.join(format!("{name}.py"));
        std::fs::write(&source_path, source).expect("write the source");
        let run = build_and_run(&dir, &source_path, &[]);
        let expected = Command::new(oracle()).arg(&source_path).output().expect("python3");
        assert_eq!(text(&pycc_scratch::lf(&run.stdout)), text(&pycc_scratch::lf(&expected.stdout)), "{name}");
        assert_eq!(run.status.code(), Some(1), "{name}");
        assert_eq!(last_line(&pycc_scratch::lf(&run.stderr)), last_line(&pycc_scratch::lf(&expected.stderr)), "{name}");
    }
}

/// CPython reads the builtin (or the module's own `__name__`) when the
/// module leaves such a name unbound, so a possibly-unbound read of one
/// cannot raise and keeps the static refusal.
#[test]
fn a_possibly_unbound_name_shadowing_a_builtin_keeps_t0041() {
    let dir = ScratchDir::new("maybe_unbound_builtin").expect("scratch");
    for source in [
        "flag: bool = len(\"ab\") > 5\nif flag:\n    len = 3\nprint(len)\n",
        "flag: bool = True\nif flag:\n    __name__ = 7\nprint(__name__)\n",
    ] {
        let path = dir.join("m.py");
        std::fs::write(&path, source).expect("write the source");
        let check = pycc().arg("check").arg(&path).output().expect("pycc should spawn");
        assert!(!check.status.success(), "{source}");
        let rendered = format!("{}{}", text(&pycc_scratch::lf(&check.stdout)), text(&pycc_scratch::lf(&check.stderr)));
        assert!(rendered.contains("error[T0041]"), "{source}: {rendered}");
    }
}
