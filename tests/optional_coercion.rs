//! Non-strict `--ext`: a CPython object returned from, or passed to, a
//! compiled function as `int | None`, `float | None`, `bool | None` or
//! `str | None` passes as in CPython: the functions compile, and an object
//! of another type is returned unchecked, as CPython does not enforce the
//! annotation. The oracle is CPython running the fixture.

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
        .args(["build", "--ext", "--non-strict", "--select", "all", "--report-fallbacks"])
        .arg(dir.join("optional_coerced.py"))
        .arg("-o")
        .arg(dir.join("optional_coerced"))
        .output()
        .expect("pycc should spawn");
    let report = text(&pycc_scratch::lf(&build.stderr));
    assert!(build.status.success(), "{report}");
    for name in ["lookup", "name_of", "ratio", "flag", "via"] {
        assert!(report.contains(&format!("compiled: optional_coerced:{name}\n")), "{report}");
    }
    let oracle = python()
        .arg(&source)
        .output()
        .expect("python3 should spawn");
    std::fs::remove_file(dir.join("optional_coerced.py")).expect("remove the source");
    let ours = python()
        .args([
            "-c",
            "import optional_coerced\noptional_coerced.main()\n\
             print(optional_coerced.lookup({'k': 'v'}, 'k'))\n",
        ])
        .current_dir(&*dir)
        .output()
        .expect("python3 should spawn");
    assert_eq!(
        text(&pycc_scratch::lf(&ours.stdout)),
        format!(
            "{}v\n",
            text(&pycc_scratch::lf(&oracle.stdout))
        ),
        "{}",
        text(&pycc_scratch::lf(&ours.stderr))
    );
}
