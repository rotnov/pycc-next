//! Method calls from compiled code into compiled methods that leave later
//! parameters to their defaults (`tests/fixtures/forwarder_defaulted_calls.py`),
//! built `--ext --non-strict --select all`: the module prints what CPython
//! prints for its source, with the attribute caches on and off, while the
//! defaults change under it; and with the caches on, the calls that leave
//! the rest to the defaults go straight to the compiled body behind the
//! method's forwarder through the site's cache
//! (`pycc_ext_site_forward_calls`, `pycc_fwd_defaulted` in
//! `src/ext/module/forward_call.c`), or from the call site itself
//! (`pycc_ext_direct_calls`).

use pycc_scratch::ScratchDir;
use std::path::Path;
use std::process::Command;

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).replace("\r\n", "\n")
}

fn python() -> Command {
    Command::new(std::env::var_os("PYCC_PYTHON").unwrap_or_else(|| "python3".into()))
}

fn counter(stderr: &str, name: &str) -> usize {
    stderr
        .lines()
        .find_map(|l| l.strip_prefix(&format!("{name} ")))
        .and_then(|n| n.trim().parse().ok())
        .unwrap_or_else(|| panic!("no {name} line\n{stderr}"))
}

#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn defaulted_method_calls_match_cpython_and_reach_the_compiled_body() {
    let dir = ScratchDir::new("forwarder_defaulted_calls").expect("scratch");
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/forwarder_defaulted_calls.py");
    let module = "forwarder_defaulted_calls_ext";
    std::fs::copy(&source, dir.join(format!("{module}.py"))).expect("copy the fixture");
    let build = Command::new(env!("CARGO_BIN_EXE_pycc"))
        .args(["build", "--ext", "--non-strict", "--select", "all", "--report-fallbacks"])
        .arg(dir.join(format!("{module}.py")))
        .arg("-o")
        .arg(dir.join(module))
        .output()
        .expect("pycc should spawn");
    let report = text(&pycc_scratch::lf(&build.stderr));
    assert!(build.status.success(), "{report}");
    for name in ["K.get", "K.f", "K.defaulted", "K.missing"] {
        assert!(report.contains(&format!("compiled: {module}:{name}\n")), "{name}\n{report}");
    }
    // The extension runs without its source beside it, so the module's
    // `__file__` is the extension the counters are read from.
    std::fs::remove_file(dir.join(format!("{module}.py"))).expect("remove the source");
    let oracle = python().arg(&source).output().expect("python3 should spawn");
    assert!(oracle.status.success(), "{}", text(&pycc_scratch::lf(&oracle.stderr)));
    // `defaulted(150)` alone, then `main()`: a function called while the
    // module body runs is called before the patch, as bytecode.
    let probe = format!(
        "import ctypes, sys\nimport {module}\n\
         lib = ctypes.CDLL({module}.__file__)\n\
         lib.pycc_ext_direct_calls.restype = ctypes.c_longlong\n\
         lib.pycc_ext_site_forward_calls.restype = ctypes.c_longlong\n\
         {module}.K('y').defaulted(150)\n\
         print('FORWARD', lib.pycc_ext_site_forward_calls(), file=sys.stderr)\n\
         print('DIRECT', lib.pycc_ext_direct_calls(), file=sys.stderr)\n\
         {module}.main()\n"
    );
    for off in [false, true] {
        let mut run = python();
        run.args(["-c", &probe]).current_dir(&*dir);
        if off {
            run.env("PYCC_NO_ATTR_CACHE", "1");
        } else {
            run.env_remove("PYCC_NO_ATTR_CACHE");
        }
        let ours = run.output().expect("python3 should spawn");
        let stderr = text(&pycc_scratch::lf(&ours.stderr));
        assert!(ours.status.success(), "caches off: {off}\n{stderr}");
        assert_eq!(
            text(&pycc_scratch::lf(&ours.stdout)),
            text(&pycc_scratch::lf(&oracle.stdout)),
            "caches off: {off}\n{stderr}"
        );
        let forward = counter(&stderr, "FORWARD");
        let direct = counter(&stderr, "DIRECT");
        if off {
            assert_eq!(forward, 0, "{stderr}");
        } else {
            // Two calls per iteration, each leaving its last parameters to
            // their defaults.
            assert!(forward + direct >= 300, "{forward} + {direct} direct calls\n{stderr}");
        }
    }
}
