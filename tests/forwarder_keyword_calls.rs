//! Keyword method calls from compiled code into compiled methods
//! (`tests/fixtures/forwarder_keyword_calls.py`), built `--ext --non-strict
//! --select all`: the module prints what CPython prints for its source,
//! with the attribute caches on and off, and with the caches on, the calls
//! whose keywords name the parameters in order go straight to the compiled
//! body behind the method's forwarder (`pycc_ext_site_forward_calls`).

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
fn keyword_method_calls_match_cpython_and_reach_the_compiled_body() {
    let dir = ScratchDir::new("forwarder_keyword_calls").expect("scratch");
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/forwarder_keyword_calls.py");
    let module = "forwarder_keyword_calls_ext";
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
    for name in ["K.kw1", "K.kw3", "K.recurse", "K.calls"] {
        assert!(report.contains(&format!("compiled: {module}:{name}\n")), "{name}\n{report}");
    }
    // The extension runs without its source beside it, so the module's
    // `__file__` is the extension the counter is read from.
    std::fs::remove_file(dir.join(format!("{module}.py"))).expect("remove the source");
    let oracle = python().arg(&source).output().expect("python3 should spawn");
    assert!(oracle.status.success(), "{}", text(&pycc_scratch::lf(&oracle.stderr)));
    // `main()` after the import: a function called while the module body
    // runs is called before the patch, as bytecode.
    let probe = format!(
        "import ctypes, sys\nimport {module}\n{module}.main()\n\
         lib = ctypes.CDLL({module}.__file__)\n\
         print('FORWARD', lib.pycc_ext_site_forward_calls(), file=sys.stderr)\n"
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
        let forward: usize = stderr
            .lines()
            .find_map(|l| l.strip_prefix("FORWARD "))
            .and_then(|n| n.trim().parse().ok())
            .unwrap_or_else(|| panic!("no FORWARD line\n{stderr}"));
        if off {
            assert_eq!(forward, 0, "{stderr}");
        } else {
            // `calls(150)`: three in-order keyword calls per iteration (two
            // of `kw1`, one of `kw3`), the others bound.
            assert!(forward >= 450, "{forward} direct calls\n{stderr}");
        }
    }
}
