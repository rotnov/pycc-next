//! Unpacked calls between compiled bodies in generic code
//! (`tests/fixtures/multi_returns.py`, `pycc_mir::multi_return_object`),
//! built `--ext --non-strict --select all`: the module prints what CPython
//! prints for its source, with the direct calls (and so the multi-value
//! twins) on and off (`PYCC_NO_FAST_OPS=1`, every call through the
//! callee's forwarder and every result unpacked whole).

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
fn unpacked_calls_match_cpython_with_and_without_twins() {
    let dir = ScratchDir::new("multi_returns").expect("scratch");
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/multi_returns.py");
    let module = "multi_returns_ext";
    std::fs::copy(&source, dir.join(format!("{module}.py"))).expect("copy the fixture");
    let build = Command::new(env!("CARGO_BIN_EXE_pycc"))
        .args([
            "build",
            "--ext",
            "--non-strict",
            "--release",
            "--select",
            "all",
            "--report-fallbacks",
        ])
        .arg(dir.join(format!("{module}.py")))
        .arg("-o")
        .arg(dir.join(module))
        .output()
        .expect("pycc should spawn");
    let report = text(&pycc_scratch::lf(&build.stderr));
    assert!(build.status.success(), "{report}");
    for name in [
        "pair",
        "triple",
        "fwd",
        "fwd_kw",
        "fwd_fwd",
        "whole",
        "fwd_whole",
        "boom",
        "deep_boom",
        "depth",
        "me",
        "who",
        "keep",
        "scan",
        "scan_kw",
        "unpack_one",
        "unpack_fwd",
        "call_deep",
        "call_whole_int",
        "call_depth",
        "frames",
        "keep_loop",
        "call_pair",
    ] {
        assert!(
            report.contains(&format!("compiled: {module}:{name}\n")),
            "{name}\n{report}"
        );
    }
    // The extension runs without its source beside it, so the module's
    // `__file__` is the extension the counter is read from.
    std::fs::remove_file(dir.join(format!("{module}.py"))).expect("remove the source");
    let oracle = python()
        .arg(&source)
        .output()
        .expect("python3 should spawn");
    assert!(
        oracle.status.success(),
        "{}",
        text(&pycc_scratch::lf(&oracle.stderr))
    );
    // `main()` after the import: a function called while the module body
    // runs is called before the patch, as bytecode.
    let probe = format!(
        "import ctypes, sys\nimport {module}\n{module}.main()\n\
         lib = ctypes.CDLL({module}.__file__)\n\
         lib.pycc_ext_direct_calls.restype = ctypes.c_longlong\n\
         print('DIRECT', lib.pycc_ext_direct_calls(), file=sys.stderr)\n"
    );
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
        assert_eq!(
            text(&pycc_scratch::lf(&ours.stdout)),
            text(&pycc_scratch::lf(&oracle.stdout)),
            "fast paths off: {off}\n{stderr}"
        );
        let direct: i64 = stderr
            .lines()
            .find_map(|line| line.strip_prefix("DIRECT "))
            .and_then(|count| count.parse().ok())
            .unwrap_or_else(|| panic!("no direct-call count\n{stderr}"));
        // On: `keep_loop` alone makes a thousand direct calls.
        if off {
            assert_eq!(direct, 0, "{stderr}");
        } else {
            assert!(direct >= 1000, "{direct}\n{stderr}");
        }
    }
}
