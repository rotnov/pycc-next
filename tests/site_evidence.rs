//! Production execution proofs first prepared for the website's Language and
//! Diagnostics evidence heroes. D-259 retired those pages; the tests remain as
//! end-to-end checks of the two fixtures. No compiler behavior is changed here.

use pycc_scratch::ScratchDir;
use serde_json::json;
use std::path::Path;
use std::process::{Command, Output};

const LANGUAGE_SOURCE: &str = "tests/fixtures/pep_0526_var_annotations.py";
const DIAGNOSTIC_SOURCE: &str = "tests/diagnostics/d0021_range_argument_type.py";

fn repo_root() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
}

fn pycc(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_pycc"))
        .args(args)
        .current_dir(repo_root())
        .output()
        .expect("could not execute the production pycc binary")
}

// D-082: only remove CR immediately before LF. Apply this to checked-out
// text fixtures (Git autocrlf) and CPython stdio, never pycc output.
fn canonical_lf(bytes: &[u8]) -> Vec<u8> {
    bytes
        .iter()
        .enumerate()
        .filter_map(|(i, &byte)| {
            (byte != b'\r' || bytes.get(i + 1) != Some(&b'\n')).then_some(byte)
        })
        .collect()
}

fn language_snapshot() -> Vec<u8> {
    canonical_lf(include_bytes!(
        "fixtures/pep_0526_var_annotations.expected.txt"
    ))
}

fn assert_transcript(output: &Output, exit: i32, stdout: &[u8]) {
    assert_eq!(output.status.code(), Some(exit), "{output:?}");
    assert!(output.stderr.is_empty(), "unexpected stderr: {output:?}");
    assert_eq!(output.stdout, stdout, "stdout transcript drift");
}

fn oracle_name(is_windows: bool) -> &'static str {
    if is_windows {
        "python3.14.exe"
    } else {
        "python3.14"
    }
}

fn check_oracle_version(output: &Output) -> Result<(), String> {
    if output.status.code() != Some(0) || !output.stderr.is_empty() {
        return Err(format!("oracle version probe failed: {output:?}"));
    }
    if canonical_lf(&output.stdout) != b"Python 3.14.7\n" {
        return Err(format!(
            "oracle must be exactly Python 3.14.7, found {:?}",
            String::from_utf8_lossy(&output.stdout)
        ));
    }
    Ok(())
}

fn require_oracle(bin: &Path) -> Result<(), String> {
    let version = Command::new(bin)
        .arg("--version")
        .current_dir(repo_root())
        .output()
        .map_err(|e| format!("could not execute pinned oracle {}: {e}", bin.display()))?;
    check_oracle_version(&version)
}

#[test]
fn language_command_matches_canonical_stdout() {
    // Exact public command; `run` defaults to debug. The existing PEP 526
    // conformance test, unchanged, separately proves debug and release builds.
    let output = pycc(&["run", LANGUAGE_SOURCE]);
    assert_transcript(&output, 0, &language_snapshot());
}

#[test]
#[ignore = "requires a pinned python3.14 (CPython 3.14.7) oracle on PATH"]
fn language_commands_match_cpython_3_14_7_and_canonical_stdout() {
    // D-080: CI runs this through --include-ignored after oracle setup, never
    // inside the pre-oracle coverage sandbox. Missing/wrong versions fail.
    let oracle = Path::new(oracle_name(cfg!(windows)));
    require_oracle(oracle).unwrap_or_else(|e| panic!("{e}"));
    let pycc_output = pycc(&["run", LANGUAGE_SOURCE]);
    let cpython_output = Command::new(oracle)
        .arg(LANGUAGE_SOURCE)
        .current_dir(repo_root())
        .output()
        .expect("could not execute pinned CPython fixture");
    assert_eq!(cpython_output.status.code(), Some(0), "{cpython_output:?}");
    assert!(cpython_output.stderr.is_empty(), "{cpython_output:?}");
    let cpython_stdout = canonical_lf(&cpython_output.stdout);
    let expected = language_snapshot();
    assert_transcript(&pycc_output, 0, &expected);
    assert_eq!(cpython_stdout, expected, "CPython snapshot drift");
    assert_eq!(
        pycc_output.stdout, cpython_stdout,
        "independent oracle diff"
    );
}

#[test]
fn diagnostics_commands_match_human_and_json_snapshots() {
    let human = pycc(&["check", DIAGNOSTIC_SOURCE]);
    let structured = pycc(&["check", DIAGNOSTIC_SOURCE, "--error-format", "json"]);
    assert_transcript(
        &human,
        1,
        &canonical_lf(include_bytes!(
            "diagnostics/d0021_range_argument_type.expected.txt"
        )),
    );
    assert_transcript(
        &structured,
        1,
        &canonical_lf(include_bytes!(
            "diagnostics/d0021_range_argument_type.expected.json"
        )),
    );

    // Bind the documented meaning as well as serialization. Human output has
    // no help line; its caret is the 1:1 placeholder, not argument precision.
    let diagnostic: serde_json::Value = serde_json::from_slice(&structured.stdout).unwrap();
    let message = "range stop expects `int`, got `str`";
    assert_eq!(
        diagnostic,
        json!({
            "code": "T0021", "format_version": 1, "severity": "error",
            "message": message, "help": ["pass an `int` value"],
            "spans": [{"file": DIAGNOSTIC_SOURCE, "line": 1, "col": 1,
                       "len": 0, "label": message}]
        })
    );
    let human_text = std::str::from_utf8(&human.stdout).unwrap();
    assert!(human_text.starts_with(&format!("error[T0021]: {message}\n")));
    assert!(human_text.contains(&format!(" --> {DIAGNOSTIC_SOURCE}:1:1\n")));
    assert!(human_text.ends_with(&format!("  | ^ {message}\n")));
    assert!(!human_text.contains("help:"));
}

#[test]
fn missing_or_non_oracle_program_is_rejected_without_fallback() {
    let scratch = ScratchDir::new("site_evidence_oracle").unwrap();
    let missing = scratch.join(oracle_name(cfg!(windows)));
    assert!(
        require_oracle(&missing)
            .unwrap_err()
            .contains("could not execute pinned oracle")
    );
    // This non-oracle rejects --version. Its failed probe must not fall back
    // to an ambient Python interpreter.
    assert!(
        require_oracle(Path::new(env!("CARGO_BIN_EXE_pycc")))
            .unwrap_err()
            .contains("oracle version probe failed")
    );
}

#[test]
fn version_probe_rejects_wrong_patch_malformed_output_and_failed_process() {
    // Synthetic version bytes test the validator, not a CPython execution.
    let mut probe = pycc(&["--help"]);
    assert_eq!(probe.status.code(), Some(0));
    assert!(probe.stderr.is_empty());
    for valid in [b"Python 3.14.7\n".as_slice(), b"Python 3.14.7\r\n"] {
        probe.stdout = valid.to_vec();
        assert!(check_oracle_version(&probe).is_ok());
    }
    for invalid in [
        b"Python 3.14.6\n".as_slice(),
        b"Python 3.14.8\n",
        b"Python 3.14.7rc1\n",
        b"Python 3.14.7",
        b"Python 3.14.7\n\n",
        b" Python 3.14.7\n",
        b"",
        b"\xff",
    ] {
        probe.stdout = invalid.to_vec();
        assert!(
            check_oracle_version(&probe)
                .unwrap_err()
                .contains("oracle must be exactly")
        );
    }
    probe.stdout = b"Python 3.14.7\n".to_vec();
    probe.stderr = b"warning\n".to_vec();
    assert!(
        check_oracle_version(&probe)
            .unwrap_err()
            .contains("probe failed")
    );
    probe.stderr.clear();
    probe.status = pycc(&["check", DIAGNOSTIC_SOURCE]).status;
    assert!(
        check_oracle_version(&probe)
            .unwrap_err()
            .contains("probe failed")
    );
}

#[test]
fn newline_translation_preserves_other_bytes_and_transcript_boundaries() {
    assert_eq!(canonical_lf(b"a\r\nb\n\r\r\n"), b"a\nb\n\r\n");
    for unchanged in [
        b"".as_slice(),
        b"15\n",
        b"15",
        b"15 \n",
        b"15\n\n",
        b"a\rb\r",
    ] {
        assert_eq!(canonical_lf(unchanged), unchanged);
    }
    assert_eq!(oracle_name(true), "python3.14.exe");
    assert_eq!(oracle_name(false), "python3.14");
}
