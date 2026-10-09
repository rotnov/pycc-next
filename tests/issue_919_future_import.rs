// Issue #919 (D-229): `from __future__ import <feature>` is a compiler
// directive, not a module. The nine no-op features CPython 3.14 accepts
// (`annotations` and the eight mandatory ones) lower to nothing, end to end
// through the real `pycc check` / `pycc build` CLI, and the driver never
// resolves `__future__` against the project, so a sibling `__future__.py`
// is never loaded.
//
// `tests/diagnostics/{l0001,c0001}_future_*` own the rejected shapes.
// Every expected stdout below was verified against CPython 3.14 on the same
// source.

use pycc_scratch::ScratchDir;
use std::path::{Path, PathBuf};
use std::process::Command;

fn pycc_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_pycc"))
}

fn write(root: &Path, relative: &str, contents: &str) -> PathBuf {
    let path = root.join(relative);
    std::fs::write(&path, contents).unwrap();
    path
}

fn check(entry: &Path) -> std::process::Output {
    Command::new(pycc_bin())
        .args(["check", entry.to_str().unwrap()])
        .output()
        .unwrap()
}

fn build_and_run(dir: &ScratchDir, entry: &Path, label: &str) -> std::process::Output {
    let out = dir.join(label);
    let status = Command::new(pycc_bin())
        .args([
            "build",
            entry.to_str().unwrap(),
            "-o",
            out.to_str().unwrap(),
        ])
        .status()
        .unwrap();
    assert!(status.success(), "`pycc build` failed for {label}");
    Command::new(&out).output().unwrap()
}

/// The issue's own reproduction, plus a call so the binary prints.
const REPRO: &str = "\
from __future__ import annotations

def f(x: int) -> int:
    return x

print(f(21))
";

#[test]
fn the_reproduction_checks_clean_and_runs() {
    let dir = ScratchDir::new("issue_919_repro").unwrap();
    let entry = write(&dir, "basic.py", REPRO);
    let checked = check(&entry);
    assert_eq!(checked.status.code(), Some(0));
    assert!(
        checked.stdout.is_empty(),
        "{}",
        String::from_utf8_lossy(&checked.stdout)
    );
    let output = build_and_run(&dir, &entry, "basic");
    assert!(output.status.success());
    assert_eq!(output.stdout, b"21\n");
}

#[test]
fn a_multi_name_future_import_after_a_docstring_runs() {
    let dir = ScratchDir::new("issue_919_multi").unwrap();
    let entry = write(
        &dir,
        "multi.py",
        "\
\"\"\"Ported from Python 2: both directives are no-ops on 3.x.\"\"\"
from __future__ import annotations, division
from __future__ import print_function

print(7 / 2)
",
    );
    assert_eq!(check(&entry).status.code(), Some(0));
    let output = build_and_run(&dir, &entry, "multi");
    assert!(output.status.success());
    assert_eq!(output.stdout, b"3.5\n");
}

/// A project directory with its own `__future__.py`: before D-229 the
/// driver resolved `from __future__ import annotations` to that file and
/// bound its names (CPython only reaches the file at run time, after the
/// directive). The sibling contains an import pycc rejects, so the entry
/// compiles and runs only if the file is never loaded.
#[test]
fn a_sibling_dunder_future_module_is_never_loaded() {
    let dir = ScratchDir::new("issue_919_sibling").unwrap();
    write(&dir, "__future__.py", "import os\nannotations = 3\n");
    let entry = write(
        &dir,
        "main.py",
        "from __future__ import annotations\nprint(1)\n",
    );
    let checked = check(&entry);
    assert_eq!(
        checked.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&checked.stdout)
    );
    let output = build_and_run(&dir, &entry, "main");
    assert!(output.status.success());
    assert_eq!(output.stdout, b"1\n");
}
