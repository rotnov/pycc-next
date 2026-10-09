//! Issue #1482: a module's own foreign import admits object receivers
//! (#1095) for its whole body, so a function written above `import gc` that
//! calls `gc.garbage.append(1)`, or `append` on a name bound to a CPython
//! object below the function, takes the foreign method call as CPython does
//! when the function runs. Before #1482 the admission started at the import
//! statement and such a call was refused with `I0404`. An import in an
//! `if TYPE_CHECKING:` body never runs and never admits.
//!
//! The `pycc check` results run everywhere. The embedded-executable
//! comparisons with CPython 3.14.7 -- the issue's two programs verbatim,
//! and one that observes both shapes through `print` -- are `#[ignore]`d
//! for the reason every embed test is.
//!
//! [#1482]: https://github.com/rotnov/pycc/issues/1482

use pycc_scratch::ScratchDir;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn pycc() -> Command {
    Command::new(env!("CARGO_BIN_EXE_pycc"))
}

fn stdout_of(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).replace("\r\n", "\n")
}

fn stderr_of(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).replace("\r\n", "\n")
}

fn rendered(output: &Output) -> String {
    let mut text = stdout_of(output);
    text.push_str(&stderr_of(output));
    text
}

fn write(dir: &Path, file: &str, body: &str) -> PathBuf {
    let path = dir.join(file);
    std::fs::write(&path, body).expect("write the fixture source");
    path
}

/// Runs `pycc check` on `entry`, written as `m.py` in a fresh scratch
/// directory.
fn check(tag: &str, entry: &str) -> Output {
    let dir = ScratchDir::new(tag).expect("scratch");
    pycc()
        .arg("check")
        .arg(write(&dir, "m.py", entry))
        .output()
        .expect("pycc should spawn")
}

fn assert_passes(tag: &str, entry: &str) {
    let output = check(tag, entry);
    let text = rendered(&output);
    assert_eq!(output.status.code(), Some(0), "{entry}: {text}");
    assert!(!text.contains("error["), "{entry}: {text}");
}

/// The embedded program: both of the issue's shapes, observed through
/// `print`.
const EMBEDDED: &str = "def f() -> None:\n    gc.garbage.append(1)\n    \
                        print(len(gc.garbage))\n    print(gc.garbage.pop())\n\n\n\
                        def h() -> None:\n    g.append(2)\n    print(g.pop())\n\n\n\
                        import gc\nimport collections\n\ng = collections.deque()\nf()\nh()\n";

/// The issue's first program, verbatim: an attribute of the imported
/// module. CPython runs it silently and exits 0.
const ISSUE_ATTR: &str = "def f() -> None:\n    gc.garbage.append(1)\n\n\nimport gc\n\nf()\n";

/// The issue's second program, verbatim: a name bound to a CPython object
/// below the function. CPython raises `AttributeError` from `g.append`, a
/// module having no `append`.
const ISSUE_NAME: &str = "def f() -> None:\n    g.append(1)\n\n\nimport gc\n\ng = gc\nf()\n";

/// The issue's two examples: an attribute of the imported module, and a
/// name bound to a CPython object below the function.
#[test]
fn a_call_above_the_module_s_own_foreign_import_passes_check() {
    for (tag, entry) in [
        ("obj_1482_attr", ISSUE_ATTR),
        ("obj_1482_name", ISSUE_NAME),
        (
            "obj_1482_block",
            "def f() -> None:\n    gc.garbage.append(1)\n\n\ntry:\n    import gc\n\
             except ImportError:\n    raise\n\nf()\n",
        ),
        (
            "obj_1482_block_from",
            "def f() -> None:\n    garbage.append(1)\n\n\ntry:\n    from gc import garbage\n\
             except ImportError:\n    raise\n\nf()\n",
        ),
    ] {
        assert_passes(tag, entry);
    }
}

/// The embedded program itself passes `pycc check`, so its shape is pinned
/// where the build-and-run comparison does not run.
#[test]
fn the_embedded_program_passes_check() {
    assert_passes("obj_1482_embedded_check", EMBEDDED);
}

/// A native list above the import keeps its container diagnostic: the
/// dispatched call's container reading still reports it.
#[test]
fn a_native_list_above_the_import_keeps_its_diagnostic() {
    let output = check("obj_1482_list", "xs = [1]\nxs.append(\"s\")\nimport gc\n");
    let text = rendered(&output);
    assert_eq!(output.status.code(), Some(1), "{text}");
    assert_eq!(text.matches("error[").count(), 1, "{text}");
    assert!(text.contains("error[T0021]"), "{text}");
    assert!(
        text.contains("cannot append `str` to a list of `int`"),
        "{text}"
    );
}

/// An import only in an `if TYPE_CHECKING:` body leaves a native module
/// unchanged.
#[test]
fn a_type_checking_only_import_leaves_the_module_native() {
    assert_passes(
        "obj_1482_type_checking",
        "from typing import TYPE_CHECKING\nif TYPE_CHECKING:\n    import gc\n\nxs = [1]\n\
         xs.append(2)\n",
    );
}

/// Builds `source`, written as `m.py` in `dir`, into the embedded
/// executable `dir/app`, and returns the interpreter its bundle names.
#[cfg(not(windows))]
fn build_embedded(dir: &Path, source: &str) -> PathBuf {
    let output = pycc()
        .arg("build")
        .arg(write(dir, "m.py", source))
        .arg("-o")
        .arg(dir.join("app"))
        .output()
        .expect("pycc should spawn");
    assert!(output.status.success(), "{}", rendered(&output));
    let marker = std::fs::read_to_string(dir.join("app.pycc").join("PYCC-BUNDLE"))
        .expect("an embedded build writes its marker");
    let mut lines = marker.lines();
    assert_eq!(lines.next(), Some("pycc-bundle 1"));
    assert_eq!(lines.next(), Some("python 3.14.7"), "{marker}");
    let executable = lines
        .next()
        .and_then(|line| line.strip_prefix("executable "))
        .expect("the marker names its interpreter");
    PathBuf::from(executable)
}

#[cfg(not(windows))]
#[test]
#[ignore = "needs CPython 3.14.7 as python3.14 or PYCC_PYTHON; run with --include-ignored"]
fn calls_above_the_import_match_cpython_3_14_7_in_an_embedded_executable() {
    let (embedded, oracle) = run_both("embed_1482_own_import", EMBEDDED);
    assert_eq!(embedded.status.code(), Some(0), "{}", stderr_of(&embedded));
    assert_eq!(stdout_of(&oracle), "1\n1\n2\n");
    assert_eq!(embedded.stdout, oracle.stdout);
}

/// Builds `source` into an embedded executable in a fresh scratch directory
/// named by `tag`, and runs it and CPython (the interpreter the bundle
/// names) on the same file.
#[cfg(not(windows))]
fn run_both(tag: &str, source: &str) -> (Output, Output) {
    let dir = ScratchDir::new(tag).expect("scratch");
    let python = build_embedded(&dir, source);
    let embedded = Command::new(dir.join("app"))
        .output()
        .expect("the embedded binary runs");
    let oracle = Command::new(python)
        .arg(dir.join("m.py"))
        .output()
        .expect("CPython runs the oracle program");
    (embedded, oracle)
}

/// The last line of `output`'s stderr: the exception line of an uncaught
/// traceback, compared the way `tests/issue_1371_object_raise.rs` does.
#[cfg(not(windows))]
fn last_stderr_line(output: &Output) -> Option<String> {
    stderr_of(output)
        .trim_end()
        .lines()
        .last()
        .map(str::to_owned)
}

/// The issue's two programs, verbatim, behave as under CPython: the
/// attribute call runs silently, and the bare-name call reaches the module
/// object and escapes as CPython's `AttributeError`, with the same exit
/// status and the same last traceback line.
#[cfg(not(windows))]
#[test]
#[ignore = "needs CPython 3.14.7 as python3.14 or PYCC_PYTHON; run with --include-ignored"]
fn the_issue_s_programs_match_cpython_3_14_7_in_an_embedded_executable() {
    let (embedded, oracle) = run_both("embed_1482_issue_attr", ISSUE_ATTR);
    assert_eq!(embedded.status.code(), Some(0), "{}", stderr_of(&embedded));
    assert_eq!(oracle.status.code(), Some(0), "{}", stderr_of(&oracle));
    assert_eq!(stdout_of(&embedded), "");
    assert_eq!(stdout_of(&embedded), stdout_of(&oracle));

    let (embedded, oracle) = run_both("embed_1482_issue_name", ISSUE_NAME);
    assert_eq!(stdout_of(&embedded), "");
    assert_eq!(stdout_of(&embedded), stdout_of(&oracle));
    assert_eq!(oracle.status.code(), Some(1), "{}", stderr_of(&oracle));
    assert_eq!(embedded.status.code(), oracle.status.code());
    assert_eq!(
        last_stderr_line(&embedded),
        Some("AttributeError: module 'gc' has no attribute 'append'".to_owned())
    );
    assert_eq!(last_stderr_line(&embedded), last_stderr_line(&oracle));
}
