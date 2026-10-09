//! Issue #575 (Part 2 of #123): public-CLI differential coverage for
//! `str * int` / `int * str` repetition.
//!
//! #574 taught the frontend to type repetition as `str`; codegen answered it
//! with the D-072 exit-101 boundary. This part emits a real call to
//! `pycc_rt_str_repeat`, so the observable contract is now the compiled
//! program's stdout. Every expectation here is CPython 3.14's own output for
//! the same source (checked against the pinned oracle by
//! `tests/conformance.rs`'s `str_repetition_matches_cpython_3_14_7_byte_for_byte`
//! over `tests/fixtures/str_repetition.py`); these tests restate it without
//! the oracle so the behavior is gated on every CI run, not only on the
//! oracle-bearing job.
//!
//! Negative counts are written `0 - 2` rather than `-2`: these tests predate
//! #602's literal-sign fold, and the `BinOp::Sub` form remains an equally
//! valid way to reach the non-positive-count rule (empty string, matching
//! CPython).

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

/// Build `source` with the public CLI and return the compiled program's stdout.
fn build_and_run(case: &str, source: &str) -> Vec<u8> {
    let dir = ScratchDir::new(&format!("issue575_{case}")).expect("failed to create scratch dir");
    let src = write_fixture(&dir, "repeat.py", source);
    let out = dir.join("repeat");

    let output = Command::new(pycc_bin())
        .args(["build", src.to_str().unwrap(), "-o", out.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "pycc build should succeed for {case}, got {:?}: {}",
        output.status.code(),
        String::from_utf8_lossy(&output.stdout)
    );

    Command::new(&out).output().unwrap().stdout
}

/// `str * int` executes natively instead of hitting the retired D-072
/// exit-101 boundary.
#[test]
fn str_times_int_prints_the_repeated_string() {
    assert_eq!(build_and_run("str_int", "print(\"ab\" * 3)\n"), b"ababab\n");
}

/// `int * str` -- the reversed operand order shares the same emission path,
/// with the str operand selected by type rather than by position.
#[test]
fn int_times_str_prints_the_repeated_string() {
    assert_eq!(build_and_run("int_str", "print(3 * \"ab\")\n"), b"ababab\n");
}

/// A `bool` count is an `int` count (D-061's tagged encoding covers both), in
/// either operand order.
#[test]
fn a_bool_count_repeats_once_or_not_at_all_in_both_orders() {
    assert_eq!(
        build_and_run(
            "bool_count",
            "print(\"ab\" * True)\nprint(True * \"ab\")\nprint(\"ab\" * False)\nprint(False * \"ab\")\n",
        ),
        b"ab\nab\n\n\n",
    );
}

/// Zero and negative counts both yield the empty string, matching CPython.
#[test]
fn a_non_positive_count_yields_the_empty_string_in_both_orders() {
    assert_eq!(
        build_and_run(
            "non_positive",
            "negative = 0 - 2\nprint(\"ab\" * 0)\nprint(0 * \"ab\")\nprint(\"ab\" * negative)\nprint(negative * \"ab\")\n",
        ),
        b"\n\n\n\n",
    );
}

/// A repetition whose result crosses D-059's 22-byte inline payload threshold
/// takes `new_pystr`'s heap branch, through the public CLI rather than only in
/// `pycc_rt`'s own unit tests.
#[test]
fn a_repetition_past_the_inline_payload_threshold_prints_in_full() {
    assert_eq!(
        build_and_run("heap_payload", "print(\"0123456789\" * 5)\n"),
        b"01234567890123456789012345678901234567890123456789\n",
    );
}

/// Repetition composes with concatenation, f-string interpolation, a
/// non-literal count, and a `str`-returning function.
#[test]
fn repetition_composes_with_the_rest_of_the_str_surface() {
    assert_eq!(
        build_and_run(
            "composition",
            concat!(
                "def banner(word: str, width: int) -> str:\n",
                "    return word * width\n",
                "\n",
                "count = 4\n",
                "doubled = \"ab\" * 2\n",
                "print(\"xy\" * count)\n",
                "print(doubled + \"!\")\n",
                "print(f\"[{doubled}]\")\n",
                "print(banner(\"-\", 7))\n",
                "print(banner(\"-\", 0))\n",
            ),
        ),
        b"xyxyxyxy\nabab!\n[abab]\n-------\n\n",
    );
}

/// The frontend still rejects `str * str`: repetition needs an int-or-bool
/// count, and nothing in this part widened that rule.
#[test]
fn multiplying_two_strings_is_still_a_type_error() {
    let dir = ScratchDir::new("issue575_str_str").expect("failed to create scratch dir");
    let src = write_fixture(&dir, "str_str.py", "print(\"ab\" * \"cd\")\n");

    let output = Command::new(pycc_bin())
        .args(["check", src.to_str().unwrap()])
        .output()
        .unwrap();
    assert_eq!(
        output.status.code(),
        Some(1),
        "pycc check should reject `str * str` with exit code 1"
    );
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        combined.contains("T0021"),
        "output should carry the T0021 operand-type diagnostic, got: {combined}"
    );
}
