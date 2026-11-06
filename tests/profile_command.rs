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

/// Passes 1 and 2 only: pass 3 (`tests/profile_verify.rs`) builds and times.
fn profile(pkg: &Path, out: &Path, extra: &[&str], workload: &[&std::ffi::OsStr]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_pycc"))
        .arg("profile")
        .arg("--no-verify")
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
    // Of those 1,000 calls, the ones that caught `KeyError`: the dict calls
    // with `i % 3 != 1` (267 of the first loop's 400), then 9 of each
    // `Box.total`'s 10 (20 of them before the sampling ends).
    assert_eq!(hot["handled"], 267 + 9 * 20);
    assert_eq!(get("Box.size")["handled"], 0);
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

/// A profile whose workload sees `PYTHONHASHSEED=0` (set-iteration order, so
/// call counts, are then the same every run), no other `PYTHON*` variable,
/// and the profiled sources first on `sys.path`, wherever the script is.
#[test]
fn the_workload_runs_with_a_fixed_hash_seed_against_the_profiled_sources() {
    let dir = ScratchDir::new("profile_cmd").unwrap();
    let pkg = package(&dir);
    // A decoy copy of the package on `PYTHONPATH`, and the script elsewhere.
    let decoy = dir.join("decoy");
    std::fs::create_dir_all(decoy.join("pkg")).unwrap();
    std::fs::write(decoy.join("pkg/__init__.py"), "").unwrap();
    std::fs::write(decoy.join("pkg/hot.py"), "def hot(d, k):\n    return None\n").unwrap();
    let away = dir.join("away");
    std::fs::create_dir_all(&away).unwrap();
    let work = away.join("seeded.py");
    std::fs::write(
        &work,
        "import sys\n\
         if sys.flags.hash_randomization:\n    raise SystemExit(5)\n\
         from pkg import hot\n\
         hot.hot({}, 0)\n\
         for s in {'a', 'b', 'c', 'd', 'e', 'f', 'g', 'h'}:\n    if s == 'd':\n        break\n    hot.hot({}, s)\n",
    )
    .unwrap();
    let run = |out: &Path| {
        Command::new(env!("CARGO_BIN_EXE_pycc"))
            .arg("profile")
            .arg(&pkg)
            .arg("-o")
            .arg(out)
            .args(["--runs", "1", "--select-share", "0", "--select-calls", "1"])
            .arg("--")
            .arg(python())
            .arg(&work)
            .env("PYTHONPATH", &decoy)
            .env("PYTHONHASHSEED", "random")
            .output()
            .expect("pycc runs")
    };
    let (a, b) = (dir.join("a.json"), dir.join("b.json"));
    for out in [&a, &b] {
        let r = run(out);
        assert!(r.status.success(), "{}", String::from_utf8_lossy(&r.stderr));
    }
    let p = read(&a);
    let hot = p["functions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|f| f["qualname"] == "hot")
        .unwrap_or_else(|| panic!("{p}"))
        .clone();
    // The profiled sources' `hot` (line 1 of HOT, its hash), not the decoy's.
    assert_eq!(hot["module"], "pkg.hot");
    assert_eq!(untimed(&read(&b)), untimed(&p));
    // Under PYTHONHASHSEED=0 the set's order is fixed: the count is the one
    // CPython gives with that seed (plus the first call).
    let expected = Command::new(python())
        .args(["-c", "n = 0\nfor s in {'a', 'b', 'c', 'd', 'e', 'f', 'g', 'h'}:\n    if s == 'd':\n        break\n    n += 1\nprint(n + 1)"])
        .env("PYTHONHASHSEED", "0")
        .output()
        .unwrap();
    let expected: u64 = String::from_utf8_lossy(&expected.stdout).trim().parse().unwrap();
    assert_eq!(hot["calls"], expected);
}

#[test]
fn a_workload_that_imports_none_of_the_package_or_another_copy_is_an_error() {
    let dir = ScratchDir::new("profile_cmd").unwrap();
    let pkg = package(&dir);
    let out = dir.join("p.json");
    let none = dir.join("none.py");
    std::fs::write(&none, "print('nothing')\n").unwrap();
    let r = profile(&pkg, &out, &[], &[none.as_os_str()]);
    assert!(!r.status.success());
    assert!(
        String::from_utf8_lossy(&r.stderr).contains("the workload imported no module of"),
        "{}",
        String::from_utf8_lossy(&r.stderr)
    );
    // A workload that puts another copy first itself.
    let other = dir.join("other");
    std::fs::create_dir_all(other.join("pkg")).unwrap();
    std::fs::write(other.join("pkg/__init__.py"), "").unwrap();
    let elsewhere = dir.join("elsewhere.py");
    std::fs::write(
        &elsewhere,
        format!("import sys\nsys.path.insert(0, {:?})\nimport pkg\n", other.to_str().unwrap()),
    )
    .unwrap();
    let r = profile(&pkg, &out, &[], &[elsewhere.as_os_str()]);
    assert!(!r.status.success());
    assert!(
        String::from_utf8_lossy(&r.stderr).contains("the workload imported pkg from"),
        "{}",
        String::from_utf8_lossy(&r.stderr)
    );
}

/// A module of a package, profiled on its own, is named and imported as the
/// package's member (`pkg.hot`, from above `pkg`), and a module in another
/// encoding than UTF-8 is read in its own.
#[test]
fn a_package_member_and_a_latin_1_module_are_profiled() {
    let dir = ScratchDir::new("profile_cmd").unwrap();
    let pkg = package(&dir);
    std::fs::write(
        pkg.join("latin.py"),
        b"# -*- coding: latin-1 -*-\ndef caf\xe9():\n    return '\xe9'\n" as &[u8],
    )
    .unwrap();
    let away = dir.join("away");
    std::fs::create_dir_all(&away).unwrap();
    let work = away.join("member.py");
    std::fs::write(
        &work,
        "from pkg.hot import hot\nfrom pkg import latin\nfor i in range(60):\n    hot({}, i)\n    latin.caf\u{e9}()\n",
    )
    .unwrap();
    let out = dir.join("p.json");
    let r = profile(&pkg.join("hot.py"), &out, &["--runs", "1"], &[work.as_os_str()]);
    assert!(r.status.success(), "{}", String::from_utf8_lossy(&r.stderr));
    let p = read(&out);
    let modules: Vec<&str> = p["functions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f["module"].as_str().unwrap())
        .collect();
    assert!(!modules.is_empty() && modules.iter().all(|m| *m == "pkg.hot"), "{p}");
    let r = profile(&pkg, &out, &["--runs", "1"], &[work.as_os_str()]);
    assert!(r.status.success(), "{}", String::from_utf8_lossy(&r.stderr));
    let p = read(&out);
    let cafe = p["functions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|f| f["module"] == "pkg.latin")
        .unwrap_or_else(|| panic!("{p}"));
    assert_eq!(cafe["qualname"], "caf\u{e9}");
    assert_eq!(cafe["calls"], 60);
}

/// A function's source text is its lines as `ast` numbers them: a U+2028 or a
/// form feed inside a line does not end it, so a change to a later function
/// is seen.
#[test]
fn source_hashes_follow_ast_lines() {
    let dir = ScratchDir::new("profile_cmd").unwrap();
    let pkg = dir.join("pkg");
    std::fs::create_dir_all(&pkg).unwrap();
    std::fs::write(pkg.join("__init__.py"), "").unwrap();
    let src = "def f():\n    return \"a\u{2028}b\x0cc\"\n\n\ndef g():\n    return 1\n";
    std::fs::write(pkg.join("m.py"), src).unwrap();
    let work = dir.join("w.py");
    std::fs::write(&work, "from pkg import m\nfor _ in range(60):\n    m.f(); m.g()\n").unwrap();
    let out = dir.join("p.json");
    let r = profile(&pkg, &out, &["--runs", "1"], &[work.as_os_str()]);
    assert!(r.status.success(), "{}", String::from_utf8_lossy(&r.stderr));
    std::fs::write(pkg.join("m.py"), src.replace("return 1", "return 2")).unwrap();
    let r = profile(&pkg, &out, &["--check"], &[]);
    let stderr = String::from_utf8_lossy(&r.stderr);
    assert!(
        stderr.contains("note: profile entry pkg.m:g (line 5) dropped: its source changed"),
        "{stderr}"
    );
    assert!(!stderr.contains("pkg.m:f "), "{stderr}");
}

/// The run's total self time, the selection's denominator, is the workload's
/// alone: the check that it imported the package's modules runs after the
/// profiler stops. Here that check reads a module's `__file__`, which runs a
/// package function; the profile must not see that call.
#[test]
fn the_profile_leaves_out_the_driver_s_own_check() {
    let dir = ScratchDir::new("profile_cmd").unwrap();
    let pkg = dir.join("pkg");
    std::fs::create_dir_all(&pkg).unwrap();
    std::fs::write(pkg.join("__init__.py"), "").unwrap();
    std::fs::write(
        pkg.join("lazy.py"),
        "import sys\n\n\ndef spin():\n    return sum(range(100000))\n\n\n\
         class Lazy:\n    @property\n    def __file__(self):\n        spin()\n        return FILE\n\n\n\
         FILE = __file__\nsys.modules[__name__] = Lazy()\n",
    )
    .unwrap();
    let work = dir.join("w.py");
    std::fs::write(&work, "import pkg.lazy\n").unwrap();
    let out = dir.join("p.json");
    let r = profile(&pkg, &out, &["--runs", "1"], &[work.as_os_str()]);
    assert!(r.status.success(), "{}", String::from_utf8_lossy(&r.stderr));
    let p = read(&out);
    let names: Vec<&str> = p["functions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f["qualname"].as_str().unwrap())
        .collect();
    assert!(!names.contains(&"spin") && !names.contains(&"Lazy.__file__"), "{p}");
}
