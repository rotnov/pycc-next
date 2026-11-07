//! Non-strict `--ext`: compiled code passing an object for a parameter
//! annotated with a class of the module (an instance read from an object
//! attribute, `use(h.c)`) gives CPython's results, and so does any other
//! object, which the compiled body uses as CPython's would. The oracle is
//! CPython running the fixture and the same driver.

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
        .args(["build", "--ext", "--non-strict", "--select", "all", "--report-fallbacks"])
        .arg(dir.join("coerced.py"))
        .arg("-o")
        .arg(dir.join("coerced"))
        .output()
        .expect("pycc should spawn");
    let report = text(&pycc_scratch::lf(&build.stderr));
    assert!(build.status.success(), "{report}");
    for function in ["use", "via", "main"] {
        assert!(report.contains(&format!("compiled: coerced:{function}\n")), "{report}");
    }
    let plain = ScratchDir::new("instance_coercion_src").expect("scratch");
    std::fs::copy(&source, plain.join("coerced.py")).expect("copy the fixture");
    std::fs::remove_file(dir.join("coerced.py")).expect("remove the source");
    let driver = "import coerced\ncoerced.main()\nclass H: pass\nh = H(); h.c = 5\ntry:\n    \
                  coerced.via(h)\nexcept (TypeError, AttributeError) as e:\n    print(type(e).__name__, e)\n";
    let run = |dir: &Path| {
        python()
            .args(["-c", driver])
            .current_dir(dir)
            .output()
            .expect("python3 should spawn")
    };
    let (ours, oracle) = (run(&dir), run(&plain));
    assert!(oracle.status.success(), "{}", text(&pycc_scratch::lf(&oracle.stderr)));
    assert_eq!(
        text(&pycc_scratch::lf(&ours.stdout)),
        text(&pycc_scratch::lf(&oracle.stdout)),
        "{}",
        text(&pycc_scratch::lf(&ours.stderr))
    );
}
