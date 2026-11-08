//! Tagged `int` and one-character `str` locals and `str` operations with
//! word operands in generic bodies (`tests/fixtures/tagged_words.py`), built
//! `--ext --non-strict --select all`: the module prints what CPython prints
//! for its source with the inline fast paths on (`pycc_ext_fast`
//! published) and off (`PYCC_NO_FAST_OPS=1`, every operation through
//! CPython's own).

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
fn tagged_words_match_cpython_with_the_fast_paths_on_and_off() {
    let dir = ScratchDir::new("tagged_words").expect("scratch");
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/tagged_words.py");
    let module = "tagged_words_ext";
    std::fs::copy(&source, dir.join(format!("{module}.py"))).expect("copy the fixture");
    let build = Command::new(env!("CARGO_BIN_EXE_pycc"))
        .args(["build", "--ext", "--non-strict", "--release", "--select", "all", "--report-fallbacks"])
        .arg(dir.join(format!("{module}.py")))
        .arg("-o")
        .arg(dir.join(module))
        .output()
        .expect("pycc should spawn");
    let report = text(&pycc_scratch::lf(&build.stderr));
    assert!(build.status.success(), "{report}");
    for name in ["skip", "word", "quoted", "tokens", "slices", "starts", "char_leaves", "counter", "compares", "pinned", "leaked", "main"] {
        assert!(report.contains(&format!("compiled: {module}:{name}\n")), "{name}\n{report}");
    }
    std::fs::remove_file(dir.join(format!("{module}.py"))).expect("remove the source");
    let oracle = python().arg(&source).output().expect("python3 should spawn");
    assert!(oracle.status.success(), "{}", text(&pycc_scratch::lf(&oracle.stderr)));
    // The module body runs `main()` before the patch, as bytecode; the
    // second call runs the compiled bodies.
    let probe = format!("import {module}\n{module}.main()\n");
    for off in [false, true] {
        let mut run = python();
        run.args(["-c", &probe]).current_dir(&*dir);
        if off {
            run.env("PYCC_NO_FAST_OPS", "1");
        } else {
            run.env_remove("PYCC_NO_FAST_OPS");
        }
        let ours = run.output().expect("python3 should spawn");
        let stderr = text(&pycc_scratch::lf(&ours.stderr));
        assert!(ours.status.success(), "fast paths off: {off}\n{stderr}");
        let oracle_out = text(&pycc_scratch::lf(&oracle.stdout));
        assert_eq!(
            text(&pycc_scratch::lf(&ours.stdout)),
            format!("{oracle_out}{oracle_out}"),
            "fast paths off: {off}\n{stderr}"
        );
    }
}
