//! A failing operation on CPython objects in a generic body
//! (`tests/fixtures/object_failure_edges.py`), built `--ext --non-strict
//! --select all`: each operation routes its own failure through
//! `foreign_fail`'s edge, with no pending-exception check after the node
//! (`exception::expression_can_set_exception`), so the failure is caught by
//! a handler in the same function, or reaches the caller, before anything
//! after it runs -- as in CPython, with the inline fast paths on and off
//! (`PYCC_NO_FAST_OPS=1`).

use pycc_scratch::ScratchDir;
use std::path::Path;
use std::process::Command;

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(&pycc_scratch::lf(bytes)).into_owned()
}

fn python() -> Command {
    Command::new(std::env::var_os("PYCC_PYTHON").unwrap_or_else(|| "python3".into()))
}

#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn failing_object_operations_match_cpython() {
    let dir = ScratchDir::new("object_failure_edges").expect("scratch");
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/object_failure_edges.py");
    let module = "object_failure_edges_ext";
    std::fs::copy(&source, dir.join(format!("{module}.py"))).expect("copy the fixture");
    let build = Command::new(env!("CARGO_BIN_EXE_pycc"))
        .args(["build", "--ext", "--non-strict", "--release", "--select", "all", "--report-fallbacks"])
        .arg(dir.join(format!("{module}.py")))
        .arg("-o")
        .arg(dir.join(module))
        .output()
        .expect("pycc should spawn");
    let report = text(&build.stderr);
    assert!(build.status.success(), "{report}");
    for name in [
        "attr", "method", "call", "length", "subscript", "unpack", "compare", "arith", "display", "nested",
        "uncaught", "main",
    ] {
        assert!(report.contains(&format!("compiled: {module}:{name}\n")), "{name}\n{report}");
    }
    std::fs::remove_file(dir.join(format!("{module}.py"))).expect("remove the source");
    let oracle = python().arg(&source).output().expect("python3 should spawn");
    assert!(oracle.status.success(), "{}", text(&oracle.stderr));
    // `main()` after the import: a function called while the module body
    // runs is called before the patch, as bytecode.
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
        let stderr = text(&ours.stderr);
        assert!(ours.status.success(), "fast paths off: {off}\n{stderr}");
        assert_eq!(text(&ours.stdout), text(&oracle.stdout), "fast paths off: {off}\n{stderr}");
    }
}
