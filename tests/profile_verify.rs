//! `pycc profile`'s pass 3 (docs/design/compile-little.md, section 2.3): the
//! selection rule's candidates are built and timed against CPython, and only
//! measured winners stay selected.
//!
//! The interpreter is `python3` (or `PYCC_PYTHON`), 3.12 or later.

use pycc_scratch::ScratchDir;
use std::path::Path;
use std::process::{Command, Output};

fn python() -> std::ffi::OsString {
    std::env::var_os("PYCC_PYTHON").unwrap_or_else(|| "python3".into())
}

fn pycc(args: &[&std::ffi::OsStr]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_pycc"))
        .args(args)
        .output()
        .expect("pycc runs")
}

/// `pycc profile PKG -o OUT -- PYTHON WORK`.
fn profile(pkg: &Path, out: &Path, work: &Path) -> Output {
    pycc(&[
        "profile".as_ref(),
        pkg.as_os_str(),
        "-o".as_ref(),
        out.as_os_str(),
        "--".as_ref(),
        &python(),
        work.as_os_str(),
    ])
}

fn check(pkg: &Path, out: &Path) -> String {
    let r = pycc(&[
        "profile".as_ref(),
        pkg.as_os_str(),
        "-o".as_ref(),
        out.as_os_str(),
        "--check".as_ref(),
        "--".as_ref(),
        &python(),
    ]);
    assert!(r.status.success(), "{}", String::from_utf8_lossy(&r.stderr));
    String::from_utf8_lossy(&r.stdout).into_owned()
}

const MODULE: &str = "\
def leaf(a, b, c):
    return a


def work(n):
    s = 0
    for i in range(n):
        s += i * i % 7
    return s
";

/// A package `pkg` whose module `m` has the two functions, and `work.py`.
fn package(dir: &Path, work: &str) -> (std::path::PathBuf, std::path::PathBuf) {
    let pkg = dir.join("pkg");
    std::fs::create_dir_all(&pkg).unwrap();
    std::fs::write(pkg.join("__init__.py"), "").unwrap();
    std::fs::write(pkg.join("m.py"), MODULE).unwrap();
    let script = dir.join("work.py");
    std::fs::write(&script, work).unwrap();
    (pkg, script)
}

fn measured(out: &Path, qualname: &str) -> serde_json::Value {
    let p: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(out).unwrap()).unwrap();
    p["functions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|f| f["qualname"] == qualname)
        .unwrap_or_else(|| panic!("{qualname}: {p}"))["measured"]
        .clone()
}

#[test]
fn a_leaf_called_from_the_interpreter_is_rejected_as_slower_when_compiled() {
    let dir = ScratchDir::new("profile_verify").unwrap();
    // Each call crosses from bytecode into the compiled body and back, which
    // costs more than CPython's own call of so small a function.
    let (pkg, work) = package(
        &dir,
        "from pkg import m\n\
         leaf = m.leaf\n\
         for i in range(500000):\n    leaf(i, 1, 2)\n    leaf(i, 1, 2)\n    leaf(i, 1, 2)\n    leaf(i, 1, 2)\n",
    );
    let out = dir.join("p.json");
    let r = profile(&pkg, &out, &work);
    let stderr = String::from_utf8_lossy(&r.stderr);
    assert!(r.status.success(), "{stderr}");
    assert!(stderr.contains("pass 3: 1 functions compiled: ") && stderr.contains("): slower"), "{stderr}");
    assert!(stderr.contains("pass 3 kept 0 of 1 candidates, measured in 1 builds in "), "{stderr}");
    let m = measured(&out, "leaf");
    assert_eq!(m["compiled"], false, "{m}");
    assert_eq!(m["with"], 1);
    assert!(m["compiled_ns"][1].as_u64() > m["cpython_ns"][2].as_u64(), "{m}");
    let shown = check(&pkg, &out);
    assert!(
        shown.contains("rejected: pkg.m:leaf: slower when compiled: the workload took "),
        "{shown}"
    );
    // Profiled again, it stays rejected without a build: its source is the
    // same.
    let r = profile(&pkg, &out, &work);
    let stderr = String::from_utf8_lossy(&r.stderr);
    assert!(r.status.success(), "{stderr}");
    assert!(!stderr.contains("pass 3"), "{stderr}");
    assert_eq!(measured(&out, "leaf"), m);
    assert!(check(&pkg, &out).contains("rejected: pkg.m:leaf: slower when compiled"));
}

#[test]
fn a_measured_winner_stays_selected() {
    let dir = ScratchDir::new("profile_verify").unwrap();
    // An integer loop inside one call: the compiled body runs it natively.
    let (pkg, work) = package(&dir, "from pkg import m\nfor i in range(60):\n    m.work(20000)\n");
    let out = dir.join("p.json");
    let r = profile(&pkg, &out, &work);
    let stderr = String::from_utf8_lossy(&r.stderr);
    assert!(r.status.success(), "{stderr}");
    assert!(stderr.contains("pass 3 kept 1 of 1 candidates"), "{stderr}");
    let m = measured(&out, "work");
    assert_eq!(m["compiled"], true, "{m}");
    assert!(m["compiled_ns"][1].as_u64() <= m["cpython_ns"][2].as_u64(), "{m}");
    assert!(check(&pkg, &out).contains("selected: pkg.m:work"));
}

#[test]
fn a_member_that_imports_its_sibling_is_measured() {
    let dir = ScratchDir::new("profile_verify").unwrap();
    let pkg = dir.join("pkg");
    std::fs::create_dir_all(&pkg).unwrap();
    std::fs::write(pkg.join("__init__.py"), "").unwrap();
    std::fs::write(pkg.join("helper.py"), "value = 7\n").unwrap();
    std::fs::write(pkg.join("m.py"), format!("from .helper import value\n\n\n{MODULE}")).unwrap();
    let work = dir.join("work.py");
    std::fs::write(&work, "from pkg import m\nassert m.value == 7\nfor i in range(60):\n    m.work(20000)\n").unwrap();
    let out = dir.join("p.json");
    // Only the member is profiled; its copy for pass 3 brings the package.
    let r = profile(&pkg.join("m.py"), &out, &work);
    let stderr = String::from_utf8_lossy(&r.stderr);
    assert!(r.status.success(), "{stderr}");
    assert!(stderr.contains("pass 3 kept 1 of 1 candidates"), "{stderr}");
    assert_eq!(measured(&out, "work")["compiled"], true);
}
