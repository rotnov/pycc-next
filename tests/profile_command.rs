//! `pycc profile` (docs/design/compile-little.md, section 2.2): the profile
//! of a workload against a package's sources, its determinism, and the
//! staleness check a build applies (`--check`).
//!
//! The interpreter is `python3` (or `PYCC_PYTHON`), 3.12 or later
//! (`sys.monitoring`).

use pycc_scratch::ScratchDir;
use std::path::Path;
use std::process::{Command, Output};

fn python() -> std::ffi::OsString {
    std::env::var_os("PYCC_PYTHON").unwrap_or_else(|| "python3".into())
}

fn profile(pkg: &Path, out: &Path, extra: &[&str], workload: &[&std::ffi::OsStr]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_pycc"))
        .arg("profile")
        .arg(pkg)
        .arg("-o")
        .arg(out)
        .args(extra)
        .arg("--")
        .arg(python())
        .args(workload)
        .output()
        .expect("pycc runs")
}

const HOT: &str = "\
def hot(d, k):
    try:
        return d[k]
    except KeyError:
        return None


def gen(n):
    yield from range(n)


class Box:
    def __init__(self, items):
        self.items = items

    def total(self):
        return sum(hot(self.items, k) or 0 for k in range(10))

    @property
    def size(self):
        return len(self.items)
";

const WORKLOAD: &str = "\
import sys
sys.path.insert(0, sys.argv[1])
from pkg import hot
for i in range(400):
    hot.hot({1: 2}, i % 3)
    hot.hot([1], 0)
b = hot.Box({1: 2})
for i in range(60):
    b.total()
    b.size
list(hot.gen(3))
";

fn package(dir: &Path) -> std::path::PathBuf {
    let pkg = dir.join("pkg");
    std::fs::create_dir_all(&pkg).unwrap();
    std::fs::write(pkg.join("__init__.py"), "").unwrap();
    std::fs::write(pkg.join("hot.py"), HOT).unwrap();
    std::fs::write(dir.join("work.py"), WORKLOAD).unwrap();
    pkg
}

fn read(path: &Path) -> serde_json::Value {
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

/// Everything in a profile but its times.
fn untimed(v: &serde_json::Value) -> serde_json::Value {
    let mut v = v.clone();
    v["total_ns"] = 0.into();
    for f in v["functions"].as_array_mut().unwrap() {
        f["self_ns"] = 0.into();
    }
    v
}

#[test]
fn a_profile_records_calls_self_time_callees_and_argument_types() {
    let dir = ScratchDir::new("profile_cmd").unwrap();
    let pkg = package(&dir);
    let out = dir.join("p.json");
    let work = dir.join("work.py");
    let r = profile(&pkg, &out, &["--runs", "2", "--select-share", "0"], &[work.as_os_str(), dir.as_os_str()]);
    assert!(r.status.success(), "{}", String::from_utf8_lossy(&r.stderr));
    let p = read(&out);
    assert_eq!(p["version"], 1);
    assert_eq!(p["runs"], 2);
    assert!(p["total_ns"].as_u64().unwrap() > 0);
    let fs = p["functions"].as_array().unwrap();
    let names: Vec<&str> = fs.iter().map(|f| f["qualname"].as_str().unwrap()).collect();
    // Sorted by module and qualified name; the property accessor and the
    // generator are recorded too.
    let mut sorted = names.clone();
    sorted.sort();
    assert_eq!(names, sorted);
    let get = |q: &str| fs.iter().find(|f| f["qualname"] == q).unwrap_or_else(|| panic!("{q}: {p}"));
    let hot = get("hot");
    assert_eq!(hot["module"], "pkg.hot");
    assert_eq!(hot["firstlineno"], 1);
    assert_eq!(hot["calls"], 800 + 600);
    assert_eq!(hot["sha256"].as_str().unwrap().len(), 64);
    assert_eq!(hot["arg_types"]["d"]["dict"], 400 + 600 - 400);
    assert_eq!(hot["arg_types"]["d"]["list"], 400);
    assert_eq!(hot["arg_types"]["k"]["int"], 1000);
    assert_eq!(get("Box.total")["arg_types"]["self"]["pkg.hot.Box"], 60);
    assert_eq!(get("Box.size")["calls"], 60);
    // Calls to Python functions, by callee: `hot` calls none; `Box.total`
    // calls `hot` from its generator expression, which counts as its own.
    assert_eq!(hot["callees"], serde_json::json!({}));
    assert_eq!(get("Box.total")["callees"], serde_json::json!({"pkg.hot:hot:1": 600}));
    assert!(!names.iter().any(|q| q.contains("<genexpr>")), "{names:?}");
    // The generator is not eligible, so pass 2 records nothing for it.
    assert_eq!(get("gen")["arg_types"], serde_json::json!({}));
    // A second profile of the same workload: the same file but for the times.
    let again = dir.join("q.json");
    let r = profile(&pkg, &again, &["--runs", "2", "--select-share", "0"], &[work.as_os_str(), dir.as_os_str()]);
    assert!(r.status.success());
    assert_eq!(untimed(&read(&again)), untimed(&p));
}

#[test]
fn check_drops_stale_entries_and_prints_the_selection() {
    let dir = ScratchDir::new("profile_cmd").unwrap();
    let pkg = package(&dir);
    let out = dir.join("p.json");
    let work = dir.join("work.py");
    let r = profile(&pkg, &out, &["--runs", "1"], &[work.as_os_str(), dir.as_os_str()]);
    assert!(r.status.success(), "{}", String::from_utf8_lossy(&r.stderr));
    // `hot` changes; `Box.size` moves down a line but keeps its text.
    let changed = HOT
        .replace("return None", "return 0")
        .replace("    @property", "\n    @property");
    std::fs::write(pkg.join("hot.py"), changed).unwrap();
    let r = profile(&pkg, &out, &["--check", "--select-share", "0", "--select-calls", "1"], &[]);
    assert!(r.status.success(), "{}", String::from_utf8_lossy(&r.stderr));
    let stderr = String::from_utf8_lossy(&r.stderr);
    let stdout = String::from_utf8_lossy(&r.stdout);
    assert!(
        stderr.contains("note: profile entry pkg.hot:hot (line 1) dropped: its source changed"),
        "{stderr}"
    );
    assert!(stdout.contains("selected: pkg.hot:Box.size"), "{stdout}");
    assert!(stdout.contains("rejected: pkg.hot:gen: not eligible"), "{stdout}");
    assert!(!stdout.contains("pkg.hot:hot "), "{stdout}");
    std::fs::remove_file(pkg.join("hot.py")).unwrap();
    let r = profile(&pkg, &out, &["--check"], &[]);
    assert!(String::from_utf8_lossy(&r.stderr).contains("dropped: it is no longer in the sources"));
}

#[test]
fn a_workload_that_fails_or_is_missing_is_an_error() {
    let dir = ScratchDir::new("profile_cmd").unwrap();
    let pkg = package(&dir);
    let out = dir.join("p.json");
    let bad = dir.join("bad.py");
    std::fs::write(&bad, "raise SystemExit(3)\n").unwrap();
    let r = profile(&pkg, &out, &[], &[bad.as_os_str()]);
    assert!(!r.status.success());
    assert!(String::from_utf8_lossy(&r.stderr).contains("the profile's calls pass failed"));
    assert!(!out.exists());
    let r = profile(&pkg, &out, &[], &[]);
    assert!(String::from_utf8_lossy(&r.stderr).contains("needs a workload"));
    let r = profile(&dir.join("nope"), &out, &[], &[bad.as_os_str()]);
    assert!(String::from_utf8_lossy(&r.stderr).contains("no such package or module"));
}
