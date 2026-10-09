//! #892: string-valued `Enum` members, `enum.StrEnum`, and `enum.auto()`,
//! exercised through the public `pycc build` CLI boundary.
//!
//! The HIR-level validation paths have their own unit tests in
//! `crates/pycc_hir/src/class/enum_class.rs`; these tests prove the whole
//! pipeline (HIR -> types -> MIR -> codegen -> link -> run) agrees, which a
//! unit test on the lowering result alone cannot.

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

fn build_and_run(dir: &std::path::Path, src: &std::path::Path, bin_name: &str) -> (bool, Vec<u8>) {
    let out = dir.join(bin_name);
    let build_status = Command::new(pycc_bin())
        .args(["build", src.to_str().unwrap(), "-o", out.to_str().unwrap()])
        .status()
        .unwrap();
    if !build_status.success() {
        return (false, Vec::new());
    }
    let output = Command::new(&out).output().unwrap();
    (output.status.success(), output.stdout)
}

fn rejection_message(dir: &std::path::Path, src: &std::path::Path) -> String {
    let out = dir.join("should_not_compile");
    let output = Command::new(pycc_bin())
        .args(["build", src.to_str().unwrap(), "-o", out.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(!output.status.success(), "fixture should not compile");
    String::from_utf8_lossy(&output.stderr).into_owned()
}

/// #892: a plain `Enum` whose members are string literals.
#[test]
fn string_valued_enum_members_round_trip() {
    let dir = ScratchDir::new("892_str_members").expect("failed to create scratch dir");
    let src = write_fixture(
        &dir,
        "suit.py",
        "from enum import Enum\n\nclass Suit(Enum):\n    HEARTS = \"hearts\"\n    SPADES = \"spades\"\n\nprint(Suit.HEARTS.value)\nprint(Suit.SPADES.value)\nprint(Suit.HEARTS.name)\nfor s in Suit:\n    print(s.value)\n",
    );
    let (ok, stdout) = build_and_run(&dir, &src, "suit");
    assert!(ok, "a string-valued enum should build and run");
    assert_eq!(stdout, b"hearts\nspades\nHEARTS\nhearts\nspades\n");
}

/// #892: `enum.StrEnum` is a marker base like `Enum`, requiring `str` members.
#[test]
fn str_enum_subclass_round_trips() {
    let dir = ScratchDir::new("892_strenum").expect("failed to create scratch dir");
    let src = write_fixture(
        &dir,
        "kind.py",
        "from enum import StrEnum\n\nclass Kind(StrEnum):\n    AXIAL = \"axial\"\n    RADIAL = \"radial\"\n\nprint(Kind.AXIAL.value)\nprint(Kind.RADIAL.name)\n",
    );
    let (ok, stdout) = build_and_run(&dir, &src, "kind");
    assert!(ok, "a `StrEnum` subclass should build and run");
    assert_eq!(stdout, b"axial\nRADIAL\n");
}

/// #892: `auto()` in both enum flavors, verified against CPython 3.14.7.
#[test]
fn auto_member_values_round_trip() {
    let dir = ScratchDir::new("892_auto").expect("failed to create scratch dir");
    let src = write_fixture(
        &dir,
        "auto.py",
        "from enum import Enum, StrEnum, auto\n\nclass N(Enum):\n    A = auto()\n    B = 10\n    C = auto()\n\nclass S(StrEnum):\n    RED = auto()\n\nprint(N.A.value)\nprint(N.B.value)\nprint(N.C.value)\nprint(S.RED.value)\n",
    );
    let (ok, stdout) = build_and_run(&dir, &src, "auto");
    assert!(ok, "`auto()` members should build and run");
    assert_eq!(stdout, b"1\n10\n11\nred\n");
}

/// #892: every member of one enum must share a value type.
#[test]
fn a_mixed_value_type_enum_is_rejected_in_both_orderings() {
    let dir = ScratchDir::new("892_mixed").expect("failed to create scratch dir");
    let str_after_int = write_fixture(
        &dir,
        "str_after_int.py",
        "from enum import Enum\n\nclass E(Enum):\n    A = 1\n    B = \"b\"\n",
    );
    let message = rejection_message(&dir, &str_after_int);
    assert!(message.contains("C0001"), "{message}");
    assert!(
        message.contains("`str`-valued but enum class `E` has `int`-valued members"),
        "{message}"
    );

    let int_after_str = write_fixture(
        &dir,
        "int_after_str.py",
        "from enum import Enum\n\nclass E(Enum):\n    A = \"a\"\n    B = 2\n",
    );
    let message = rejection_message(&dir, &int_after_str);
    assert!(
        message.contains("`int`-valued but enum class `E` has `str`-valued members"),
        "{message}"
    );
}

/// #892: `StrEnum` fixes the value type before any member is read, so a
/// non-string first member is already an error (CPython 3.14.7 raises
/// `TypeError: 1 is not a string` for the same source).
#[test]
fn a_non_string_member_of_a_str_enum_is_rejected() {
    let dir = ScratchDir::new("892_strenum_int").expect("failed to create scratch dir");
    let src = write_fixture(
        &dir,
        "bad.py",
        "from enum import StrEnum\n\nclass K(StrEnum):\n    A = 1\n",
    );
    let message = rejection_message(&dir, &src);
    assert!(
        message.contains("derives from `StrEnum`, so member `A` must be assigned a string literal"),
        "{message}"
    );
}

/// #892: the widened catch-all still rejects every non-literal member value.
#[test]
fn a_non_literal_member_value_is_still_rejected() {
    let dir = ScratchDir::new("892_non_literal").expect("failed to create scratch dir");
    for (name, body) in [
        ("bool.py", "    A = True\n"),
        ("float.py", "    A = 1.5\n"),
        ("const.py", "    A = SOME_CONST\n"),
    ] {
        let src = write_fixture(
            &dir,
            name,
            &format!("from enum import Enum\n\nclass E(Enum):\n{body}"),
        );
        let message = rejection_message(&dir, &src);
        assert!(message.contains("C0001"), "{name}: {message}");
    }
}
