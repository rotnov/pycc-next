//! A selected function's compiled body is specialized for the exact types a
//! profile observed its parameters holding (docs/design/compile-little.md,
//! section 2.5): a guard tests them at each call and runs the native copy,
//! and any other argument -- another type, or a subclass of the observed one
//! -- runs the generic compiled body. Both paths must behave as CPython:
//! the same output and the same exceptions.

use pycc_scratch::ScratchDir;
use std::path::Path;
use std::process::{Command, Output};

fn python() -> std::ffi::OsString {
    std::env::var_os("PYCC_PYTHON").unwrap_or_else(|| "python3".into())
}

fn pycc(args: &[&std::ffi::OsStr]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_pycc")).args(args).output().expect("pycc runs")
}

fn text(o: &Output) -> String {
    format!("{}{}", String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr)).replace('\r', "")
}

const MODULE: &str = "\
def count(s, c):
    n = 0
    for i in range(len(s)):
        if s[i] == c:
            n += 1
    return n


def scale(xs, k):
    total = 0.0
    for x in xs:
        total += x * k
    return total
";

/// The training workload: `count` always with two `str`s, `scale` with a
/// `list` and a `float`.
const WORK: &str = "\
from pkg import m
for i in range(1200):
    m.count('abcab' * 3, 'a')
    m.scale([1.0, 2.0, 3.0], 0.5)
";

/// Calls on both paths: the observed exact types, other types, a `str`
/// subclass that changes indexing, and arguments that raise.
const CALLS: &str = "\
import sys
sys.path.insert(0, sys.argv[1])
from pkg import m

class Loud(str):
    def __getitem__(self, i):
        return str.__getitem__(self, i).upper()

class Weird(str):
    def __len__(self):
        return 2

def show(f, *args):
    try:
        print(f.__name__, repr(f(*args)))
    except Exception as e:
        print(f.__name__, type(e).__name__, e)

show(m.count, 'banana', 'a')
show(m.count, '', 'a')
show(m.count, ['a', 'b', 'a'], 'a')
show(m.count, ('a', 1, 'a'), 1)
show(m.count, Loud('banana'), 'A')
show(m.count, Loud('banana'), 'a')
show(m.count, Weird('aaaa'), 'a')
show(m.count, 'banana', Loud('a'))
show(m.count, 5, 'a')
show(m.count, None, 'a')
show(m.count, 'abc', None)
show(m.scale, [1.0, 2.0], 2.0)
show(m.scale, [1, 2], 3)
show(m.scale, 'ab', 2)
show(m.scale, [1.0], 'x')
show(m.scale, None, 1.0)
";

/// A package `pkg` with `m`, profiled (no pass 3) and built with `count`
/// and `scale` selected and specialized for the observed types; returns the
/// built directory, the plain one, and the build's report.
fn built(scratch: &Path) -> (std::path::PathBuf, std::path::PathBuf, String) {
    let mut dirs = Vec::new();
    for name in ["built", "plain"] {
        let pkg = scratch.join(name).join("pkg");
        std::fs::create_dir_all(&pkg).unwrap();
        std::fs::write(pkg.join("__init__.py"), "").unwrap();
        std::fs::write(pkg.join("m.py"), MODULE).unwrap();
        dirs.push(scratch.join(name));
    }
    let work = scratch.join("work.py");
    std::fs::write(&work, WORK).unwrap();
    let profile = scratch.join("p.json");
    let pkg = dirs[0].join("pkg");
    let o = pycc(&[
        "profile".as_ref(),
        pkg.as_os_str(),
        "-o".as_ref(),
        profile.as_os_str(),
        "--no-verify".as_ref(),
        "--".as_ref(),
        &python(),
        work.as_os_str(),
    ]);
    assert!(o.status.success(), "{}", text(&o));
    let out = pkg.join("__init__");
    let o = pycc(&[
        "build".as_ref(),
        pkg.as_os_str(),
        "-o".as_ref(),
        out.as_os_str(),
        "--ext".as_ref(),
        "--non-strict".as_ref(),
        "--report-fallbacks".as_ref(),
        "--select".as_ref(),
        "none".as_ref(),
        "--compile".as_ref(),
        "pkg.m:count".as_ref(),
        "--compile".as_ref(),
        "pkg.m:scale".as_ref(),
        "--types-profile".as_ref(),
        profile.as_os_str(),
    ]);
    assert!(o.status.success(), "{}", text(&o));
    (dirs.remove(0), dirs.remove(0), text(&o))
}

fn run(dir: &Path) -> String {
    let o = Command::new(python()).args(["-I", "-c", CALLS]).arg(dir).output().unwrap();
    assert!(o.status.success(), "{}", text(&o));
    text(&o)
}

#[test]
fn a_specialized_body_and_its_guard_failures_behave_as_cpython() {
    let scratch = ScratchDir::new("select_types").unwrap();
    let (built, plain, report) = built(&scratch);
    // `count` is specialized for its two `str`s; `scale`'s list is no type
    // a native body takes, its float is.
    assert!(report.contains("compiled: pkg.m:count"), "{report}");
    assert!(report.contains("specialized: `count` runs natively when called with s: str, c: str"), "{report}");
    assert!(report.contains("specialized: `scale` runs natively when called with k: float"), "{report}");
    let out = run(&built);
    assert_eq!(out, run(&plain));
    // Both paths ran: the guard's (the observed `str`s), and the generic
    // body's (a subclass, another type, an exception).
    assert!(out.contains("count 3\ncount 0\ncount 2\ncount 1\ncount 3\ncount 0\ncount 2\ncount 3\n"), "{out}");
    assert!(out.contains("count TypeError object of type 'int' has no len()"), "{out}");
    // The module is the one the package's extension serves.
    let o = Command::new(python())
        .args(["-I", "-c", "import sys; sys.path.insert(0, sys.argv[1]); from pkg import m; print(type(m.__spec__.loader).__name__)"])
        .arg(&built)
        .output()
        .unwrap();
    assert_eq!(text(&o), "_PyccFinder\n");
}
