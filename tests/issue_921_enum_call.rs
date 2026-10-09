//! #921 (PEP 435): calling an enum class (`Color()`, `Color(1)`) is rejected
//! with `C0001` and never reaches the internal-error panic
//! `pycc_types::class::binding::resolve_instantiation`'s `__init__` MRO walk
//! keeps for a genuinely inconsistent class table. Before #921 both shapes
//! aborted the compiler with `internal error: no `__init__` found in class
//! `Color`'s MRO`; before #944 the `C0001` rendered at `1:1`, because
//! `pycc_types` emits it with a zero span. Since #944 the reporting site is
//! `pycc_hir`'s per-item AST scan (`class::enum_call`, D-233), which points
//! at the call expression itself; `resolve_instantiation` keeps its
//! span-less guard behind it.
//!
//! The unit tests beside the scan and the guard prove they fire; these
//! prove the binary reports the `C0001` at the call's own position on the
//! issue's program and its sibling shapes, and never panics.

use pycc_scratch::ScratchDir;
use std::io::Write;
use std::process::Command;

fn pycc_bin() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_BIN_EXE_pycc"))
}

fn write_fixture(dir: &std::path::Path, name: &str, source: &str) -> std::path::PathBuf {
    let path = dir.join(name);
    let mut f = std::fs::File::create(&path).unwrap();
    f.write_all(source.as_bytes()).unwrap();
    path
}

/// The `line:column` (both 1-based) of the first `needle` in `source`,
/// computed from the inline source rather than guessed, so every call site
/// below pins the position of its own call expression.
fn position_of(source: &str, needle: &str) -> String {
    let start = source
        .find(needle)
        .unwrap_or_else(|| panic!("{needle:?} not in {source:?}"));
    let line = source[..start].matches('\n').count() + 1;
    let column = start - source[..start].rfind('\n').map_or(0, |nl| nl + 1) + 1;
    format!("{line}:{column}")
}

/// Runs `pycc <subcommand>` on `source` written as `prog.py`, asserting the
/// compiler exits with status 1 (a diagnostic, not the 101 a panic exits
/// with), reports the enum-call `C0001` naming `class_name` at the position
/// of `call_text` in `source` (#944), and never prints a panic or
/// internal-error line. `pycc check` renders its diagnostic on stdout and
/// `pycc build` on stderr, so both streams are searched together.
fn assert_enum_call_rejected(
    slug: &str,
    subcommand: &str,
    source: &str,
    class_name: &str,
    call_text: &str,
) {
    let dir = ScratchDir::new(slug).expect("failed to create scratch dir");
    let src = write_fixture(&dir, "prog.py", source);
    assert_enum_call_rejected_in(&dir, subcommand, &src, source, class_name, call_text);
}

/// The multi-file form of [`assert_enum_call_rejected`]: `entry` is the file
/// handed to the compiler, `entry_source` its text (for the call position).
fn assert_enum_call_rejected_in(
    dir: &ScratchDir,
    subcommand: &str,
    entry: &std::path::Path,
    entry_source: &str,
    class_name: &str,
    call_text: &str,
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
        combined.contains(&format!("cannot call enum class `{class_name}`")),
        "the output should name the enum class, got: {combined}"
    );
    let file_name = entry.file_name().unwrap().to_str().unwrap();
    let location = format!("{file_name}:{}", position_of(entry_source, call_text));
    assert!(
        combined.contains(&location),
        "the C0001 should point at the call expression ({location}), got: {combined}"
    );
    for forbidden in ["panicked", "internal error"] {
        assert!(
            !combined.contains(forbidden),
            "the compiler must diagnose, not abort; found {forbidden:?} in: {combined}"
        );
    }
    assert!(!out.exists(), "a failing build must not leave a binary");
}

/// The issue's own reproduction: a zero-argument call inside a function.
#[test]
fn a_zero_argument_enum_call_in_a_function_is_c0001_not_a_panic() {
    assert_enum_call_rejected(
        "921_no_args",
        "check",
        "from enum import Enum\n\n\nclass Color(Enum):\n    RED = 1\n    GREEN = 2\n\n\ndef main() -> None:\n    c = Color()\n    print(c.value)\n\n\nmain()\n",
        "Color",
        "Color()",
    );
}

/// CPython's by-value member lookup, at module scope.
#[test]
fn a_by_value_enum_call_at_module_scope_is_c0001_not_a_panic() {
    assert_enum_call_rejected(
        "921_by_value",
        "check",
        "from enum import Enum\n\n\nclass Color(Enum):\n    RED = 1\n    GREEN = 2\n\n\nc = Color(1)\nprint(c.value)\n",
        "Color",
        "Color(1)",
    );
}

/// `raise Color()` is a call expression like any other to the scan, so it
/// is reported at the call; on the guard path it took before #944 it was the
/// ordinary-call inference (an enum is not an exception class).
#[test]
fn raising_an_enum_call_is_c0001_not_a_panic() {
    assert_enum_call_rejected(
        "921_raise",
        "check",
        "from enum import Enum\n\n\nclass Color(Enum):\n    RED = 1\n\n\nraise Color()\n",
        "Color",
        "Color()",
    );
}

/// A docstring-only enum (#744) has an empty member table; the guard keys
/// on `HirClassDef::is_enum`, so it is rejected all the same.
#[test]
fn calling_a_member_less_enum_is_c0001_not_a_panic() {
    assert_enum_call_rejected(
        "921_member_less",
        "check",
        "from enum import Enum\n\n\nclass E(Enum):\n    \"doc\"\n\n\ne = E()\nprint(1)\n",
        "E",
        "E()",
    );
}

/// The `StrEnum` spelling (#892) shares `lower_enum_class`, so it carries
/// the same marker and the same rejection.
#[test]
fn calling_a_str_enum_class_is_c0001_not_a_panic() {
    assert_enum_call_rejected(
        "921_str_enum",
        "check",
        "from enum import StrEnum\n\n\nclass S(StrEnum):\n    A = \"a\"\n\n\ns = S(\"a\")\nprint(s.value)\n",
        "S",
        "S(\"a\")",
    );
}

/// `pycc build` stops at HIR lowering too (the scan rejects the module before
/// the type checker runs), so `pycc_mir`'s own `Instantiate` MRO-walk panic
/// is unreachable for an enum class; the span is the call's on this path as
/// well.
#[test]
fn building_an_enum_call_is_c0001_and_never_reaches_mir() {
    assert_enum_call_rejected(
        "921_build",
        "build",
        "from enum import Enum\n\n\nclass Color(Enum):\n    RED = 1\n    GREEN = 2\n\n\ndef main() -> None:\n    c = Color()\n    print(c.value)\n\n\nmain()\n",
        "Color",
        "Color()",
    );
}

/// #944: an enum pulled in by a project import (#898) is known to the scan
/// through `HirClassDef::is_enum` rather than the syntactic pre-collection
/// (the class header is in another file), so this is the end-to-end proof of
/// that half of the name set. The enum is docstring-only (#744) so
/// `enum_members` emptiness cannot be what makes it known.
#[test]
fn calling_an_imported_docstring_only_enum_is_c0001_at_the_call() {
    let dir = ScratchDir::new("921_import").expect("failed to create scratch dir");
    write_fixture(
        &dir,
        "colors.py",
        "from enum import Enum\n\n\nclass Color(Enum):\n    \"doc\"\n",
    );
    let entry_source = "from colors import Color\n\n\nColor()\n";
    let entry = write_fixture(&dir, "prog.py", entry_source);
    assert_enum_call_rejected_in(&dir, "check", &entry, entry_source, "Color", "Color()");
}

/// PR #971 review: a module-level `def Color()` beside `class Color(Enum)`
/// is a name collision the class item reports; a `Color()` between the two
/// resolves to the function, so the binary must report the collision alone
/// and never a false-kind "cannot call enum class" ahead of it.
#[test]
fn a_call_to_a_def_bound_name_before_the_enum_class_reports_the_collision_only() {
    let dir = ScratchDir::new("921_def_shadow").expect("failed to create scratch dir");
    let entry = write_fixture(
        &dir,
        "prog.py",
        "from enum import Enum\n\n\ndef Color() -> int:\n    return 1\n\n\nColor()\n\n\nclass Color(Enum):\n    RED = 1\n",
    );
    let result = Command::new(pycc_bin())
        .arg("check")
        .arg(entry.to_str().unwrap())
        .output()
        .unwrap();
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(result.status.code(), Some(1), "got: {combined}");
    assert!(
        combined.contains("class `Color` collides with a function of the same name"),
        "the collision diagnostic should be reported, got: {combined}"
    );
    assert!(
        !combined.contains("cannot call enum class"),
        "no enum-call C0001 may precede the collision, got: {combined}"
    );
}

/// PR #971 review, second round: the same rule for a project import --
/// `from colors import Color` beside `class Color(Enum)` collides at the
/// class item, and a `Color()` between the two resolves to the import.
#[test]
fn a_call_to_an_import_bound_name_before_the_enum_class_reports_the_collision_only() {
    let dir = ScratchDir::new("921_import_shadow").expect("failed to create scratch dir");
    write_fixture(&dir, "colors.py", "def Color() -> int:\n    return 1\n");
    let entry = write_fixture(
        &dir,
        "prog.py",
        "from enum import Enum\nfrom colors import Color\n\n\nColor()\n\n\nclass Color(Enum):\n    RED = 1\n",
    );
    let result = Command::new(pycc_bin())
        .arg("check")
        .arg(entry.to_str().unwrap())
        .output()
        .unwrap();
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(result.status.code(), Some(1), "got: {combined}");
    assert!(
        combined.contains("class `Color` collides with an import of the same name"),
        "the collision diagnostic should be reported, got: {combined}"
    );
    assert!(
        !combined.contains("cannot call enum class"),
        "no enum-call C0001 may precede the collision, got: {combined}"
    );
}

#[test]
fn a_call_after_an_identical_repeated_import_is_reported_at_the_call() {
    let dir = ScratchDir::new("921_repeated_import").expect("failed to create scratch dir");
    write_fixture(
        &dir,
        "colors.py",
        "from enum import Enum\n\n\nclass Color(Enum):\n    RED = 1\n",
    );
    let entry = write_fixture(
        &dir,
        "prog.py",
        "from colors import Color\nfrom colors import Color\n\n\nColor()\n",
    );
    let result = Command::new(pycc_bin())
        .arg("check")
        .arg(entry.to_str().unwrap())
        .output()
        .unwrap();
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(result.status.code(), Some(1), "got: {combined}");
    assert!(
        combined.contains("cannot call enum class `Color`") && combined.contains("prog.py:5:1"),
        "the repeated import binds the same class twice and the call keeps its span, got: {combined}"
    );
}

/// D-233 decision 5's documented residual: one enum imported under one
/// local name through two module paths (`colors.Color` directly and via
/// a `palette` re-export) is indistinguishable from a rebinding, so the
/// scan stays silent and the span-less guard reports the call at `1:1`.
#[test]
fn a_call_after_a_two_path_import_of_one_enum_falls_back_to_the_span_less_guard() {
    let dir = ScratchDir::new("921_two_path_import").expect("failed to create scratch dir");
    write_fixture(
        &dir,
        "colors.py",
        "from enum import Enum\n\n\nclass Color(Enum):\n    RED = 1\n",
    );
    write_fixture(&dir, "palette.py", "from colors import Color\n");
    let entry = write_fixture(
        &dir,
        "prog.py",
        "from colors import Color\nfrom palette import Color\n\n\nColor()\n",
    );
    let result = Command::new(pycc_bin())
        .arg("check")
        .arg(entry.to_str().unwrap())
        .output()
        .unwrap();
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(result.status.code(), Some(1), "got: {combined}");
    assert!(
        combined.contains("cannot call enum class `Color`") && combined.contains("prog.py:1:1"),
        "the two-path import is the documented span-less residual, got: {combined}"
    );
    assert!(
        !combined.contains("prog.py:5:1"),
        "the scan must not claim the call, got: {combined}"
    );
}

/// PR #971 review: a `from __future__` import binds nothing, so an enum
/// that shares a feature name is scanned and reported at the call.
#[test]
fn a_call_after_a_value_less_module_annotation_is_reported_at_the_call() {
    // `Color: int` at module level binds nothing at runtime, so the scan
    // still attributes the call (PR #971 review).
    let source = "from enum import Enum\nclass Color(Enum):\n    RED = 1\nColor: int\nColor()\n";
    assert_enum_call_rejected(
        "921_value_less_annotation",
        "check",
        source,
        "Color",
        "Color()",
    );
}

#[test]
fn an_enum_sharing_a_future_feature_name_is_reported_at_the_call() {
    let source = "from __future__ import annotations\nfrom enum import Enum\n\n\nclass annotations(Enum):\n    A = 1\n\n\ndef f() -> None:\n    annotations()\n";
    assert_enum_call_rejected(
        "921_future_name",
        "check",
        source,
        "annotations",
        "annotations()",
    );
}
