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

/// The module of `defaults_rebound_during_the_lookup_stay_alive`.
const REENTRANT_MODULE: &str = "\
from typing import Any


class K(metaclass=type):
    def f(self, a: Any, b: Any = [1, 2, 3], *, c: Any = 3) -> Any:
        return (a, len(b), b[0], c)
";

/// Its driver: each call leaves `b` and `c` to their defaults, and the key
/// of `c` in `__kwdefaults__` compares with an `__eq__` that drops
/// `__defaults__` while the forwarder looks `c` up, which frees the
/// default of `b` unless the call holds it. The first call takes the
/// defaulted path (`pycc_fwd_defaulted`); the second names `a` by an equal
/// but other string, so `pycc_fwd_bind` binds it. (Dropping
/// `__kwdefaults__` itself in `__eq__` crashes CPython 3.15 too, so there is
/// no oracle for it.)
const REENTRANT_DRIVER: &str = "\
import sys
sys.path.insert(0, sys.argv[1])
from {module} import K


class S(str):
    __hash__ = str.__hash__

    def __eq__(self, other):
        K.f.__defaults__ = None
        return str.__eq__(self, other)


class D(list):
    def __del__(self):
        print('del D')


K.f.__defaults__ = (D([1, 2, 3]),)
K.f.__kwdefaults__ = {S('c'): 7}
print(K().f(0))
K.f.__defaults__ = (D([4, 5]),)
print(K().f(**{''.join(['a']): 1}))
print('done')
";

#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn defaults_rebound_during_the_lookup_stay_alive() {
    let dir = ScratchDir::new("forwarder_defaults_rebound").expect("scratch");
    let module = "forwarder_defaults_rebound_ext";
    let pure = dir.join("pure");
    let native = dir.join("native");
    std::fs::create_dir_all(&pure).expect("pure dir");
    std::fs::create_dir_all(&native).expect("native dir");
    std::fs::write(pure.join(format!("{module}.py")), REENTRANT_MODULE).expect("write the module");
    std::fs::write(native.join(format!("{module}.py")), REENTRANT_MODULE).expect("write the module");
    let build = Command::new(env!("CARGO_BIN_EXE_pycc"))
        .args(["build", "--ext", "--non-strict", "--select", "all", "--report-fallbacks"])
        .arg(native.join(format!("{module}.py")))
        .arg("-o")
        .arg(native.join(module))
        .output()
        .expect("pycc should spawn");
    let report = text(&pycc_scratch::lf(&build.stderr));
    assert!(build.status.success(), "{report}");
    assert!(report.contains(&format!("compiled: {module}:K.f\n")), "{report}");
    std::fs::remove_file(native.join(format!("{module}.py"))).expect("remove the source");
    let driver = dir.join("driver.py");
    std::fs::write(&driver, REENTRANT_DRIVER.replace("{module}", module)).expect("write the driver");
    let run = |from: &Path| {
        // The debug allocator overwrites freed memory, so a default freed
        // under the call shows as a crash or a wrong value, not by luck.
        python()
            .args(["-X", "dev"])
            .arg(&driver)
            .arg(from)
            .env("PYTHONMALLOC", "debug")
            .env_remove("PYCC_NO_ATTR_CACHE")
            .output()
            .expect("python3 should spawn")
    };
    let oracle = run(&pure);
    assert!(oracle.status.success(), "{}", text(&pycc_scratch::lf(&oracle.stderr)));
    let ours = run(&native);
    assert!(ours.status.success(), "{:?}\n{}", ours.status, text(&pycc_scratch::lf(&ours.stderr)));
    assert_eq!(text(&pycc_scratch::lf(&ours.stdout)), text(&pycc_scratch::lf(&oracle.stdout)));
}
