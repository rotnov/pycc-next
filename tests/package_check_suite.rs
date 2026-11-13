//! `bench/package_check.py` and `bench/wheel_check.py` run each real package's own
//! unittest suite through `SuiteLoader` (`bench/package_check_workloads.py`), which
//! drops exactly the upstream tests that assert only a wall-time ratio (their names
//! end in `_near_linear`) and keeps every other test. Every suite run of both scripts,
//! for every package, loads its tests with it, so the rule is the same for each tree
//! and build mode.

use std::process::Command;

const SCRIPT: &str = r#"
import io, os, sys, unittest, contextlib
sys.path.insert(0, os.path.join(sys.argv[1], "bench"))
import package_check as pc
import wheel_check as wc
from package_check_workloads import SUITE_LOADER

ns = {}
exec(SUITE_LOADER, ns)

class T(unittest.TestCase):
    def test_render(self): pass
    def test_x_are_near_linear(self): self.fail("timed")
    def test_y_is_near_linear(self): self.fail("timed")
    def test_near_linear_output_is_escaped(self): pass
    def test_returns_quickly(self): pass

out = io.StringIO()
with contextlib.redirect_stdout(out):
    print(ns["SuiteLoader"]().getTestCaseNames(T))
    suite = ns["SuiteLoader"]().loadTestsFromTestCase(T)
print(out.getvalue(), end="")
r = unittest.TextTestRunner(stream=io.StringIO()).run(suite)
print("ran", r.testsRun, "ok", r.wasSuccessful())

# Every suite run of both scripts defines the loader and passes it to each
# `unittest.main`.
runs = [(f"package {n}", p.tests) for n, p in pc.PACKAGES.items()]
runs += [(f"harness {n}", h.run) for n, h in wc.HARNESS.items()]
for name, code in runs:
    mains = code.count("unittest.main(")
    if not code.startswith(SUITE_LOADER) or mains == 0 \
            or code.count("unittest.main(testLoader=SuiteLoader(),") != mains:
        print("not through SuiteLoader:", name)
print("suites", len(runs))
"#;

#[test]
#[ignore = "requires python3 (PYCC_PYTHON, default python3)"]
fn package_suites_exclude_only_wall_time_ratio_tests() {
    let python = std::env::var_os("PYCC_PYTHON").unwrap_or_else(|| "python3".into());
    let out = Command::new(python)
        .args(["-I", "-B", "-c", SCRIPT, env!("CARGO_MANIFEST_DIR")])
        .output()
        .expect("python");
    let stdout = pycc_scratch::lf(&*String::from_utf8_lossy(&out.stdout)).replace("\r\n", "\n");
    assert!(out.status.success(), "{stdout}{}", pycc_scratch::lf(&*String::from_utf8_lossy(&out.stderr)));
    assert_eq!(
        stdout,
        "excluded (asserts only a wall-time ratio): __main__.T.test_x_are_near_linear\n\
         excluded (asserts only a wall-time ratio): __main__.T.test_y_is_near_linear\n\
         ['test_near_linear_output_is_escaped', 'test_render', 'test_returns_quickly']\n\
         excluded (asserts only a wall-time ratio): __main__.T.test_x_are_near_linear\n\
         excluded (asserts only a wall-time ratio): __main__.T.test_y_is_near_linear\n\
         ran 3 ok True\n\
         suites 11\n"
    );
}
