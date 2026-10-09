//! #941 (PEP 435): an ordinary class that names an enum class as a base
//! (`class Foo(Color): pass`) is rejected with `C0001` by
//! `pycc_hir::class::mro::validate_bases`. Before this fix the class
//! statement type-checked, D-225's `ensure_init` synthesized an empty
//! constructor for the subclass, and instantiating it aborted the compiled
//! program with `pycc_rt: invalid encoded int word 0x0` because the `value`
//! and `name` slots inherited from the enum's `attrs` were never filled.
//!
//! The unit tests beside the check prove both wordings and the span; these
//! prove the binary reports the diagnostic on the issue's own program and its
//! sibling shapes, including an enum imported from another project module.

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

/// Runs `pycc <subcommand>` on `entry`, asserting the compiler exits with
/// status 1 (a diagnostic, not the 101 a panic exits with), reports the
/// enum-subclass `C0001` carrying `message`, renders the span on the class
/// header line `header` (line `line` of the entry), and never prints a
/// panic or internal-error line. `pycc check` renders its diagnostic on
/// stdout and `pycc build` on stderr, so both streams are searched together.
fn assert_enum_subclass_rejected(
    dir: &ScratchDir,
    subcommand: &str,
    entry: &Path,
    line: u32,
    header: &str,
    message: &str,
) {
    let out = dir.join("prog");
    let mut cmd = Command::new(pycc_bin());
    cmd.arg(subcommand).arg(entry.to_str().unwrap());
    if subcommand == "build" {
        cmd.args(["-o", out.to_str().unwrap()]);
    }
    let result = cmd.output().unwrap();
    let stderr = String::from_utf8_lossy(&result.stderr);
    let stdout = String::from_utf8_lossy(&result.stdout);
    let combined = format!("{stdout}{stderr}");
    assert_eq!(
        result.status.code(),
        Some(1),
        "pycc {subcommand} should fail with a diagnostic, got {:?}\nstdout: {stdout}\nstderr: {stderr}",
        result.status
    );
    assert!(
        combined.contains("error[C0001]"),
        "the output should carry a C0001 diagnostic, got: {combined}"
    );
    assert!(
        combined.contains(message),
        "the output should carry {message:?}, got: {combined}"
    );
    // The renderer prints paths with forward slashes on every platform
    // (see `tests/slice0.rs`), so normalize the expected location the same way.
    let location = format!("{}:{line}:1", entry.to_string_lossy().replace('\\', "/"));
    assert!(
        combined.contains(&location),
        "the diagnostic should point at the class header {location}, got: {combined}"
    );
    assert!(
        combined.contains(&format!("{line} | {header}")),
        "the rendered source line should be the class header {header:?}, got: {combined}"
    );
    for forbidden in ["panicked", "internal error", "pycc_rt:"] {
        assert!(
            !combined.contains(forbidden),
            "the compiler must diagnose, not abort; found {forbidden:?} in: {combined}"
        );
    }
    assert!(!out.exists(), "a failing build must not leave a binary");
}

const ISSUE_PROGRAM: &str = "from enum import Enum\n\n\nclass Color(Enum):\n    RED = 1\n\n\nclass Foo(Color):\n    pass\n\n\ndef main() -> None:\n    f = Foo()\n    print(f.value)\n\n\nmain()\n";

const WITH_MEMBERS_MESSAGE: &str = "class `Foo` cannot inherit from enum class `Color` -- CPython raises `TypeError: <enum 'Foo'> cannot extend <enum 'Color'>` because an enum class with members cannot be extended";

/// The issue's own reproduction: `pycc check` used to exit 0 here.
#[test]
fn the_issue_program_is_c0001_on_the_class_header() {
    let dir = ScratchDir::new("941_check").expect("failed to create scratch dir");
    let entry = write(&dir, "prog.py", ISSUE_PROGRAM);
    assert_enum_subclass_rejected(
        &dir,
        "check",
        &entry,
        8,
        "class Foo(Color):",
        WITH_MEMBERS_MESSAGE,
    );
}

/// `pycc build` stops at HIR lowering too, so the runtime abort the issue
/// reports is unreachable and no artifact is produced.
#[test]
fn building_the_issue_program_is_c0001_and_leaves_no_binary() {
    let dir = ScratchDir::new("941_build").expect("failed to create scratch dir");
    let entry = write(&dir, "prog.py", ISSUE_PROGRAM);
    assert_enum_subclass_rejected(
        &dir,
        "build",
        &entry,
        8,
        "class Foo(Color):",
        WITH_MEMBERS_MESSAGE,
    );
}

/// The `StrEnum` spelling (#892) shares `lower_enum_class` and therefore the
/// `is_enum` marker the check keys on.
#[test]
fn subclassing_a_str_enum_is_c0001() {
    let dir = ScratchDir::new("941_str_enum").expect("failed to create scratch dir");
    let entry = write(
        &dir,
        "prog.py",
        "from enum import StrEnum\n\n\nclass Color(StrEnum):\n    RED = \"red\"\n\n\nclass Foo(Color):\n    pass\n\n\nf = Foo()\nprint(f.value)\n",
    );
    assert_enum_subclass_rejected(
        &dir,
        "check",
        &entry,
        8,
        "class Foo(Color):",
        WITH_MEMBERS_MESSAGE,
    );
}

/// A grand-subclass (`Bar(Foo)` where `Foo(Color)`) is stopped at the first
/// subclass, which is the one that names the enum.
#[test]
fn a_grand_subclass_of_an_enum_is_c0001_at_the_first_subclass() {
    let dir = ScratchDir::new("941_grand").expect("failed to create scratch dir");
    let entry = write(
        &dir,
        "prog.py",
        "from enum import Enum\n\n\nclass Color(Enum):\n    RED = 1\n\n\nclass Foo(Color):\n    pass\n\n\nclass Bar(Foo):\n    pass\n\n\nb = Bar()\nprint(b.value)\n",
    );
    assert_enum_subclass_rejected(
        &dir,
        "check",
        &entry,
        8,
        "class Foo(Color):",
        WITH_MEMBERS_MESSAGE,
    );
}

/// A docstring-only enum (#744) has an empty member table. CPython allows
/// extending it, so the wording is "not supported yet" rather than CPython's
/// own `TypeError`.
#[test]
fn subclassing_a_member_less_enum_is_c0001_not_supported_yet() {
    let dir = ScratchDir::new("941_member_less").expect("failed to create scratch dir");
    let entry = write(
        &dir,
        "prog.py",
        "from enum import Enum\n\n\nclass Base(Enum):\n    \"A base.\"\n\n\nclass Color(Base):\n    RED = 1\n\n\nprint(Color.RED.value)\n",
    );
    assert_enum_subclass_rejected(
        &dir,
        "check",
        &entry,
        8,
        "class Color(Base):",
        "class `Color` cannot inherit from member-less enum class `Base` -- extending an enum class that has no members is not supported yet",
    );
}

/// An enum imported from another project module (#881, D-222) carries its
/// `is_enum` marker across the import, so the subclass in the entry file is
/// rejected against the entry file's own class header.
#[test]
fn subclassing_an_enum_imported_from_another_module_is_c0001() {
    let dir = ScratchDir::new("941_import").expect("failed to create scratch dir");
    write(
        &dir,
        "colors.py",
        "from enum import Enum\n\n\nclass Color(Enum):\n    RED = 1\n",
    );
    let entry = write(
        &dir,
        "main.py",
        "from colors import Color\n\n\nclass Foo(Color):\n    pass\n\n\nf = Foo()\nprint(f.value)\n",
    );
    assert_enum_subclass_rejected(
        &dir,
        "check",
        &entry,
        4,
        "class Foo(Color):",
        WITH_MEMBERS_MESSAGE,
    );
}
