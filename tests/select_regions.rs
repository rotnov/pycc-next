//! Native calls inside a region of selected functions
//! (docs/design/selected-regions.md, R2): a region's members get native
//! copies that call each other directly while the region is intact, and
//! fall back to the generic bodies once anything they rely on changes -- a
//! member global rebound, a member's `__code__` or `__defaults__` replaced,
//! a dispatch target rebound, the pattern global replaced. Every path must
//! behave as CPython: the same output and the same exceptions, with the
//! same traceback frames, and `RecursionError` on a deep cycle of copies.

use pycc_scratch::ScratchDir;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const MODULE: &str = include_str!("fixtures/select_regions.py");
const CALLS: &str = include_str!("fixtures/select_regions_calls.py");

/// The selected functions: `word` and `esc_b` are left out, and join the
/// region as its position-flow callees.
const SELECTED: [&str; 10] = [
    "skip", "number", "value", "items", "parse", "escapes", "esc_a", "tails", "size", "sizes",
];

fn python() -> std::ffi::OsString {
    std::env::var_os("PYCC_PYTHON").unwrap_or_else(|| "python3".into())
}

fn text(o: &Output) -> String {
    // Line endings as on POSIX: Windows' Python writes `\r\n`.
    format!("{}{}", String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr)).replace('\r', "")
}

/// The fixture as `m.py` in `built` (compiled with `SELECTED`) and in
/// `plain` (left alone); returns both directories and the build's report.
fn build(scratch: &Path) -> (PathBuf, PathBuf, String) {
    let built = scratch.join("built");
    let plain = scratch.join("plain");
    for dir in [&built, &plain] {
        std::fs::create_dir_all(dir).unwrap();
        std::fs::write(dir.join("m.py"), MODULE).unwrap();
    }
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_pycc"));
    cmd.arg("build")
        .arg(built.join("m.py"))
        .arg("-o")
        .arg(built.join("m"))
        .args(["--ext", "--non-strict", "--report-fallbacks", "--select", "none"]);
    for name in SELECTED {
        cmd.args(["--compile", &format!("m:{name}")]);
    }
    let o = cmd.output().expect("pycc runs");
    assert!(o.status.success(), "{}", text(&o));
    (built, plain, String::from_utf8_lossy(&o.stderr).replace('\r', ""))
}

/// The calls of `scenario` against the module in `dir`.
fn run(scratch: &Path, dir: &Path, scenario: &str) -> String {
    let calls = scratch.join("calls.py");
    std::fs::write(&calls, CALLS).unwrap();
    let o = Command::new(python()).arg("-I").arg(&calls).arg(dir).arg(scenario).output().unwrap();
    assert!(o.status.success(), "{scenario}: {}", text(&o));
    text(&o)
}

#[test]
fn region_copies_behave_as_cpython_until_and_after_what_they_rely_on_changes() {
    let scratch = ScratchDir::new("select_regions").unwrap();
    let (built, plain, report) = build(&scratch);
    // The flow callees joined, reported apart from the selection, which
    // pass 3 checks a build against; every member that has something
    // native got a copy.
    for name in ["word", "esc_b"] {
        assert!(report.contains(&format!("compiled as a region's callee: m:{name}\n")), "{report}");
        assert!(!report.contains(&format!("compiled: m:{name}\n")), "{report}");
    }
    for name in ["skip", "word", "number", "esc_a", "esc_b", "escapes", "value", "items", "parse", "tails"] {
        assert!(report.contains(&format!("specialized: `region {name}` runs natively")), "{name}: {report}");
    }
    assert!(!report.contains("not specialized"), "{report}");
    for scenario in [
        "plain",
        "rebind",
        "code",
        "defaults",
        "dispatch",
        "pattern",
        "deep",
        "midrun-int",
        "midrun-pattern-int",
        "traceback",
    ] {
        let expected = run(&scratch, &plain, scenario);
        assert_eq!(run(&scratch, &built, scenario), expected, "{scenario}");
    }
    // docs/DEVIATIONS.md: a member rebound while a region call runs, whose
    // new result is no `int` where the copy needs one, raises `TypeError`
    // for the rest of that call only.
    let expected = run(&scratch, &plain, "midrun-index");
    assert_eq!(expected, "midrun ['ab', 'cd', 12]\nafter ['ab', 'cd', 12]\n");
    assert_eq!(
        run(&scratch, &built, "midrun-index"),
        "midrun TypeError skip() returned Index, the compiled caller needs int\nafter ['ab', 'cd', 12]\n"
    );
    // The same for the pattern global replaced while a call runs: the
    // caller of the member whose result came from the new pattern raises.
    let expected = run(&scratch, &plain, "midrun-pattern-index");
    assert_eq!(expected, "midrun ['custom', 'custom', 12]\nafter ['custom', 'custom', 12]\n");
    assert_eq!(
        run(&scratch, &built, "midrun-pattern-index"),
        "midrun TypeError value() returned a tuple whose item 0 is Index, the compiled caller needs int\n\
         after ['custom', 'custom', 12]\n"
    );
    // docs/DEVIATIONS.md: a position a copy hands back unchanged is a new
    // `int` object outside the small-int cache, the same one inside it.
    assert_eq!(run(&scratch, &plain, "identity"), "identity True True True\n");
    assert_eq!(run(&scratch, &built, "identity"), "identity True False True\n");
}

#[test]
fn the_scenarios_reach_what_they_test() {
    // CPython's own answers, so a scenario that stops exercising its case
    // fails here rather than passing on both sides.
    let scratch = ScratchDir::new("select_regions").unwrap();
    let plain = scratch.join("plain");
    std::fs::create_dir_all(&plain).unwrap();
    std::fs::write(plain.join("m.py"), MODULE).unwrap();
    let out = run(&scratch, &plain, "plain");
    assert!(out.contains("parse ['ab', [1, 'cd', [22, 'ef']], 'AAA', ['B'], 7]\n"), "{out}");
    // `pos` is read by the lambda after `pos += 1`.
    assert!(out.contains("tails [2, 'xyz', '']\n"), "{out}");
    assert!(out.contains("skip-big 1180591620717411303424\n"), "{out}");
    // A parameter named `len` is called, not the builtin.
    assert!(out.contains("size 'x'\nsizes 3\n"), "{out}");
    let out = run(&scratch, &plain, "rebind");
    assert!(out.contains("parse []\n"), "{out}");
    let out = run(&scratch, &plain, "dispatch");
    assert!(out.contains("parse ['ab', [1, 'cd', [22, 'ef']], 'ZZZ', ['ZZ'], 7]\n"), "{out}");
    let out = run(&scratch, &plain, "pattern");
    assert!(out.contains("parse ['custom', [1, 'custom', [22, 'custom']], 'AAA', ['B'], 7]\n"), "{out}");
    let out = run(&scratch, &plain, "deep");
    assert!(out.contains("deep RecursionError"), "{out}");
    // An exception escaping a region entry's copy, and one escaping
    // through a chain of copies: one frame per Python call, none for an
    // entry guard.
    let out = run(&scratch, &plain, "traceback");
    assert!(out.contains("entry IndexError [('frames', 72), ('items', 74)]\n"), "{out}");
    assert!(
        out.contains("chain ValueError [('frames', 72), ('parse', 88), ('value', 62), ('items', 76), ('value', 67), ('word', 29)]\n"),
        "{out}"
    );
    let out = run(&scratch, &plain, "midrun-pattern-int");
    assert!(out.contains("midrun ['custom', 'custom', 12]\n"), "{out}");
}
