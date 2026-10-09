//! #864 Part 1 (D-217): `pycc check` reports every diagnostic the failing
//! frontend pass collected for a file, and `pycc build`/`pycc run` print the
//! same renders to stderr before stopping. The byte-exact pins live in
//! `tests/diagnostics_test.rs` (`l0001_two_syntax_errors`,
//! `c0001_issue_864_repro`); the tests here check the *structure* of the
//! multi-diagnostic output that the whole-stdout snapshots cannot express
//! on their own: one JSON object per line with no blank separators, per-file
//! output concatenated byte-for-byte across files, and the `build` stderr
//! mirror of `check`'s human stdout. Part 2 (#867, D-219) adds the HIR
//! fan-out case: two `C0001`s from one file reach the driver through the
//! same payload. Part 3 (#868, D-220) adds the type-checker cases at the
//! bottom: one diagnostic per failing function in solver-first order, the
//! HIR-failure-stops boundary, and a no-panic sweep of the corpus.

use pycc_scratch::ScratchDir;
use std::path::Path;
use std::process::{Command, Output};

/// `def main(:\n` -- exactly two ruff parse errors (see the fixture's
/// comment in `tests/diagnostics_test.rs`).
const FIXTURE: &str = "tests/diagnostics/l0001_two_syntax_errors.py";

fn pycc_bin() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_BIN_EXE_pycc"))
}

fn repo_root() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
}

/// Runs `pycc check <paths...> [--error-format json]` from the repo root so
/// the repo-relative fixture path is embedded verbatim (the same convention
/// `tests/diagnostics_test.rs` uses).
fn check(paths: &[&str], json: bool) -> Output {
    let mut cmd = Command::new(pycc_bin());
    cmd.arg("check").args(paths).current_dir(repo_root());
    if json {
        cmd.args(["--error-format", "json"]);
    }
    cmd.output().unwrap()
}

fn stdout(output: &Output) -> String {
    String::from_utf8(output.stdout.clone()).unwrap()
}

fn stderr(output: &Output) -> String {
    String::from_utf8(output.stderr.clone()).unwrap()
}

fn read_fixture(name: &str) -> String {
    std::fs::read_to_string(repo_root().join("tests/diagnostics").join(name))
        .unwrap()
        .replace("\r\n", "\n")
}

#[test]
fn json_output_is_one_object_per_line_with_no_separators() {
    let output = check(&[FIXTURE], true);
    assert_eq!(output.status.code(), Some(1));
    let text = stdout(&output);
    // `lines()` deliberately not filtered: a blank separator line between the
    // two objects would count as a third line here.
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines.len(), 2, "expected exactly two JSON lines:\n{text}");
    for line in &lines {
        let object: serde_json::Value = serde_json::from_str(line).unwrap();
        assert_eq!(object["code"], "L0001");
        assert_eq!(object["format_version"], 1);
    }
    // Every object ends with exactly one newline (kills `print!` without a
    // newline, a missing trailing newline, and a doubled separator).
    assert_eq!(text, format!("{}\n{}\n", lines[0], lines[1]));
}

#[test]
fn multi_file_output_is_the_concatenation_of_per_file_output() {
    let dir = ScratchDir::new("issue_864_multi_file").expect("failed to create scratch dir");
    let second = dir.join("second.py");
    // Two ruff errors as well, on different spans than the fixture's, so a
    // within-file reorder or a cross-file interleave changes the bytes.
    std::fs::write(&second, "x = (1 +\ny = 2\n").unwrap();
    let second = second.to_string_lossy().into_owned();

    for json in [false, true] {
        let a = check(&[FIXTURE], json);
        let b = check(&[&second], json);
        let both = check(&[FIXTURE, &second], json);
        assert_eq!(both.status.code(), Some(1));
        assert_eq!(
            stdout(&both),
            format!("{}{}", stdout(&a), stdout(&b)),
            "json={json}: multi-file stdout must be the per-file outputs, in order"
        );
        assert!(
            stdout(&a).matches("L0001").count() >= 2 && stdout(&b).matches("L0001").count() >= 2,
            "json={json}: both inputs must contribute several diagnostics"
        );
    }
}

#[test]
fn unreadable_middle_path_keeps_every_diagnostic_and_exits_2() {
    let dir = ScratchDir::new("issue_864_unreadable_middle").expect("failed to create scratch dir");
    let missing = dir.join("missing.py").to_string_lossy().into_owned();
    let second = dir.join("second.py");
    std::fs::write(&second, "x = (1 +\ny = 2\n").unwrap();
    let second = second.to_string_lossy().into_owned();

    let a = check(&[FIXTURE], true);
    let b = check(&[&second], true);
    let all = check(&[FIXTURE, &missing, &second], true);
    // `2` (unreadable input) takes precedence over `1` (compile diagnostics)
    // in the existing per-file `max` fold; the diagnostics themselves still
    // reach stdout while the `Input` failure goes to stderr.
    assert_eq!(all.status.code(), Some(2));
    assert_eq!(stdout(&all), format!("{}{}", stdout(&a), stdout(&b)));
    assert!(
        stderr(&all).contains("error: could not read"),
        "stderr was: {}",
        stderr(&all)
    );
}

#[test]
fn build_prints_every_diagnostic_to_stderr_and_stops_before_codegen() {
    let dir = ScratchDir::new("issue_864_build_stderr").expect("failed to create scratch dir");
    let out = dir.join("out");
    let output = Command::new(pycc_bin())
        .args(["build", FIXTURE, "-o"])
        .arg(&out)
        .current_dir(repo_root())
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    assert!(stdout(&output).is_empty(), "build must not write to stdout");
    assert!(!out.exists(), "no output artifact may be produced");
    // `build`'s stderr is exactly `check`'s human stdout for the same file:
    // every collected diagnostic, same renders, same order. A first-only
    // `report_build_failure` fails here.
    assert_eq!(
        stderr(&output),
        read_fixture("l0001_two_syntax_errors.expected.txt")
    );
}

/// #867 (D-219): HIR lowering's per-item collection reaches the driver
/// through the same `Vec<Diagnostic>` payload as the parser's fan-out: two
/// `C0001` objects in source order as JSON Lines, and the same two human
/// renders on `build`'s stderr, which exits 1 before codegen.
#[test]
fn hir_per_item_diagnostics_reach_check_json_and_build_stderr() {
    const HIR_FIXTURE: &str = "tests/diagnostics/c0001_issue_864_repro.py";
    let output = check(&[HIR_FIXTURE], true);
    assert_eq!(output.status.code(), Some(1));
    let text = stdout(&output);
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines.len(), 2, "expected exactly two JSON lines:\n{text}");
    let mut seen_lines = Vec::new();
    for line in &lines {
        let object: serde_json::Value = serde_json::from_str(line).unwrap();
        assert_eq!(object["code"], "C0001");
        seen_lines.push(object["spans"][0]["line"].as_u64().unwrap());
    }
    // Source order: the HIR pass's own collection order is the loop order
    // over top-level items (no re-sort, D-217 rule 3).
    assert_eq!(seen_lines, vec![2, 4]);

    let dir = ScratchDir::new("issue_867_build_stderr").expect("failed to create scratch dir");
    let out = dir.join("out");
    let build = Command::new(pycc_bin())
        .args(["build", HIR_FIXTURE, "-o"])
        .arg(&out)
        .current_dir(repo_root())
        .output()
        .unwrap();
    assert_eq!(build.status.code(), Some(1));
    assert!(stdout(&build).is_empty(), "build must not write to stdout");
    assert!(!out.exists(), "no output artifact may be produced");
    assert_eq!(
        stderr(&build),
        read_fixture("c0001_issue_864_repro.expected.txt")
    );
    assert_eq!(stderr(&build).matches("error[C0001]").count(), 2);
}

/// #868 (D-220): the type checker's per-function list reaches the driver
/// through the same payload: three objects for the three broken functions
/// of the fixture, in solver-first order (`f`'s T0022 and `h`'s T0021 from
/// the solver's body walk, then `g`'s T0043 from the annotation checker),
/// and the same three human renders on `build`'s stderr.
#[test]
fn type_per_function_diagnostics_reach_check_json_and_build_stderr() {
    const TYPES_FIXTURE: &str = "tests/diagnostics/t0022_types_per_function.py";
    let output = check(&[TYPES_FIXTURE], true);
    assert_eq!(output.status.code(), Some(1));
    let text = stdout(&output);
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines.len(), 3, "expected exactly three JSON lines:\n{text}");
    let codes: Vec<String> = lines
        .iter()
        .map(|line| {
            let object: serde_json::Value = serde_json::from_str(line).unwrap();
            object["code"].as_str().unwrap().to_string()
        })
        .collect();
    assert_eq!(codes, vec!["T0022", "T0021", "T0043"]);

    let dir = ScratchDir::new("issue_868_build_stderr").expect("failed to create scratch dir");
    let out = dir.join("out");
    let build = Command::new(pycc_bin())
        .args(["build", TYPES_FIXTURE, "-o"])
        .arg(&out)
        .current_dir(repo_root())
        .output()
        .unwrap();
    assert_eq!(build.status.code(), Some(1));
    assert!(stdout(&build).is_empty(), "build must not write to stdout");
    assert!(!out.exists(), "no output artifact may be produced");
    assert_eq!(
        stderr(&build),
        read_fixture("t0022_types_per_function.expected.txt")
    );
    assert_eq!(stderr(&build).matches("error[T").count(), 3);
}

/// #868 decision A: a HIR lowering failure still stops before the type
/// checker runs, so a file with both a `C0001` and a type error reports
/// only the `C0001` -- the type checker's list never mixes with an earlier
/// pass's.
#[test]
fn hir_failure_still_stops_before_the_type_checker() {
    let dir = ScratchDir::new("issue_868_hir_stops").expect("failed to create scratch dir");
    let path = dir.join("mixed.py");
    std::fs::write(
        &path,
        "async def g() -> None:\n    pass\n\n\ndef f() -> int:\n    return \"a\"\n",
    )
    .unwrap();
    let path = path.to_string_lossy().into_owned();
    let output = check(&[&path], true);
    assert_eq!(output.status.code(), Some(1));
    let text = stdout(&output);
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines.len(), 1, "expected exactly one JSON line:\n{text}");
    let object: serde_json::Value = serde_json::from_str(lines[0]).unwrap();
    assert_eq!(object["code"], "C0001");
    assert!(
        !text.contains("\"T0"),
        "no type diagnostic may follow a HIR failure:\n{text}"
    );
}

/// #868: the per-function collectors keep walking after a body fails, so a
/// later body is checked in a state the pre-#868 driver never reached. This
/// sweeps every `.py` file in the repository's fixture corpora that lowers
/// through both public entry points and asserts only that neither panics
/// and that neither ever returns an empty `Err`.
#[test]
fn type_checker_entry_points_never_panic_on_the_fixture_corpus() {
    let mut sources = Vec::new();
    for corpus in ["tests/fixtures", "tests/diagnostics", "tests/regress"] {
        collect_py_files(&repo_root().join(corpus), &mut sources);
    }
    sources.sort();
    assert!(
        sources.len() > 100,
        "corpus unexpectedly small: {}",
        sources.len()
    );
    let mut lowered = 0usize;
    for path in &sources {
        let source = std::fs::read_to_string(path).unwrap();
        let Ok(module) = pycc_parser::parse(&source) else {
            continue;
        };
        let Ok(hir) = pycc_hir::lower_checked(&module) else {
            continue;
        };
        lowered += 1;
        if let Err(diagnostics) = pycc_types::check_all(&hir) {
            assert!(!diagnostics.is_empty(), "{}: empty Err", path.display());
        }
        if let Err(diagnostics) = pycc_types::check_and_resolve_all(&hir) {
            assert!(!diagnostics.is_empty(), "{}: empty Err", path.display());
        }
    }
    assert!(lowered > 50, "too few fixtures lowered: {lowered}");
}

fn collect_py_files(dir: &Path, out: &mut Vec<std::path::PathBuf>) {
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            collect_py_files(&path, out);
        } else if path.extension().is_some_and(|ext| ext == "py") {
            out.push(path);
        }
    }
}
