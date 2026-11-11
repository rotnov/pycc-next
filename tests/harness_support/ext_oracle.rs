//! Builds a fixture as a non-strict `--ext` module (`--select all`), or as a
//! strict one ([`check_strict`]), and compares what its compiled functions
//! print with CPython's output for the source, with the fast paths on and
//! off (`PYCC_NO_FAST_OPS=1`). Shared by `tests/idiom_lowerings.rs` and
//! `tests/handler_exceptions.rs`.

use pycc_scratch::ScratchDir;
use std::path::Path;
use std::process::Command;

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).replace("\r\n", "\n")
}

fn python() -> Command {
    Command::new(std::env::var_os("PYCC_PYTHON").unwrap_or_else(|| "python3".into()))
}

/// The marker a probe prints between what `main()` printed and the rest.
const MARK: &str = "--- only with the lowerings on ---\n";

/// Builds `tests/fixtures/<fixture>.py` as an extension module, checks that
/// `compiled` functions were compiled, and compares what it prints with
/// CPython's output for the source, with the lowerings on and off. `extra`
/// names a function of the fixture that only the run with the lowerings on
/// calls after `main()`. `counter` names a counter of the shim and whether
/// it moves: with the lowerings on it must have moved then, and with them
/// off, or when it does not move, it must read zero.
pub fn check(fixture: &str, compiled: &[&str], counter: Option<(&str, bool)>, extra: Option<&str>) {
    let dir = ScratchDir::new(fixture).expect("scratch");
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join(format!("tests/fixtures/{fixture}.py"));
    let module = format!("{fixture}_ext");
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
        .arg(dir.join(&module))
        .output()
        .expect("pycc should spawn");
    let report = text(&pycc_scratch::lf(&build.stderr));
    assert!(build.status.success(), "{report}");
    for name in compiled {
        assert!(
            report.contains(&format!("compiled: {module}:{name}\n")),
            "{name}\n{report}"
        );
    }
    std::fs::remove_file(dir.join(format!("{module}.py"))).expect("remove the source");

    // CPython's output: the module body runs `main()`; `extra` follows the
    // marker.
    let oracle_dir = ScratchDir::new(&format!("{fixture}_oracle")).expect("scratch");
    std::fs::copy(&source, oracle_dir.join(format!("{fixture}_src.py"))).expect("copy the fixture");
    let extra_call = |name: &str| extra.map(|f| format!("{name}.{f}()\n")).unwrap_or_default();
    let oracle_probe = format!(
        "import sys\nimport {fixture}_src\nsys.stdout.write({MARK:?})\n{}",
        extra_call(&format!("{fixture}_src"))
    );
    let oracle = python()
        .args(["-c", &oracle_probe])
        .current_dir(&*oracle_dir)
        .output()
        .expect("python3 should spawn");
    assert!(
        oracle.status.success(),
        "{}",
        text(&pycc_scratch::lf(&oracle.stderr))
    );
    let oracle_out = text(&pycc_scratch::lf(&oracle.stdout));
    let (main_out, extra_out) = oracle_out
        .split_once(MARK)
        .expect("the oracle prints the marker");
    assert!(!main_out.is_empty());

    for off in [false, true] {
        // The module body runs `main()` before the patch, as bytecode; the
        // second call runs the compiled bodies.
        let count_probe = counter.map_or(String::new(), |(counter, _)| {
            format!(
                "lib = ctypes.CDLL({module}.__file__)\n\
                 lib.{counter}.restype = ctypes.c_longlong\n\
                 print('COUNT', lib.{counter}(), file=sys.stderr)\n"
            )
        });
        let probe = format!(
            "import ctypes, sys\nimport {module}\n{module}.main()\n{}{count_probe}",
            if off {
                String::new()
            } else {
                extra_call(&module)
            }
        );
        let mut run = python();
        run.args(["-c", &probe]).current_dir(&*dir);
        if off {
            run.env("PYCC_NO_FAST_OPS", "1");
        } else {
            run.env_remove("PYCC_NO_FAST_OPS");
        }
        let ours = run.output().expect("python3 should spawn");
        let stderr = text(&pycc_scratch::lf(&ours.stderr));
        assert!(ours.status.success(), "lowerings off: {off}\n{stderr}");
        let expected = format!("{main_out}{main_out}{}", if off { "" } else { extra_out });
        assert_eq!(
            text(&pycc_scratch::lf(&ours.stdout)),
            expected,
            "lowerings off: {off}\n{stderr}"
        );
        let Some((counter, moves)) = counter else {
            continue;
        };
        let count: i64 = stderr
            .lines()
            .find_map(|line| line.strip_prefix("COUNT "))
            .and_then(|n| n.parse().ok())
            .unwrap_or_else(|| panic!("no counter line\n{stderr}"));
        if off || !moves {
            assert_eq!(count, 0, "{counter}, lowerings off: {off}\n{stderr}");
        } else {
            assert!(count > 0, "{counter} never moved\n{stderr}");
        }
    }
}


/// Builds `tests/fixtures/<fixture>.py` as a strict `--ext` module, every
/// function of which is compiled, and compares what
/// `tests/fixtures/<driver>.py`'s `run(module)` prints for it with what it
/// prints for the source under CPython, with the fast paths on and off.
#[allow(dead_code)]
pub fn check_strict(fixture: &str, driver: &str) {
    let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    let source = fixtures.join(format!("{fixture}.py"));
    let driver_source = fixtures.join(format!("{driver}.py"));
    let run = |dir: &Path, module: &str, off: bool| {
        std::fs::copy(&driver_source, dir.join(format!("{driver}.py"))).expect("copy the driver");
        let mut python = python();
        python
            .args(["-c", &format!("import {driver}, {module}\n{driver}.run({module})\n")])
            .current_dir(dir);
        if off {
            python.env("PYCC_NO_FAST_OPS", "1");
        } else {
            python.env_remove("PYCC_NO_FAST_OPS");
        }
        let out = python.output().expect("python3 should spawn");
        let stderr = text(&pycc_scratch::lf(&out.stderr));
        assert!(out.status.success(), "{module}, lowerings off: {off}\n{stderr}");
        text(&pycc_scratch::lf(&out.stdout))
    };

    let oracle_dir = ScratchDir::new(&format!("{fixture}_oracle")).expect("scratch");
    let oracle_module = format!("{fixture}_src");
    std::fs::copy(&source, oracle_dir.join(format!("{oracle_module}.py"))).expect("copy the fixture");
    let expected = run(&oracle_dir, &oracle_module, false);
    assert!(!expected.is_empty());

    let dir = ScratchDir::new(fixture).expect("scratch");
    let module = format!("{fixture}_ext");
    std::fs::copy(&source, dir.join(format!("{module}.py"))).expect("copy the fixture");
    let build = Command::new(env!("CARGO_BIN_EXE_pycc"))
        .args(["build", "--ext", "--release"])
        .arg(dir.join(format!("{module}.py")))
        .arg("-o")
        .arg(dir.join(&module))
        .output()
        .expect("pycc should spawn");
    assert!(build.status.success(), "{}", text(&pycc_scratch::lf(&build.stderr)));
    // Only the extension is left to import.
    std::fs::remove_file(dir.join(format!("{module}.py"))).expect("remove the source");
    for off in [false, true] {
        assert_eq!(run(&dir, &module, off), expected, "lowerings off: {off}");
    }
}
