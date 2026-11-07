//! v0.11 accept 3: what a whole-package extension reads by name while its
//! modules import is read by the interned key the module already holds, and
//! still answers as before, with and without `-O`.
//!
//! * A module's interface table (the functions other members call
//!   natively) is filled by the keys its body published them with
//!   (`pycc_ext_publish_key`): a call from another member is native while
//!   the name binds the published function, and goes through the new object
//!   once Python code rebinds it.
//!
//! The oracle is CPython running the same sources.

use pycc_scratch::ScratchDir;
use std::path::Path;
use std::process::{Command, Output};

fn pycc() -> Command {
    Command::new(env!("CARGO_BIN_EXE_pycc"))
}

fn python() -> Command {
    Command::new(std::env::var_os("PYCC_PYTHON").unwrap_or_else(|| "python3".into()))
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).replace("\r\n", "\n")
}

const INIT: &str = "\"\"\"Two members with fallback code and a native cross-module call.\"\"\"\n";

const A: &str = r#"import collections


class Tally(collections.OrderedDict):
    """Runs in CPython: its base is a CPython class."""

    def bump(self, key: str) -> int:
        assert (lambda: isinstance(key, str))(), "key " + repr(key)
        self[key] = self.get(key, 0) + 1
        return self[key]


def twice(n: int) -> int:
    return n * 2


def thrice(n: int) -> int:
    return n * 3
"#;

const B: &str = r#"import collections

from pki import a


class Log(collections.OrderedDict):
    """Runs in CPython too, in the second member imported."""

    def note(self, key: str) -> int:
        assert (lambda: key != "")(), "empty key"
        self[key] = len(self)
        return self[key]


def use(n: int) -> int:
    return a.twice(n) + a.thrice(n)
"#;

/// Imports the package, then exercises both members.
const DRIVER: &str = r#"
import sys
sys.path.insert(0, sys.argv[1])
from pki import a, b
t = a.Tally()
print(t.bump("x"), t.bump("x"), b.Log().note("k"))
for bad in (lambda: a.Tally().bump(3), lambda: b.Log().note("")):
    try:
        print(bad())
    except AssertionError as e:
        print("AssertionError", e)
print(b.use(5))
a.twice = lambda n: n + 100
print(b.use(5))
a.thrice = a.twice
print(b.use(1))
"#;

fn write_package(root: &Path) {
    let package = root.join("pki");
    std::fs::create_dir_all(&package).expect("create the package");
    for (name, source) in [("__init__.py", INIT), ("a.py", A), ("b.py", B)] {
        std::fs::write(package.join(name), source).expect("write a module");
    }
}

fn run(root: &Path, optimize: bool) -> Output {
    let mut command = python();
    command.arg("-I");
    if optimize {
        command.arg("-O");
    }
    command
        .args(["-c", DRIVER])
        .arg(root)
        .current_dir(root)
        .output()
        .expect("python3 should spawn")
}

#[test]
fn every_member_s_native_calls_answer_as_cpython_does() {
    let compiled = ScratchDir::new("package_import_names").expect("scratch");
    write_package(&compiled);
    let package = compiled.join("pki");
    let build = pycc()
        .arg("build")
        .arg(&package)
        .arg("-o")
        .arg(package.join("__init__"))
        .args(["--ext", "--non-strict", "--select", "all", "--report-fallbacks"])
        .output()
        .expect("pycc should spawn");
    let report = text(&build.stderr);
    assert!(build.status.success(), "{report}");
    assert!(
        report.contains("class `Tally`") && report.contains("class `Log`"),
        "{report}"
    );
    for name in ["__init__.py", "a.py", "b.py"] {
        std::fs::remove_file(package.join(name)).expect("import the artifact, not the source");
    }
    let plain = ScratchDir::new("package_import_names_src").expect("scratch");
    write_package(&plain);
    for optimize in [false, true] {
        let ours = run(&compiled, optimize);
        let oracle = run(&plain, optimize);
        let stderr = text(&ours.stderr);
        assert_eq!(ours.status.code(), Some(0), "{stderr}");
        assert_eq!(text(&ours.stdout), text(&oracle.stdout), "{stderr}");
    }
    let oracle = text(&run(&plain, false).stdout);
    assert_eq!(
        oracle, "1 2 0\nAssertionError key 3\nAssertionError empty key\n25\n120\n202\n",
        "the CPython reference itself"
    );
}
