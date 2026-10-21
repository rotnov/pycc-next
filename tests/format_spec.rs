//! Format specs (`f"{x:>8.3f}"`, `format(x, spec)`) and printf-style `%`
//! formatting compile natively (`pycc_rt::format`) and match CPython 3.13,
//! in strict and `--non-strict` builds.

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
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/format_spec.py")
}

/// Builds `source` with `flags`, runs it and returns the output.
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
    if flags.contains(&"--report-fallbacks") {
        assert!(!text(&pycc_scratch::lf(&build.stderr)).contains("fallback:"), "{}", text(&pycc_scratch::lf(&build.stderr)));
    }
    Command::new(&binary).output().expect("the binary runs")
}

fn cpython(source: &Path) -> Output {
    Command::new(oracle()).arg(source).output().expect("python3 should spawn")
}

#[test]
fn format_specs_and_printf_match_cpython() {
    let dir = ScratchDir::new("format_spec").expect("scratch");
    let expected = cpython(&fixture());
    assert!(expected.status.success(), "{}", text(&pycc_scratch::lf(&expected.stderr)));
    for flags in [&[][..], &["--non-strict", "--report-fallbacks"][..]] {
        let run = build_and_run(&dir, &fixture(), flags);
        assert!(run.status.success(), "{flags:?}: {}", text(&pycc_scratch::lf(&run.stderr)));
        assert_eq!(text(&pycc_scratch::lf(&run.stdout)), text(&pycc_scratch::lf(&expected.stdout)), "{flags:?}");
    }
}

/// A conversion of an argument type CPython refuses at run time is a
/// compile-time `T0021` in strict mode; `--non-strict` hands the expression
/// to CPython, which raises its own `TypeError`.
#[test]
fn a_printf_conversion_of_the_wrong_type_is_refused_in_strict_mode() {
    let dir = ScratchDir::new("format_spec_refused").expect("scratch");
    let source = dir.join("m.py");
    std::fs::write(
        &source,
        "x = 1.5\ntry:\n    print('%x' % x)\nexcept TypeError as e:\n    print('TypeError', e)\n",
    )
    .expect("write the source");
    let check = pycc().arg("check").arg(&source).output().expect("pycc should spawn");
    assert!(!check.status.success());
    let rendered = format!("{}{}", text(&pycc_scratch::lf(&check.stdout)), text(&pycc_scratch::lf(&check.stderr)));
    assert!(rendered.contains("error[T0021]"), "{rendered}");
    let run = build_and_run(&dir, &source, &["--non-strict"]);
    assert_eq!(text(&pycc_scratch::lf(&run.stdout)), text(&cpython(&source).stdout));
}

/// A spec with a nested replacement field is not compiled in strict mode
/// (`C0001`); `--non-strict` runs the statement in CPython.
#[test]
fn a_nested_spec_field_is_refused_in_strict_mode() {
    let dir = ScratchDir::new("format_spec_nested").expect("scratch");
    let source = dir.join("m.py");
    std::fs::write(&source, "w = 6\nx = 2.5\nprint(f\"[{x:>{w}}]\")\n").expect("write the source");
    let check = pycc().arg("check").arg(&source).output().expect("pycc should spawn");
    assert!(!check.status.success());
    let rendered = format!("{}{}", text(&pycc_scratch::lf(&check.stdout)), text(&pycc_scratch::lf(&check.stderr)));
    assert!(rendered.contains("error[C0001]") && rendered.contains("nested field"), "{rendered}");
    let run = build_and_run(&dir, &source, &["--non-strict"]);
    assert_eq!(text(&pycc_scratch::lf(&run.stdout)), "[   2.5]\n");
}
