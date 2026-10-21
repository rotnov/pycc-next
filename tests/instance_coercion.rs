//! Non-strict `--ext`: a CPython object compiled code passes for a parameter
//! annotated with a class compiled in the module (an instance read from an
//! object attribute, `use(h.c)`) is converted to the instance its carrier
//! holds instead of the calling function falling back; any other object is
//! a `TypeError` naming the class (`docs/DEVIATIONS.md`). The oracle is
//! CPython running the fixture.

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
fn an_object_passed_for_a_compiled_class_parameter_is_its_instance() {
    let dir = ScratchDir::new("instance_coercion").expect("scratch");
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/instance_coercion.py");
    std::fs::copy(&source, dir.join("coerced.py")).expect("copy the fixture");
    let build = Command::new(env!("CARGO_BIN_EXE_pycc"))
        .args(["build", "--ext", "--non-strict", "--report-fallbacks"])
        .arg(dir.join("coerced.py"))
        .arg("-o")
        .arg(dir.join("coerced"))
        .output()
        .expect("pycc should spawn");
    let report = text(&pycc_scratch::lf(&build.stderr));
    assert!(build.status.success(), "{report}");
    assert!(!report.contains("function `via`"), "{report}");
    let oracle = python()
        .arg(&source)
        .output()
        .expect("python3 should spawn");
    std::fs::remove_file(dir.join("coerced.py")).expect("remove the source");
    let ours = python()
        .args([
            "-c",
            "import coerced\ncoerced.main()\nclass H: pass\nh = H(); h.c = 5\ntry:\n    \
             coerced.via(h)\nexcept TypeError as e:\n    print(e)\n",
        ])
        .current_dir(&*dir)
        .output()
        .expect("python3 should spawn");
    assert_eq!(
        text(&pycc_scratch::lf(&ours.stdout)),
        format!(
            "{}use() argument 1 must be coerced.Conf, not int\n",
            text(&pycc_scratch::lf(&oracle.stdout))
        ),
        "{}",
        text(&pycc_scratch::lf(&ours.stderr))
    );
}
