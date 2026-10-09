//! Guards the conformance harness's differential assertions against becoming
//! tautological (#224). The harness is `tests/conformance.rs` plus its
//! `tests/conformance/*.rs` cohort submodules, read as one text in that order
//! by `harness_support/conformance_sources.rs` (shared with
//! `tests/conformance_matrix_guard.rs`).
//!
//! Every conformance claim in `docs/PYTHON_STANDARDS.md` ultimately rests on one
//! sentence of `tests/conformance.rs`: the shared helper returns the compiled
//! pycc binary's stdout as its first tuple element and the pinned CPython
//! oracle's stdout as its second, and each per-PEP test compares those two.
//! Nothing in the repository bound that sentence. A helper returning
//! `(pycc_output.stdout.clone(), pycc_output.stdout)` still builds pycc, still
//! launches the pinned `python3.14`, still asserts both processes exited zero --
//! and turns all 40 differential tests into `assert_eq!(x, x)` while every gate
//! stays green.
//!
//! This guard closes that hole by reading the harness sources as text and
//! asserting their differential-oracle semantics structurally. It deliberately
//! needs neither LLVM nor the oracle interpreter, so it is a genuine negative
//! control rather than a second copy of the tests it guards: the mutations below
//! are applied to synthetic harness sources in this file's own tests and the
//! guard is required to reject each one, while the real harness is required to
//! pass.
//!
//! Scope boundary: `tests/conformance_matrix_guard.rs` (#578/#583) checks that
//! evidence-backed matrix rows cite fixtures that exist and are registered here.
//! It says nothing about what those registered tests actually compare. This
//! guard owns that second half.
//!
//! Ordering invariant: the mutation controls at the bottom of this file rewrite
//! the *first* occurrence of a pattern in the concatenated harness text, so the
//! root-first, sorted-cohort order is part of the contract, and the root must
//! keep at least one dual-profile differential test (the PR-9 seed tests such
//! as `pep_0526_var_annotations_...` stay there) so the rewritten occurrence is
//! the same function for every reader.

use std::collections::BTreeSet;
use std::path::PathBuf;

#[path = "harness_support/conformance_sources.rs"]
mod conformance_sources;

/// Differential tests are named `<subject>_matches_cpython_3_14_7_byte_for_byte`.
const DIFFERENTIAL_SUFFIX: &str = "_matches_cpython_3_14_7_byte_for_byte";
/// The dual-profile helper every PEP-matrix fixture must go through.
const PROFILE_HELPER: &str = "run_conformance_fixture_with_profile";
/// The `--debug`-only convenience wrapper, which must delegate to the above.
const PLAIN_HELPER: &str = "run_conformance_fixture";

/// The only two differential tests allowed to stay on the `--debug`-only
/// wrapper. Both predate `docs/TESTING.md`'s "both profiles" rule and neither
/// is a PEP-matrix row, as `run_conformance_fixture_with_profile`'s own doc
/// comment records. A new test cannot join this list without editing it here.
const DEBUG_ONLY_TESTS: [&str; 2] = [
    "fib_matches_cpython_3_14_7_byte_for_byte",
    "mandelbrot_ascii_matches_cpython_3_14_7_byte_for_byte",
];

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// Index just past the closing quote of the string literal opening at `quote`.
fn end_of_string(chars: &[char], quote: usize) -> usize {
    let mut i = quote + 1;
    while i < chars.len() {
        if chars[i] == '\\' {
            i += 2;
            continue;
        }
        if chars[i] == '"' {
            return i + 1;
        }
        i += 1;
    }
    chars.len()
}

/// Blanks out line and block comments, preserving string literals verbatim and
/// keeping newlines so line structure survives. Everything downstream therefore
/// analyses real code: "the comparison is only in a comment" needs no separate
/// check, because commented-out code is gone by the time the guard looks.
fn strip_comments(src: &str) -> String {
    let chars: Vec<char> = src.chars().collect();
    let mut out = String::with_capacity(src.len());
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '"' {
            let end = end_of_string(&chars, i);
            out.extend(&chars[i..end]);
            i = end;
            continue;
        }
        if chars[i] == '/' && chars.get(i + 1) == Some(&'/') {
            while i < chars.len() && chars[i] != '\n' {
                out.push(' ');
                i += 1;
            }
            continue;
        }
        if chars[i] == '/' && chars.get(i + 1) == Some(&'*') {
            let mut depth = 0usize;
            while i < chars.len() {
                if chars[i] == '/' && chars.get(i + 1) == Some(&'*') {
                    depth += 1;
                    out.push_str("  ");
                    i += 2;
                    continue;
                }
                if chars[i] == '*' && chars.get(i + 1) == Some(&'/') {
                    depth -= 1;
                    out.push_str("  ");
                    i += 2;
                    if depth == 0 {
                        break;
                    }
                    continue;
                }
                out.push(if chars[i] == '\n' { '\n' } else { ' ' });
                i += 1;
            }
            continue;
        }
        out.push(chars[i]);
        i += 1;
    }
    out
}

fn find_at(hay: &[char], needle: &[char], from: usize) -> Option<usize> {
    hay.windows(needle.len())
        .enumerate()
        .skip(from)
        .find(|(_, window)| *window == needle)
        .map(|(index, _)| index)
}

/// Index of the delimiter closing the one that opens at `open`, skipping string
/// literals so a `")"` inside an assertion message cannot unbalance the scan.
fn matching(chars: &[char], open: usize) -> usize {
    let opener = chars[open];
    let closer = if opener == '(' { ')' } else { '}' };
    let mut depth = 0usize;
    let mut i = open;
    while i < chars.len() {
        if chars[i] == '"' {
            i = end_of_string(chars, i);
            continue;
        }
        if chars[i] == opener {
            depth += 1;
        } else if chars[i] == closer {
            depth -= 1;
            if depth == 0 {
                return i;
            }
        }
        i += 1;
    }
    panic!("unbalanced delimiter opened at offset {open}");
}

/// Splits on `sep` at nesting depth zero, skipping string literals, and drops
/// the empty tail a trailing comma leaves behind.
fn split_top_level(chars: &[char], sep: char) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut current = String::new();
    let mut depth = 0i32;
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c == '"' {
            let end = end_of_string(chars, i);
            current.extend(&chars[i..end]);
            i = end;
            continue;
        }
        if c == '(' || c == '[' || c == '{' {
            depth += 1;
        } else if c == ')' || c == ']' || c == '}' {
            depth -= 1;
        }
        if c == sep && depth == 0 {
            out.push(current.trim().to_string());
            current = String::new();
        } else {
            current.push(c);
        }
        i += 1;
    }
    out.push(current.trim().to_string());
    out.into_iter().filter(|piece| !piece.is_empty()).collect()
}

struct FnItem {
    name: String,
    attrs: String,
    body: String,
}

fn preceding_attrs(text: &[char], start: usize) -> String {
    let before: String = text[..start].iter().collect();
    let mut attrs: Vec<String> = Vec::new();
    for line in before.lines().rev() {
        let trimmed = line.trim();
        if !trimmed.starts_with("#[") {
            break;
        }
        attrs.push(trimmed.to_string());
    }
    attrs.join("\n")
}

/// Every item-level `fn` in `src`, with its attribute block and its body's
/// interior text. Items nested inside a function body are deliberately not
/// collected: a comparison hidden in a nested helper is exactly the mutation
/// the depth checks below reject.
fn parse_fns(src: &str) -> Vec<FnItem> {
    let text: Vec<char> = format!("\n{src}").chars().collect();
    let marker: Vec<char> = "\nfn ".chars().collect();
    let mut out = Vec::new();
    let mut cursor = 0;
    while let Some(found) = find_at(&text, &marker, cursor) {
        let start = found + marker.len();
        let name: String = text[start..]
            .iter()
            .take_while(|c| c.is_alphanumeric() || **c == '_')
            .collect();
        let open = find_at(&text, &['{'], start).expect("a `fn` item without a body");
        let close = matching(&text, open);
        out.push(FnItem {
            name,
            attrs: preceding_attrs(&text, found + 1),
            body: text[open + 1..close].iter().collect(),
        });
        cursor = close;
    }
    out
}

/// A macro invocation's nesting depth within the enclosing body, plus its
/// top-level arguments.
struct MacroCall {
    depth: usize,
    args: Vec<String>,
}

fn macro_calls(body: &str, name: &str) -> Vec<MacroCall> {
    let chars: Vec<char> = body.chars().collect();
    let needle: Vec<char> = format!("{name}(").chars().collect();
    let mut out = Vec::new();
    let mut cursor = 0;
    while let Some(found) = find_at(&chars, &needle, cursor) {
        let open = found + needle.len() - 1;
        let close = matching(&chars, open);
        out.push(MacroCall {
            depth: brace_depth(&chars, found),
            args: split_top_level(&chars[open + 1..close], ','),
        });
        cursor = close;
    }
    out
}

fn brace_depth(chars: &[char], index: usize) -> usize {
    let mut depth = 0usize;
    let mut i = 0;
    while i < index {
        if chars[i] == '"' {
            i = end_of_string(chars, i);
            continue;
        }
        if chars[i] == '{' {
            depth += 1;
        } else if chars[i] == '}' {
            depth -= 1;
        }
        i += 1;
    }
    depth
}

/// One `let (<pycc>, <cpython>) = run_conformance_fixture*(..)` binding.
struct Destructure {
    pycc: String,
    cpython: String,
    callee: String,
    args: Vec<String>,
}

fn parse_destructuring(chars: &[char], at: usize) -> Option<Destructure> {
    let open = at + "let ".len();
    let close = matching(chars, open);
    let names = split_top_level(&chars[open + 1..close], ',');
    let [pycc, cpython] = <[String; 2]>::try_from(names).ok()?;
    let tail: String = chars[close + 1..].iter().collect();
    let assigned = tail
        .trim_start()
        .strip_prefix('=')?
        .trim_start()
        .to_string();
    let callee: String = assigned
        .chars()
        .take_while(|c| c.is_alphanumeric() || *c == '_')
        .collect();
    if !callee.starts_with(PLAIN_HELPER) {
        return None;
    }
    let call_chars: Vec<char> = assigned.chars().collect();
    let call_open = callee.chars().count();
    if call_chars.get(call_open) != Some(&'(') {
        return None;
    }
    let call_close = matching(&call_chars, call_open);
    Some(Destructure {
        pycc,
        cpython,
        callee,
        args: split_top_level(&call_chars[call_open + 1..call_close], ','),
    })
}

fn destructurings(body: &str) -> Vec<Destructure> {
    let chars: Vec<char> = body.chars().collect();
    let needle: Vec<char> = "let (".chars().collect();
    let mut out = Vec::new();
    let mut cursor = 0;
    while let Some(found) = find_at(&chars, &needle, cursor) {
        out.extend(parse_destructuring(&chars, found));
        cursor = found + needle.len();
    }
    out
}

/// The helper's trailing expression: everything after its last top-level `;`.
fn trailing_expression(body: &str) -> String {
    let chars: Vec<char> = body.chars().collect();
    let pieces = split_top_level(&chars, ';');
    pieces.last().cloned().unwrap_or_default()
}

fn audit_helper(helper: &FnItem, errors: &mut Vec<String>) {
    for (needle, complaint) in [
        (
            "oracle_python_bin()",
            "never launches the pinned CPython oracle",
        ),
        (
            "cpython_output.status.success()",
            "never asserts the CPython oracle process succeeded",
        ),
        (
            "pycc_output.status.success()",
            "never asserts the compiled pycc binary succeeded",
        ),
    ] {
        if !helper.body.contains(needle) {
            errors.push(format!("`{PROFILE_HELPER}` {complaint} (#224)"));
        }
    }

    let tail = trailing_expression(&helper.body);
    let tail = tail.trim();
    if !tail.starts_with('(') || !tail.ends_with(')') {
        errors.push(format!(
            "`{PROFILE_HELPER}` must end in a two-element tuple literal, found `{tail}` (#224)"
        ));
        return;
    }
    let inner: Vec<char> = tail[1..tail.len() - 1].chars().collect();
    let elements = split_top_level(&inner, ',');
    let [first, second] = match <[String; 2]>::try_from(elements) {
        Ok(pair) => pair,
        Err(found) => {
            errors.push(format!(
                "`{PROFILE_HELPER}` must return exactly two elements, found {} (#224)",
                found.len()
            ));
            return;
        }
    };
    if !first.contains("pycc_output.stdout") || first.contains("cpython_output") {
        errors.push(format!(
            "`{PROFILE_HELPER}`'s first element must derive from the pycc process alone, \
             found `{first}` (#224)"
        ));
    }
    if !second.contains("cpython_output.stdout") || second.contains("pycc_output") {
        errors.push(format!(
            "`{PROFILE_HELPER}`'s second element must derive from the CPython oracle alone, \
             found `{second}` (#224)"
        ));
    }
}

fn audit_differential(item: &FnItem, errors: &mut Vec<String>) {
    let name = &item.name;
    for attribute in ["#[test]", "#[ignore"] {
        if !item.attrs.contains(attribute) {
            errors.push(format!("`{name}` is missing `{attribute}` (#224)"));
        }
    }

    let asserts = macro_calls(&item.body, "assert_eq!");
    for call in &asserts {
        if call.args.len() >= 2 && call.args[0] == call.args[1] {
            errors.push(format!(
                "`{name}` compares `{}` with itself (#224)",
                call.args[0]
            ));
        }
    }

    let pairs = destructurings(&item.body);
    if pairs.is_empty() {
        errors.push(format!(
            "`{name}` never binds a `{PLAIN_HELPER}*` result pair (#224)"
        ));
        return;
    }

    let mut profiles: BTreeSet<String> = BTreeSet::new();
    let mut dual_profile = false;
    for pair in &pairs {
        if !pair.pycc.contains("pycc") {
            errors.push(format!(
                "`{name}` binds `{}` where the pycc-side stdout is expected (#224)",
                pair.pycc
            ));
        }
        if !pair.cpython.contains("cpython") {
            errors.push(format!(
                "`{name}` binds `{}` where the CPython-side stdout is expected (#224)",
                pair.cpython
            ));
        }
        let compared = asserts.iter().any(|call| {
            call.depth == 0
                && call.args.len() >= 2
                && call.args[0] == pair.pycc
                && call.args[1] == pair.cpython
        });
        if !compared {
            errors.push(format!(
                "`{name}` never compares (`{}`, `{}`) from its own `{}` call in a top-level \
                 `assert_eq!` (#224)",
                pair.pycc, pair.cpython, pair.callee
            ));
        }
        if pair.callee == PROFILE_HELPER {
            dual_profile = true;
            profiles.insert(pair.args.last().cloned().unwrap_or_default());
        }
    }

    if dual_profile {
        for (flag, profile) in [("false", "--debug"), ("true", "--release")] {
            if !profiles.contains(flag) {
                errors.push(format!(
                    "`{name}` never exercises the `{profile}` profile (#224)"
                ));
            }
        }
    } else if !DEBUG_ONLY_TESTS.contains(&name.as_str()) {
        errors.push(format!(
            "`{name}` uses the `--debug`-only `{PLAIN_HELPER}` without being listed in \
             DEBUG_ONLY_TESTS (#224)"
        ));
    }
}

/// Every structural complaint the guard has about `harness`. Empty means the
/// harness's differential assertions genuinely bind pycc's output to the
/// pinned oracle's.
fn audit(harness: &str) -> Vec<String> {
    let src = strip_comments(harness);
    let fns = parse_fns(&src);
    let mut errors = Vec::new();

    match fns.iter().find(|item| item.name == PROFILE_HELPER) {
        Some(helper) => audit_helper(helper, &mut errors),
        None => errors.push(format!(
            "`{PROFILE_HELPER}` is missing from the harness (#224)"
        )),
    }
    match fns.iter().find(|item| item.name == PLAIN_HELPER) {
        Some(plain) => {
            if !plain.body.contains(&format!("{PROFILE_HELPER}(")) {
                errors.push(format!(
                    "`{PLAIN_HELPER}` no longer delegates to `{PROFILE_HELPER}` (#224)"
                ));
            }
        }
        None => errors.push(format!(
            "`{PLAIN_HELPER}` is missing from the harness (#224)"
        )),
    }

    let differential: Vec<&FnItem> = fns
        .iter()
        .filter(|item| item.name.ends_with(DIFFERENTIAL_SUFFIX))
        .collect();
    if differential.is_empty() {
        errors.push(format!(
            "the harness declares no `*{DIFFERENTIAL_SUFFIX}` test (#224)"
        ));
    }
    for item in differential {
        audit_differential(item, &mut errors);
    }
    errors
}

/// The harness sources (`tests/conformance.rs` then `tests/conformance/*.rs`)
/// with line endings normalized. A Windows checkout can materialize them with
/// CRLF endings, and every structural pattern this guard reasons about —
/// including the mutation controls below — is written against LF.
fn harness() -> String {
    conformance_sources::harness_sources_in(&repo_root())
}

// --- Positive control: the real harness ------------------------------------

#[test]
fn the_conformance_harness_binds_every_differential_assertion_to_the_oracle() {
    let errors = audit(&harness());
    assert!(
        errors.is_empty(),
        "the conformance harness (tests/conformance.rs + tests/conformance/*.rs) no longer proves \
         pycc against the pinned CPython oracle:\n{}",
        errors.join("\n")
    );
}

#[test]
fn the_harness_declares_the_differential_tests_this_guard_expects() {
    let fns = parse_fns(&strip_comments(&harness()));
    let differential: Vec<&FnItem> = fns
        .iter()
        .filter(|item| item.name.ends_with(DIFFERENTIAL_SUFFIX))
        .collect();
    assert!(
        differential.len() >= 40,
        "expected at least the 40 differential tests present when #224 was closed, found {}",
        differential.len()
    );
    for expected in DEBUG_ONLY_TESTS {
        assert!(
            fns.iter().any(|item| item.name == expected),
            "DEBUG_ONLY_TESTS names `{expected}`, which the harness no longer declares"
        );
    }
}

/// Every `pep_*.py` fixture the harness registers must actually be exercised by
/// one of its `#[test]` functions, so a fixture cannot be registered (satisfying
/// `tests/conformance_matrix_guard.rs`) while no test ever runs it.
#[test]
fn every_registered_pep_fixture_is_exercised_by_a_test() {
    let src = strip_comments(&harness());
    let fns = parse_fns(&src);
    let dir = repo_root().join("tests").join("fixtures");
    let registered: Vec<String> = std::fs::read_dir(&dir)
        .expect("cannot read tests/fixtures/")
        .map(|entry| {
            entry
                .expect("cannot read a tests/fixtures/ entry")
                .file_name()
        })
        .map(|name| name.to_string_lossy().into_owned())
        .filter(|name| name.starts_with("pep_") && name.ends_with(".py"))
        .filter(|name| src.contains(&format!("tests/fixtures/{name}")))
        .collect();
    assert!(
        !registered.is_empty(),
        "no registered pep_*.py fixtures found — the guard would pass vacuously"
    );
    let unexercised: Vec<&String> = registered
        .iter()
        .filter(|name| {
            let needle = format!("tests/fixtures/{name}");
            !fns.iter()
                .any(|item| item.attrs.contains("#[test]") && item.body.contains(&needle))
        })
        .collect();
    assert!(
        unexercised.is_empty(),
        "these registered fixtures are named in the conformance harness but never run by a test: \
         {unexercised:?}"
    );
}

// --- Negative controls -----------------------------------------------------

/// A minimal harness with the same structure as the real one. Each control
/// below mutates exactly one thing about it.
const SYNTHETIC: &str = r####"
fn run_conformance_fixture(label: &str, py_path: &Path) -> (Vec<u8>, Vec<u8>) {
    run_conformance_fixture_with_profile(label, py_path, false)
}

fn run_conformance_fixture_with_profile(
    label: &str,
    py_path: &Path,
    release: bool,
) -> (Vec<u8>, Vec<u8>) {
    let pycc_output = Command::new(&out).output().unwrap();
    assert!(pycc_output.status.success(), "pycc binary ({label}) failed");
    let cpython_output = Command::new(oracle_python_bin()).arg(py_path).output().unwrap();
    assert!(cpython_output.status.success(), "oracle failed");
    (
        pycc_output.stdout,
        strip_windows_newline_translation(cpython_output.stdout),
    )
}

#[test]
#[ignore = "requires a pinned python3.14 (CPython 3.14.7) oracle on PATH"]
fn pep_9999_demo_matches_cpython_3_14_7_byte_for_byte() {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/pep_9999_demo.py");
    let (debug_pycc, debug_cpython) =
        run_conformance_fixture_with_profile("pep_9999_demo_debug", &fixture, false);
    assert_eq!(
        debug_pycc, debug_cpython,
        "pycc (--debug) and CPython 3.14.7 disagree (a message with (parens) and a ; inside)"
    );
    let (release_pycc, release_cpython) =
        run_conformance_fixture_with_profile("pep_9999_demo_release", &fixture, true);
    assert_eq!(
        release_pycc, release_cpython,
        "pycc (--release) and CPython 3.14.7 disagree"
    );
}
"####;

fn mutate(from: &str, to: &str) -> String {
    assert!(
        SYNTHETIC.contains(from),
        "the synthetic harness no longer contains `{from}`"
    );
    SYNTHETIC.replacen(from, to, 1)
}

fn rejects(harness: &str, expected: &str) {
    let errors = audit(harness);
    assert!(
        errors.iter().any(|error| error.contains(expected)),
        "expected a complaint containing `{expected}`, got: {errors:?}"
    );
}

#[test]
fn the_synthetic_harness_is_itself_accepted() {
    assert_eq!(audit(SYNTHETIC), Vec::<String>::new());
}

#[test]
fn a_helper_returning_pycc_output_twice_is_rejected() {
    rejects(
        &mutate(
            "strip_windows_newline_translation(cpython_output.stdout),",
            "pycc_output.stdout.clone(),",
        ),
        "second element must derive from the CPython oracle alone",
    );
}

#[test]
fn a_helper_returning_the_oracle_output_twice_is_rejected() {
    rejects(
        &mutate("pycc_output.stdout,\n", "cpython_output.stdout.clone(),\n"),
        "first element must derive from the pycc process alone",
    );
}

#[test]
fn a_helper_that_never_launches_the_oracle_is_rejected() {
    rejects(
        &mutate("oracle_python_bin()", "pycc_bin()"),
        "never launches the pinned CPython oracle",
    );
}

#[test]
fn a_helper_that_ignores_the_oracle_exit_status_is_rejected() {
    rejects(
        &mutate(
            "assert!(cpython_output.status.success(), \"oracle failed\");",
            "",
        ),
        "never asserts the CPython oracle process succeeded",
    );
}

#[test]
fn a_helper_that_ignores_the_pycc_exit_status_is_rejected() {
    rejects(
        &mutate(
            "assert!(pycc_output.status.success(), \"pycc binary ({label}) failed\");",
            "",
        ),
        "never asserts the compiled pycc binary succeeded",
    );
}

#[test]
fn a_helper_returning_a_single_value_is_rejected() {
    rejects(
        &mutate(
            "(\n        pycc_output.stdout,\n        strip_windows_newline_translation(cpython_output.stdout),\n    )",
            "(pycc_output.stdout)",
        ),
        "must return exactly two elements",
    );
}

#[test]
fn a_helper_that_does_not_end_in_a_tuple_is_rejected() {
    rejects(
        &mutate(
            "(\n        pycc_output.stdout,\n        strip_windows_newline_translation(cpython_output.stdout),\n    )",
            "pycc_output.stdout",
        ),
        "must end in a two-element tuple literal",
    );
}

#[test]
fn a_missing_profile_helper_is_rejected() {
    rejects(
        &mutate(
            "fn run_conformance_fixture_with_profile(",
            "fn renamed_helper(",
        ),
        "`run_conformance_fixture_with_profile` is missing from the harness",
    );
}

#[test]
fn a_missing_plain_helper_is_rejected() {
    rejects(
        &mutate("fn run_conformance_fixture(label", "fn renamed_plain(label"),
        "`run_conformance_fixture` is missing from the harness",
    );
}

#[test]
fn a_plain_helper_that_stops_delegating_is_rejected() {
    rejects(
        &mutate(
            "    run_conformance_fixture_with_profile(label, py_path, false)\n}",
            "    (Vec::new(), Vec::new())\n}",
        ),
        "no longer delegates",
    );
}

#[test]
fn a_harness_without_any_differential_test_is_rejected() {
    rejects(
        &mutate(
            "fn pep_9999_demo_matches_cpython_3_14_7_byte_for_byte()",
            "fn pep_9999_demo_smoke()",
        ),
        "declares no `*_matches_cpython_3_14_7_byte_for_byte` test",
    );
}

#[test]
fn a_deleted_assertion_is_rejected() {
    rejects(
        &mutate(
            "    assert_eq!(\n        release_pycc, release_cpython,\n        \"pycc (--release) and CPython 3.14.7 disagree\"\n    );\n",
            "",
        ),
        "never compares (`release_pycc`, `release_cpython`)",
    );
}

#[test]
fn a_tautological_assertion_is_rejected() {
    rejects(
        &mutate("debug_pycc, debug_cpython,", "debug_pycc, debug_pycc,"),
        "compares `debug_pycc` with itself",
    );
}

#[test]
fn comparing_two_different_runs_pycc_outputs_is_rejected() {
    rejects(
        &mutate("debug_pycc, debug_cpython,", "debug_pycc, release_pycc,"),
        "never compares (`debug_pycc`, `debug_cpython`)",
    );
}

#[test]
fn an_assertion_moved_into_an_unexecuted_path_is_rejected() {
    rejects(
        &mutate(
            "    assert_eq!(\n        debug_pycc, debug_cpython,",
            "    if false {\n    assert_eq!(\n        debug_pycc, debug_cpython,",
        )
        .replacen("a ; inside)\"\n    );", "a ; inside)\"\n    );\n    }", 1),
        "never compares (`debug_pycc`, `debug_cpython`)",
    );
}

#[test]
fn an_assertion_hidden_in_a_comment_is_rejected() {
    rejects(
        &mutate(
            "    assert_eq!(\n        debug_pycc, debug_cpython,",
            "    /*\n    assert_eq!(\n        debug_pycc, debug_cpython,",
        )
        .replacen("a ; inside)\"\n    );", "a ; inside)\"\n    );\n    */", 1),
        "never compares (`debug_pycc`, `debug_cpython`)",
    );
}

#[test]
fn dropping_the_release_profile_is_rejected() {
    rejects(
        &mutate("&fixture, true)", "&fixture, false)"),
        "never exercises the `--release` profile",
    );
}

#[test]
fn dropping_the_debug_profile_is_rejected() {
    rejects(
        &mutate("&fixture, false);", "&fixture, true);"),
        "never exercises the `--debug` profile",
    );
}

#[test]
fn a_new_debug_only_differential_test_is_rejected() {
    rejects(
        &mutate(
            "        run_conformance_fixture_with_profile(\"pep_9999_demo_debug\", &fixture, false);",
            "        run_conformance_fixture(\"pep_9999_demo_debug\", &fixture);",
        )
        .replacen(
            "    let (release_pycc, release_cpython) =\n        run_conformance_fixture_with_profile(\"pep_9999_demo_release\", &fixture, true);",
            "    let (release_pycc, release_cpython) = (Vec::new(), Vec::new());",
            1,
        ),
        "without being listed in DEBUG_ONLY_TESTS",
    );
}

#[test]
fn a_differential_test_that_never_calls_the_harness_is_rejected() {
    rejects(
        &mutate(
            "run_conformance_fixture_with_profile(\"pep_9999_demo_debug\"",
            "unrelated_helper(\"pep_9999_demo_debug\"",
        )
        .replacen(
            "run_conformance_fixture_with_profile(\"pep_9999_demo_release\"",
            "unrelated_helper(\"pep_9999_demo_release\"",
            1,
        ),
        "never binds a `run_conformance_fixture*` result pair",
    );
}

#[test]
fn a_differential_test_without_the_test_attribute_is_rejected() {
    rejects(
        &mutate("#[test]\n#[ignore", "#[ignore"),
        "is missing `#[test]`",
    );
}

#[test]
fn a_differential_test_without_the_ignore_attribute_is_rejected() {
    rejects(
        &mutate(
            "#[ignore = \"requires a pinned python3.14 (CPython 3.14.7) oracle on PATH\"]\n",
            "",
        ),
        "is missing `#[ignore`",
    );
}

#[test]
fn a_swapped_pycc_side_binding_is_rejected() {
    rejects(
        &mutate(
            "let (debug_pycc, debug_cpython)",
            "let (debug_compiled, debug_cpython)",
        ),
        "binds `debug_compiled` where the pycc-side stdout is expected",
    );
}

#[test]
fn a_swapped_oracle_side_binding_is_rejected() {
    rejects(
        &mutate(
            "let (debug_pycc, debug_cpython)",
            "let (debug_pycc, debug_oracle)",
        ),
        "binds `debug_oracle` where the CPython-side stdout is expected",
    );
}

#[test]
fn a_three_element_destructuring_is_not_treated_as_a_harness_call() {
    rejects(
        &mutate(
            "let (debug_pycc, debug_cpython) =\n        run_conformance_fixture_with_profile(\"pep_9999_demo_debug\", &fixture, false);",
            "let (debug_pycc, debug_cpython, extra) =\n        run_conformance_fixture_with_profile(\"pep_9999_demo_debug\", &fixture, false);",
        ),
        "never exercises the `--debug` profile",
    );
}

#[test]
fn a_destructuring_without_an_assignment_is_not_treated_as_a_harness_call() {
    rejects(
        &mutate(
            "let (debug_pycc, debug_cpython) =\n        run_conformance_fixture_with_profile(\"pep_9999_demo_debug\", &fixture, false);",
            "let (debug_pycc, debug_cpython);\n        run_conformance_fixture_with_profile(\"pep_9999_demo_debug\", &fixture, false);",
        ),
        "never exercises the `--debug` profile",
    );
}

#[test]
fn a_pair_bound_from_something_other_than_a_call_is_not_treated_as_a_harness_call() {
    rejects(
        &mutate(
            "run_conformance_fixture_with_profile(\"pep_9999_demo_debug\", &fixture, false)",
            "run_conformance_fixture_with_profile_result",
        ),
        "never exercises the `--debug` profile",
    );
}

#[test]
fn a_profile_helper_call_without_arguments_is_rejected() {
    rejects(
        &mutate(
            "run_conformance_fixture_with_profile(\"pep_9999_demo_debug\", &fixture, false)",
            "run_conformance_fixture_with_profile()",
        ),
        "never exercises the `--debug` profile",
    );
}

#[test]
fn the_real_harness_rejects_the_issue_224_helper_mutation() {
    let mutated = harness().replacen(
        "    (\n        pycc_output.stdout,\n        strip_windows_newline_translation(cpython_output.stdout),\n    )",
        "    (pycc_output.stdout.clone(), pycc_output.stdout)",
        1,
    );
    assert_ne!(
        mutated,
        harness(),
        "the mutation did not apply to the conformance harness sources"
    );
    rejects(
        &mutated,
        "second element must derive from the CPython oracle alone",
    );
}

#[test]
fn the_real_harness_rejects_a_tautological_per_pep_assertion() {
    let mutated = harness().replacen(
        "debug_pycc, debug_cpython,\n        \"pycc (--debug)",
        "debug_pycc, debug_pycc,\n        \"pycc (--debug)",
        1,
    );
    assert_ne!(
        mutated,
        harness(),
        "the mutation did not apply to the conformance harness sources"
    );
    rejects(&mutated, "compares `debug_pycc` with itself");
}

// --- Unit controls for the text scanners -----------------------------------

#[test]
fn strip_comments_removes_line_and_block_comments_but_keeps_strings() {
    let stripped = strip_comments(
        "let a = \"// not a comment\"; // gone\n/* also /* nested */ gone */let b = 1;\n",
    );
    assert!(stripped.contains("\"// not a comment\""));
    assert!(!stripped.contains("gone"));
    assert!(stripped.contains("let b = 1;"));
}

#[test]
fn end_of_string_tolerates_an_unterminated_literal() {
    let chars: Vec<char> = "\"unterminated \\\" still open".chars().collect();
    assert_eq!(end_of_string(&chars, 0), chars.len());
}

#[test]
fn split_top_level_ignores_separators_inside_nesting_and_strings() {
    let chars: Vec<char> = "a, f(b, c), \"d, e\", [g, h], {i, j},".chars().collect();
    assert_eq!(
        split_top_level(&chars, ','),
        vec!["a", "f(b, c)", "\"d, e\"", "[g, h]", "{i, j}"]
    );
}

#[test]
fn parse_fns_reads_names_attributes_and_bodies() {
    let fns = parse_fns("#[test]\nfn alpha() {\n    let x = \"}\";\n}\nfn beta() {}\n");
    assert_eq!(fns.len(), 2);
    assert_eq!(fns[0].name, "alpha");
    assert_eq!(fns[0].attrs, "#[test]");
    assert!(fns[0].body.contains("let x = \"}\";"));
    assert_eq!(fns[1].name, "beta");
    assert_eq!(fns[1].attrs, "");
}

#[test]
fn brace_depth_counts_nesting_and_ignores_braces_in_strings() {
    let chars: Vec<char> = "{ \"{{\" { X } }".chars().collect();
    let index = chars.iter().position(|c| *c == 'X').unwrap();
    assert_eq!(brace_depth(&chars, index), 2);
}
