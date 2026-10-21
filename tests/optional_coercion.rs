//! Non-strict `--ext`: a CPython object returned from, or passed to, a
//! compiled function as `int | None`, `float | None`, `bool | None` or
//! `str | None` is `None` or converted to the scalar, instead of the function
//! falling back; any other object is a `TypeError`
//! (`docs/DEVIATIONS.md`). The oracle is CPython running the fixture.

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
fn an_object_reaches_an_optional_scalar_slot() {
    let dir = ScratchDir::new("optional_coercion").expect("scratch");
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/optional_coercion.py");
    std::fs::copy(&source, dir.join("optional_coerced.py")).expect("copy the fixture");
    let build = Command::new(env!("CARGO_BIN_EXE_pycc"))
        .args(["build", "--ext", "--non-strict", "--report-fallbacks"])
        .arg(dir.join("optional_coerced.py"))
        .arg("-o")
        .arg(dir.join("optional_coerced"))
        .output()
        .expect("pycc should spawn");
    let report = text(&pycc_scratch::lf(&build.stderr));
    assert!(build.status.success(), "{report}");
    for name in ["lookup", "name_of", "ratio", "flag", "via"] {
        assert!(!report.contains(&format!("function `{name}`")), "{report}");
    }
    let oracle = python()
        .arg(&source)
        .output()
        .expect("python3 should spawn");
    std::fs::remove_file(dir.join("optional_coerced.py")).expect("remove the source");
    let ours = python()
        .args([
            "-c",
            "import optional_coerced\noptional_coerced.main()\ntry:\n    \
             optional_coerced.lookup({'k': 'v'}, 'k')\nexcept TypeError as e:\n    print(e)\n",
        ])
        .current_dir(&*dir)
        .output()
        .expect("python3 should spawn");
    assert_eq!(
        text(&pycc_scratch::lf(&ours.stdout)),
        format!(
            "{}return value must be int, not str\n",
            text(&pycc_scratch::lf(&oracle.stdout))
        ),
        "{}",
        text(&pycc_scratch::lf(&ours.stderr))
    );
}
