use std::path::Path;
use std::process::Command;

fn pycc_bin() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_BIN_EXE_pycc"))
}

/// `pycc check` embeds whatever `path` string it was invoked with verbatim
/// into its diagnostic output (`render_human`'s ` --> {file_path}:...`
/// line, `render_json`'s `"file"` field). If this harness passed an
/// absolute path (e.g. one built from `CARGO_MANIFEST_DIR`), the checked-in
/// `.expected.txt` fixtures would bake in a machine-specific checkout path,
/// failing on every other machine and violating DIAGNOSTICS.md's
/// byte-identical-across-platforms bar. Instead, `pycc` is invoked with its
/// `current_dir` set to the repo root and a repo-relative, forward-slash
/// path literal (stable on every OS -- this exact string is what gets
/// embedded, not a `PathBuf`'s platform-dependent `Display`).
fn assert_diagnostic_matches_fixture(fixture_stem: &str) {
    let repo_root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let expected_path = repo_root
        .join("tests/diagnostics")
        .join(format!("{fixture_stem}.expected.txt"));
    // pycc's diagnostic renderer always emits `\n` line endings, matching
    // DIAGNOSTICS.md's byte-identical-across-platforms bar. Git on Windows
    // checks these fixtures out with `\r\n` under the default `core.autocrlf`
    // text conversion, so the raw file bytes must be normalized before
    // comparison rather than compared as checked out.
    let expected = std::fs::read_to_string(&expected_path)
        .unwrap_or_else(|e| panic!("could not read {}: {e}", expected_path.display()))
        .replace("\r\n", "\n");

    let relative_py_path = format!("tests/diagnostics/{fixture_stem}.py");
    let output = Command::new(pycc_bin())
        .args(["check", &relative_py_path])
        .current_dir(repo_root)
        .output()
        .unwrap();
    let actual = String::from_utf8_lossy(&output.stdout);
    assert_eq!(
        actual, expected,
        "diagnostic output for {fixture_stem} did not match its .expected.txt fixture"
    );
    assert_eq!(
        output.status.code(),
        Some(1),
        "{fixture_stem} should be a compile error"
    );
}

fn assert_json_diagnostic_matches_fixture(fixture_stem: &str) {
    let repo_root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let expected_path = repo_root
        .join("tests/diagnostics")
        .join(format!("{fixture_stem}.expected.json"));
    let expected = std::fs::read_to_string(&expected_path)
        .unwrap_or_else(|e| panic!("could not read {}: {e}", expected_path.display()))
        .replace("\r\n", "\n");

    let relative_py_path = format!("tests/diagnostics/{fixture_stem}.py");
    let output = Command::new(pycc_bin())
        .args(["check", &relative_py_path, "--error-format", "json"])
        .current_dir(repo_root)
        .output()
        .unwrap();
    let actual = String::from_utf8_lossy(&output.stdout);
    assert_eq!(
        actual, expected,
        "JSON diagnostic output for {fixture_stem} did not match its .expected.json fixture"
    );
    assert_eq!(
        output.status.code(),
        Some(1),
        "{fixture_stem} should be a compile error"
    );
}

/// #1224: run `pycc check tests/diagnostics/<source>.py <extra_args>` and
/// compare its human and JSON output with `<expected_stem>.expected.txt`
/// and `<expected_stem>.expected.json`. Unlike the one-stem helpers above,
/// one source can carry several snapshots (one per flag set), and a source
/// may sit in a subdirectory that holds its own `pycc.toml`.
fn assert_check_snapshots(source: &str, expected_stem: &str, extra_args: &[&str]) {
    let repo_root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let relative_py_path = format!("tests/diagnostics/{source}.py");
    for (json, extension) in [(false, "txt"), (true, "json")] {
        let expected_path = repo_root
            .join("tests/diagnostics")
            .join(format!("{expected_stem}.expected.{extension}"));
        let expected = std::fs::read_to_string(&expected_path)
            .unwrap_or_else(|e| panic!("could not read {}: {e}", expected_path.display()))
            .replace("\r\n", "\n");
        let mut command = Command::new(pycc_bin());
        command
            .args(["check", &relative_py_path])
            .args(extra_args)
            .current_dir(repo_root);
        if json {
            command.args(["--error-format", "json"]);
        }
        let output = command.output().unwrap();
        let actual = String::from_utf8_lossy(&output.stdout);
        assert_eq!(
            actual, expected,
            "{extension} output for {source} {extra_args:?} did not match {expected_stem}"
        );
        assert_eq!(
            output.status.code(),
            Some(1),
            "{source} {extra_args:?} should be a compile error"
        );
    }
}

#[test]
fn c0001_unsupported_valid_python() {
    assert_diagnostic_matches_fixture("c0001_unsupported_valid_python");
}

// Issue #141 / D-148: `break`/`continue` and `async for` are misclassified
// as `C0001` ("valid Python, not implemented yet") when CPython actually
// rejects all three as a `SyntaxError` -- a context-invalidity failure, not
// a capability gap -- unless a real enclosing loop makes break/continue
// genuinely valid-but-unimplemented. These are not byte-for-byte
// CPython-oracle-diffed (per D-138's own precedent): CPython raises
// `SyntaxError`, pycc emits a structured diagnostic -- different failure
// kinds, not meaningfully comparable byte-for-byte. Each asserts pycc's own
// diagnostic output against its fixture.
#[test]
fn l0001_break_outside_loop() {
    assert_diagnostic_matches_fixture("l0001_break_outside_loop");
}

// PEP 758 (Part 3 of #543, #740): `except A, B as e:` (bare comma, no
// parens, with an `as` binding) is rejected by the *parser*, not by HIR/
// types/MIR -- CPython requires parentheses around a multi-type handler's
// exception list whenever it also binds a name. This is existing,
// already-correct parser behavior; the fixture is a regression guard.
#[test]
fn l0001_except_multi_type_as_binding_requires_parens() {
    assert_diagnostic_matches_fixture("l0001_except_multi_type_as_binding_requires_parens");
}

// #864 Part 1 (D-217): a file with several syntax errors reports every one
// of them, in ruff's discovery order, as concatenated human renders (no
// separator) and as one JSON object per line. `def main(:\n` yields exactly
// two ruff errors (the malformed parameter list, then the trailing
// `unexpected EOF` recovery cascade, reported verbatim). The first render in
// both fixtures is byte-identical to what the pre-#864 single-diagnostic
// `check` printed for the same file.
#[test]
fn l0001_two_syntax_errors() {
    assert_diagnostic_matches_fixture("l0001_two_syntax_errors");
    assert_json_diagnostic_matches_fixture("l0001_two_syntax_errors");
}

// #864's own reproduction, regenerated for Part 2 (#867, D-219): HIR
// lowering now collects one diagnostic per failing top-level item, so both
// class-body `C0001`s (`2:5` and `4:5`) are reported, in source order. The
// first render is byte-identical to the Part 1 fixture (D-217 rule 2). The
// type error on line 6 is the type checker's, which still does not run
// after an HIR failure.
#[test]
fn c0001_issue_864_repro() {
    assert_diagnostic_matches_fixture("c0001_issue_864_repro");
    assert_json_diagnostic_matches_fixture("c0001_issue_864_repro");
}

// #867 (D-219) cascade suppression: an unsupported `import os.path as o`, a
// `class A`, and a later genuine `*args` gap are the three reported
// `C0001`s; `class B(A)` (unknown base) and `def g(a: A)` (unknown
// annotation) name the skipped `A` and are skipped silently -- no render
// of any kind between the line-3 and line-9 diagnostics.
//
// The import carries an `as` alias only since Part 1 of #1026: a plain
// `import os` now binds a CPython module object instead of reaching the
// `C0001` catch-all, and this fixture needs a rejected first item for the
// cascade to be a cascade at all. #1291 then admitted the undotted
// `import os as o` as a CPython module bound to `o`, so the rejected
// first item became the dotted `import os.path as o`. #1381 (Part 3 of
// #1138) admitted that too, so the rejected first item is now
// `import os.path as os`, which binds a submodule under its own root's
// name and keeps a `C0001`.
#[test]
fn c0001_hir_cascade_suppressed() {
    assert_diagnostic_matches_fixture("c0001_hir_cascade_suppressed");
    assert_json_diagnostic_matches_fixture("c0001_hir_cascade_suppressed");
}

// PEP 758 (Part 3 of #543, #740): `except ():` parses successfully as an
// empty-tuple exception type, but names zero exception types -- rejected
// at HIR lowering rather than reaching MIR/codegen's non-empty
// handler-tag-set invariant unchecked.
#[test]
fn c0001_except_empty_tuple_type() {
    assert_diagnostic_matches_fixture("c0001_except_empty_tuple_type");
}

#[test]
fn l0001_continue_outside_loop() {
    assert_diagnostic_matches_fixture("l0001_continue_outside_loop");
}

#[test]
fn l0001_async_for_outside_async_function() {
    assert_diagnostic_matches_fixture("l0001_async_for_outside_async_function");
}

/// Paired with the `l0001_*` fixtures above: a real enclosing loop keeps
/// break/continue on the existing valid-but-unimplemented `C0001` path --
/// this issue is scoped to classification, not to implementing loop control
/// flow.
#[test]
fn c0001_break_inside_loop() {
    assert_diagnostic_matches_fixture("c0001_break_inside_loop");
}

#[test]
fn c0001_continue_inside_loop() {
    assert_diagnostic_matches_fixture("c0001_continue_inside_loop");
}

// Issue #361 / D-149, the expression-lowering sequel to #141/D-148:
// `yield`/`yield from` outside any function are misclassified as `C0001`
// ("valid Python, not implemented yet") when CPython actually rejects both
// as a `SyntaxError` -- a context-invalidity failure, not a capability gap --
// unless a real enclosing function makes them genuinely
// valid-but-unimplemented (generator codegen itself remains out of scope).
#[test]
fn l0001_yield_outside_function() {
    assert_diagnostic_matches_fixture("l0001_yield_outside_function");
}

// `from __future__ import ...` (#919, D-229): the two shapes CPython 3.14
// rejects as a `SyntaxError` are `L0001` with CPython's own wording; the two
// it accepts but pycc does not model are `C0001` capability gaps. The
// accepted no-op shapes are exercised end to end by
// `tests/issue_919_future_import.rs`.
#[test]
fn l0001_future_feature_not_defined() {
    assert_diagnostic_matches_fixture("l0001_future_feature_not_defined");
}

#[test]
fn l0001_future_import_not_at_beginning() {
    assert_diagnostic_matches_fixture("l0001_future_import_not_at_beginning");
}

#[test]
fn c0001_future_barry_as_flufl() {
    assert_diagnostic_matches_fixture("c0001_future_barry_as_flufl");
}

#[test]
fn c0001_future_import_alias() {
    assert_diagnostic_matches_fixture("c0001_future_import_alias");
}

#[test]
fn l0001_yield_from_outside_function() {
    assert_diagnostic_matches_fixture("l0001_yield_from_outside_function");
}

/// Paired with the `l0001_*` fixtures above: a real enclosing function keeps
/// `yield`/`yield from` on the existing valid-but-unimplemented `C0001` path
/// -- this issue is scoped to classification, not to implementing generator
/// codegen.
#[test]
fn c0001_yield_inside_function() {
    assert_diagnostic_matches_fixture("c0001_yield_inside_function");
}

#[test]
fn c0001_yield_from_inside_function() {
    assert_diagnostic_matches_fixture("c0001_yield_from_inside_function");
}

// Issue #738 / Part 1 of #543, PEP 765: `return`/`break`/`continue` that
// would exit a `finally` block are rejected with a dedicated `L0001`
// context-invalidity diagnostic, distinct from the plain "outside loop"/
// "outside function" family above -- CPython 3.14 emits a `SyntaxWarning`
// for the same construct (not yet a hard `SyntaxError`), so this is not
// byte-for-byte CPython-oracle-diffed either, matching the D-148/D-149
// precedent's own rationale. Each fixture wraps the offending statement in
// a valid escape target (an enclosing function for `return`, an enclosing
// loop for `break`/`continue`) -- verified directly against
// `python3.14 -W all` to be the scenario where CPython's own fatal error
// actually becomes the finally-specific one; see the sibling
// `*_with_no_enclosing_*` and `d0024_return_inside_finally_with_no_enclosing_function`
// fixtures below for the complementary case where no valid target exists at
// all and pycc instead defers to its pre-existing diagnostics.
#[test]
fn l0001_return_inside_finally() {
    assert_diagnostic_matches_fixture("l0001_return_inside_finally");
}

#[test]
fn l0001_break_inside_finally() {
    assert_diagnostic_matches_fixture("l0001_break_inside_finally");
}

#[test]
fn l0001_continue_inside_finally() {
    assert_diagnostic_matches_fixture("l0001_continue_inside_finally");
}

// #795 (PEP 654): CPython rejects `return`, `break`, and `continue` inside
// an `except*` clause body outright -- `SyntaxError: 'break', 'continue' and
// 'return' cannot appear in an except* block`, verified directly against
// CPython 3.14.6. `L0001` is reused for this post-parse context violation,
// matching the PEP 765 `finally` family above. The `return` case is the only
// genuine accept-to-reject change: `break`/`continue` in an `except*` clause
// were already rejected, only with the wrong diagnostic (`C0001` "inside a
// loop", or `L0001` "outside loop"), so these fixtures pin the new message
// rather than a new rejection.
#[test]
fn l0001_return_in_except_star() {
    assert_diagnostic_matches_fixture("l0001_return_in_except_star");
}

// An intervening loop inside the `except*` clause body does NOT shield a
// `return` (it does shield `break`/`continue` -- that direction has no
// public-CLI fixture, because the shielded form simply compiles; it is
// covered by `crates/pycc_hir/src/stmt/exception.rs`'s
// `except_star_context_tests` instead). Verified
// against CPython 3.14.6: `for i in range(3): return 1` inside an `except*`
// clause is still a `SyntaxError`, while the same loop containing `break`
// compiles.
#[test]
fn l0001_return_in_loop_in_except_star() {
    assert_diagnostic_matches_fixture("l0001_return_in_loop_in_except_star");
}

// Unlike the PEP 765 `finally` family, this check has no "valid escape
// target" precondition for `break`/`continue`: CPython reports the `except*`
// error even with no enclosing loop anywhere, where `'break' outside loop`
// would otherwise apply.
#[test]
fn l0001_break_in_except_star() {
    assert_diagnostic_matches_fixture("l0001_break_in_except_star");
}

#[test]
fn l0001_continue_in_except_star() {
    assert_diagnostic_matches_fixture("l0001_continue_in_except_star");
}

// A real enclosing loop *outside* the `try`/`except*` does not shield the
// `break` either -- only a loop entered within the clause body does.
#[test]
fn l0001_break_in_except_star_inside_loop() {
    assert_diagnostic_matches_fixture("l0001_break_in_except_star_inside_loop");
}

// A `finally` nested inside an `except*` clause body propagates the `except*`
// context rather than clearing it, and the `except*` message wins over the
// PEP 765 `finally` message -- matching CPython's own precedence, where the
// `except*` failure is the fatal `SyntaxError` and the `finally` restriction
// is only a `SyntaxWarning`.
#[test]
fn l0001_break_in_except_star_in_finally() {
    assert_diagnostic_matches_fixture("l0001_break_in_except_star_in_finally");
}

// Companion to `l0001_return_in_except_star`: with NO enclosing function at
// all, CPython's actual fatal error is the pre-existing `SyntaxError:
// 'return' outside function`, not the `except*` message. pycc mirrors that
// precedence with the `in_function` conjunct on the `except*` `return`
// guard, deferring to this pre-existing `T0024`.
#[test]
fn d0024_return_in_except_star_at_module_level() {
    assert_diagnostic_matches_fixture("d0024_return_in_except_star_at_module_level");
}

// #795 (PEP 654), gap 2: CPython *accepts* `except* ExceptionGroup:` at
// compile time and raises `TypeError: catching ExceptionGroup with except*
// is not allowed. Use except instead.` at handler-match time. pycc cannot
// raise that yet (D-173 keeps no materialized group value to type-test), so
// it rejects the program at compile time with `C0001` -- a deliberate,
// documented divergence recorded in the ADR that narrows D-202. #903 tracks
// delivering the real runtime behavior.
#[test]
fn c0001_except_star_exception_group() {
    assert_diagnostic_matches_fixture("c0001_except_star_exception_group");
}

// The same rejection for `BaseExceptionGroup`, and in a *non-first* position
// of a PEP 758 multi-type handler -- proving the check runs per element of
// the tuple rather than only on the first name.
#[test]
fn c0001_except_star_base_exception_group_in_tuple() {
    assert_diagnostic_matches_fixture("c0001_except_star_base_exception_group_in_tuple");
}

// #795, second round (D-068 review of the first): CPython's runtime
// `TypeError` fires for any class whose MRO reaches `BaseExceptionGroup`, so
// a user-defined subclass of a group class is refused at compile time too.
// Without this the compiler would silently lower `except* G:` into ordinary
// tag matching -- the wrong-runtime-answer outcome the divergence exists to
// prevent.
#[test]
fn c0001_except_star_exception_group_subclass() {
    assert_diagnostic_matches_fixture("c0001_except_star_exception_group_subclass");
}

// Companion to the three fixtures above: with NO valid escape target
// anywhere (no enclosing loop at all), CPython's actual fatal error for a
// `break`/`continue` directly in a `finally` is the pre-existing "outside
// loop"/"not properly in loop" `SyntaxError`, not the finally-specific
// `SyntaxWarning` (verified against `python3.14 -W all`: the finally
// warning prints too, but does not by itself fail compilation). pycc
// mirrors that precedence by falling through to its own pre-existing
// `in_loop`-driven `L0001` handling rather than reporting the
// finally-specific message.
#[test]
fn l0001_break_inside_finally_with_no_enclosing_loop() {
    assert_diagnostic_matches_fixture("l0001_break_inside_finally_with_no_enclosing_loop");
}

#[test]
fn l0001_continue_inside_finally_with_no_enclosing_loop() {
    assert_diagnostic_matches_fixture("l0001_continue_inside_finally_with_no_enclosing_loop");
}

#[test]
fn d0001_missing_public_annotation() {
    assert_diagnostic_matches_fixture("d0001_missing_public_annotation");
}

#[test]
fn d0002_any_forbidden() {
    assert_diagnostic_matches_fixture("d0002_any_forbidden");
}

#[test]
fn d0021_range_argument_type() {
    assert_diagnostic_matches_fixture("d0021_range_argument_type");
    assert_json_diagnostic_matches_fixture("d0021_range_argument_type");
}

#[test]
fn d0021_unbound_local() {
    assert_diagnostic_matches_fixture("d0021_unbound_local");
    assert_json_diagnostic_matches_fixture("d0021_unbound_local");
}

#[test]
fn d0022_missing_return() {
    assert_diagnostic_matches_fixture("d0022_missing_return");
}

#[test]
fn d0023_incompatible_assignment() {
    assert_diagnostic_matches_fixture("d0023_incompatible_assignment");
}

#[test]
fn d0024_return_outside_function() {
    assert_diagnostic_matches_fixture("d0024_return_outside_function");
}

// Companion to the `l0001_return_inside_finally`/PEP 765 fixtures above:
// with NO enclosing function at all, CPython's actual fatal error for a
// `return` directly in a `finally` is the pre-existing "outside function"
// `SyntaxError`, not the finally-specific `SyntaxWarning` (verified against
// `python3.14 -W all`). pycc mirrors that precedence by letting HIR
// lowering succeed and deferring to this pre-existing `T0024` type-check
// path rather than reporting the finally-specific `L0001` message.
#[test]
fn d0024_return_inside_finally_with_no_enclosing_function() {
    assert_diagnostic_matches_fixture("d0024_return_inside_finally_with_no_enclosing_function");
}

#[test]
fn d0025_annotated_assignment_mismatch() {
    assert_diagnostic_matches_fixture("d0025_annotated_assignment_mismatch");
}

#[test]
fn d0026_annotation_only_unbound() {
    assert_diagnostic_matches_fixture("d0026_annotation_only_unbound");
}

// #133: a module-level value binding shadows builtin and user-function call
// lookup at every later call site. The four fixtures cover the issue's
// completion criteria through the public `pycc check` path: a user-function
// shadow, a builtin shadow, a call inside an annotated function body (checked
// against the final module environment, D-041), and the private-helper
// inference path.
#[test]
fn d0027_module_value_shadows_function() {
    assert_diagnostic_matches_fixture("d0027_module_value_shadows_function");
}

#[test]
fn d0028_module_value_shadows_builtin() {
    assert_diagnostic_matches_fixture("d0028_module_value_shadows_builtin");
}

#[test]
fn d0029_function_body_calls_shadowed_name() {
    assert_diagnostic_matches_fixture("d0029_function_body_calls_shadowed_name");
}

#[test]
fn d0030_private_helper_calls_shadowed_name() {
    assert_diagnostic_matches_fixture("d0030_private_helper_calls_shadowed_name");
}

// D-110: the shadowed-builtin shape inside an inference-signature helper,
// pinned through the public CLI. The mirror gate fires before the solver's
// `print` special case; pass 3 would independently reject this shape too, so
// this fixture pins behavior rather than discriminating the mirror (the
// discriminating case lives in pycc_types' own unit tests).
#[test]
fn d0031_private_helper_calls_shadowed_builtin() {
    assert_diagnostic_matches_fixture("d0031_private_helper_calls_shadowed_builtin");
}

#[test]
fn d0032_heterogeneous_list_literal() {
    assert_diagnostic_matches_fixture("d0032_heterogeneous_list_literal");
}

#[test]
fn d0033_subscript_on_non_list() {
    assert_diagnostic_matches_fixture("d0033_subscript_on_non_list");
}

#[test]
fn d0034_list_element_type_not_int() {
    assert_diagnostic_matches_fixture("d0034_list_element_type_not_int");
}

#[test]
fn d0035_heterogeneous_dict_literal() {
    assert_diagnostic_matches_fixture("d0035_heterogeneous_dict_literal");
}

#[test]
fn d0036_dict_key_value_type_not_str_int() {
    assert_diagnostic_matches_fixture("d0036_dict_key_value_type_not_str_int");
}

#[test]
fn d0037_heterogeneous_set_literal() {
    assert_diagnostic_matches_fixture("d0037_heterogeneous_set_literal");
}

#[test]
fn d0038_set_element_type_not_int() {
    assert_diagnostic_matches_fixture("d0038_set_element_type_not_int");
}

#[test]
fn d0039_tuple_element_type_not_int_bool_float() {
    assert_diagnostic_matches_fixture("d0039_tuple_element_type_not_int_bool_float");
}

#[test]
fn d0040_tuple_index_not_a_literal() {
    assert_diagnostic_matches_fixture("d0040_tuple_index_not_a_literal");
}

// PR #827 review finding: an oversized literal index into a *tuple* base
// must keep reporting T0040 ("non-negative literal within range"), not
// T0051 ("out of range for a list index") -- tuple indexing is resolved
// entirely at compile time in `pycc_types`, with no D-141 runtime `int`
// boundary at all, so `crates/pycc_hir/src/expr.rs`'s subscript-lowering
// arm must defer to this check for a tuple-literal base instead of
// preempting it. See `crates/pycc_hir/src/expr.rs`'s
// `boundary_tuple_literal_index_is_not_t0051` unit test for the
// HIR-lowering-level half of this coverage.
#[test]
fn d0040_tuple_index_oversized_literal_still_t0040() {
    assert_diagnostic_matches_fixture("d0040_tuple_index_oversized_literal_still_t0040");
}

#[test]
fn d0041_value_less_annotation_later_assignment_mismatch() {
    assert_diagnostic_matches_fixture("d0041_value_less_annotation_later_assignment_mismatch");
}

#[test]
fn t0041_maybe_bound_if_no_else() {
    assert_diagnostic_matches_fixture("t0041_maybe_bound_if_no_else");
}

#[test]
fn t0041_maybe_bound_while() {
    assert_diagnostic_matches_fixture("t0041_maybe_bound_while");
}

#[test]
fn t0041_maybe_bound_for_range() {
    assert_diagnostic_matches_fixture("t0041_maybe_bound_for_range");
}

// #1289: a `try` statement's fall-through join. A name the body binds but a
// falling-through handler does not stays possibly unbound; `finally` is
// checked against the conservative state because it also runs on the paths
// that leave early; an `as` name is unbound after its handler (CPython's
// implicit `del`); and every path's binding is type-checked, including a
// handler that always terminates and the `else` path against a handler.
#[test]
fn t0041_maybe_bound_try_handler() {
    assert_diagnostic_matches_fixture("t0041_maybe_bound_try_handler");
}

#[test]
fn t0041_maybe_bound_try_finally_read() {
    assert_diagnostic_matches_fixture("t0041_maybe_bound_try_finally_read");
}

#[test]
fn t0041_try_as_name_after_handler() {
    assert_diagnostic_matches_fixture("t0041_try_as_name_after_handler");
}

#[test]
fn t0041_try_as_name_terminating_handler() {
    assert_diagnostic_matches_fixture("t0041_try_as_name_terminating_handler");
}

#[test]
fn t0023_try_as_name_rebound_after() {
    assert_diagnostic_matches_fixture("t0023_try_as_name_rebound_after");
}

#[test]
fn t0023_try_handler_else_mismatch() {
    assert_diagnostic_matches_fixture("t0023_try_handler_else_mismatch");
}

#[test]
fn t0023_try_terminating_handler_mismatch() {
    assert_diagnostic_matches_fixture("t0023_try_terminating_handler_mismatch");
}

// PEP 591 (#383): reassigning a `Final`-annotated name after its initial
// binding is T0045. `T0045` covers the variable-level position only
// (module-level and function-local annotated assignments); `Final` on a
// function parameter is still out of scope. A class-body `Final` attribute
// (#916) is accepted but never reaches `T0045`: it produces no
// `HirStmt::AnnAssign`, so `Environment.finals` never learns about it, and
// every write to it is already rejected with `T0044`.
#[test]
fn t0045_final_reassignment() {
    assert_diagnostic_matches_fixture("t0045_final_reassignment");
}

// Issue #868 (Part 3 of #864, D-220): the type checker reports one
// diagnostic per failing function. Report order is the solver's body walk
// (item order) first -- `f`'s T0022 and `h`'s T0021 -- then the concrete
// annotation checker's entries for functions the solver did not flag --
// `g`'s T0043 -- so the order is `f, h, g`, not source order. The first
// render is byte-identical to the pre-#868 single diagnostic (D-217 rule
// 2). Every `pycc_types` diagnostic still carries an empty span, so all
// three render at `:1:1` (D-043).
#[test]
fn t0022_types_per_function() {
    assert_diagnostic_matches_fixture("t0022_types_per_function");
    assert_json_diagnostic_matches_fixture("t0022_types_per_function");
}

// Issue #949: the private-helper solver runs over every module-level
// function, so its `T0022` return conflict used to be worded "private helper
// return type" even for an annotated public function. The discriminator is
// the return term, not the function's visibility: a written annotation is a
// declared/actual mismatch, an unannotated helper's return is a genuinely
// inferred one. This fixture pins the declared wording for the issue's own
// program; `t0022_types_per_function` above pins it for the plain-`str` case,
// and `constraints`' in-crate tests pin the unchanged inferred wording.
#[test]
fn t0022_declared_return_protocol_param() {
    assert_diagnostic_matches_fixture("t0022_declared_return_protocol_param");
}

// Issue #618: an out-of-range `int` literal in a runtime `int`-boundary
// position (D-141) is rejected at compile time with a spanned T0051
// diagnostic instead of reaching `pycc_rt_int_untag_checked` and aborting at
// run time (D-178's own knowingly-deferred consequence). This fixture
// exercises the "list index" position; every other named position is
// exercised without going through the CLI's exact `--error-format human`
// rendering, by the position-specific unit tests in
// `crates/pycc_hir/src/tests.rs` and `crates/pycc_hir/src/stmt.rs`'s own
// test module, which is what D-014's coverage gate actually needs -- this
// fixture's job is only to prove the diagnostic reaches the real `pycc
// check` CLI end to end with a correct rendered span, not to duplicate all
// 13 positions through the slower CLI harness.
#[test]
fn t0051_int_literal_boundary_list_index() {
    assert_diagnostic_matches_fixture("t0051_int_literal_boundary_list_index");
}

#[test]
fn d0021_float_wrong_arity() {
    assert_diagnostic_matches_fixture("d0021_float_wrong_arity");
}

#[test]
fn d0021_float_non_numeric_argument() {
    assert_diagnostic_matches_fixture("d0021_float_non_numeric_argument");
}

#[test]
fn cli_spec_example() {
    assert_diagnostic_matches_fixture("cli_spec_example");
}

// PR-12 Task 12 (D-119): comprehensions, slicing, and the new
// `.pop()`/`.get()`/`.add()` container methods mint zero new diagnostic
// codes -- every rejection below reuses T0021/T0033/T0034/T0036/T0038 or
// the existing C0001 capability catch-all. These fixtures pin the new
// *rejection paths* through the public `pycc check` CLI, one per genuinely
// new way to reach an already-registered code.

#[test]
fn d0033_slice_on_non_list() {
    assert_diagnostic_matches_fixture("d0033_slice_on_non_list");
}

#[test]
fn d0021_slice_bound_not_int() {
    assert_diagnostic_matches_fixture("d0021_slice_bound_not_int");
}

#[test]
fn d0033_pop_on_non_list() {
    assert_diagnostic_matches_fixture("d0033_pop_on_non_list");
}

// `.pop()` with an argument is an arity mismatch caught during HIR
// lowering, before `pycc_types` ever runs -- it reports through the
// generic C0001 capability path, not T0033, exactly like `.append()`'s
// own pre-existing arity check.
#[test]
fn c0001_list_pop_with_argument() {
    assert_diagnostic_matches_fixture("c0001_list_pop_with_argument");
}

#[test]
fn d0033_get_on_non_dict() {
    assert_diagnostic_matches_fixture("d0033_get_on_non_dict");
}

#[test]
fn d0021_dict_get_key_type_mismatch() {
    assert_diagnostic_matches_fixture("d0021_dict_get_key_type_mismatch");
}

#[test]
fn d0021_dict_get_default_type_mismatch() {
    assert_diagnostic_matches_fixture("d0021_dict_get_default_type_mismatch");
}

// Same HIR-lowering-time arity shape as `.pop()` above -- C0001, not T0033.
#[test]
fn c0001_dict_get_wrong_argument_count() {
    assert_diagnostic_matches_fixture("c0001_dict_get_wrong_argument_count");
}

#[test]
fn d0033_add_on_non_set() {
    assert_diagnostic_matches_fixture("d0033_add_on_non_set");
}

#[test]
fn d0021_set_add_value_type_mismatch() {
    assert_diagnostic_matches_fixture("d0021_set_add_value_type_mismatch");
}

#[test]
fn c0001_comprehension_two_for_clauses() {
    assert_diagnostic_matches_fixture("c0001_comprehension_two_for_clauses");
}

#[test]
fn c0001_comprehension_two_if_filters() {
    assert_diagnostic_matches_fixture("c0001_comprehension_two_if_filters");
}

// #1254 (D-250): a comprehension lowers in any expression position now, so
// the former call-argument refusal is gone; a walrus inside one is refused
// in both the expression and the statement form, spanning the comprehension.
#[test]
fn c0001_comprehension_walrus_in_expression() {
    assert_diagnostic_matches_fixture("c0001_comprehension_walrus_in_expression");
}

#[test]
fn c0001_comprehension_walrus_in_assignment() {
    assert_diagnostic_matches_fixture("c0001_comprehension_walrus_in_assignment");
}

// #1254: the one-`for`-clause limit applies to the expression form too.
#[test]
fn c0001_comprehension_expression_two_for_clauses() {
    assert_diagnostic_matches_fixture("c0001_comprehension_expression_two_for_clauses");
}

// #1254: the expression form resolves its iterable exactly like `for`.
#[test]
fn t0033_comprehension_expression_str_iterable() {
    assert_diagnostic_matches_fixture("t0033_comprehension_expression_str_iterable");
}

// #1254: an inner comprehension's iterable naming the outer loop variable
// resolves to that variable (an `int`), not to the module-level list.
#[test]
fn t0033_comprehension_nested_iterable_names_outer_variable() {
    assert_diagnostic_matches_fixture("t0033_comprehension_nested_iterable_names_outer_variable");
}

#[test]
fn d0034_comprehension_list_str() {
    assert_diagnostic_matches_fixture("d0034_comprehension_list_str");
}

#[test]
fn d0036_comprehension_dict_str_str() {
    assert_diagnostic_matches_fixture("d0036_comprehension_dict_str_str");
}

#[test]
fn d0038_comprehension_set_str() {
    assert_diagnostic_matches_fixture("d0038_comprehension_set_str");
}

// Real Python's dict-comprehension grammar has no `**`-unpacking form the
// way a plain dict literal does, but the vendored parser accepts
// `{**x for k in y}` anyway (silently dropping the `**`) rather than
// rejecting it at parse time -- so this is real, parseable Python that
// reaches a dedicated C0001 capability diagnostic, not an internal panic.
#[test]
fn c0001_dict_comprehension_unpacking() {
    assert_diagnostic_matches_fixture("c0001_dict_comprehension_unpacking");
}

// PR-14 (D-136/D-137): an unrecognized stdlib/third-party module name
// (`cgi`, `os`, `typing`, ...) fails closed with the same generic C0001
// catch-all every other unimplemented statement shape uses -- not a
// dedicated per-module message.
//
// Part 1 of #1026 narrowed which shapes reach that catch-all: a plain
// `import cgi` binds a CPython module object now, so what is pinned here
// was the aliased `import cgi as c` until #1291 admitted an undotted
// alias on the same terms; what was pinned next was the dotted
// `import cgi.x as c`, until #1381 (Part 3 of #1138) admitted a plain
// dotted import of a CPython module too. No plain import of a
// standard-library or third-party name reaches the catch-all any more, so
// what is pinned now is the one plain shape that still does: a dotted name
// under a project module (here the fixture's own module), which is not
// foreign and which project imports do not resolve yet. The undotted
// form is accepted by `pycc check` and, since `cgi` left the standard
// library in 3.13, built by a plain `pycc build` like any
// non-standard-library root only from the program's `pycc.lock` closure,
// and refused with exit 2 naming `pycc lock` without one -- pinned in
// `tests/issue_1242_locked_closure.rs`.
#[test]
fn c0001_import_unrecognized_module() {
    assert_diagnostic_matches_fixture("c0001_import_unrecognized_module");
}

// PR-14 (D-136): a recognized module (`math`) with one unresolvable symbol
// inside an otherwise-valid `from math import ...` list is C0002, distinct
// from C0001, and fails the whole statement -- `sqrt` is not partially
// bound even though it is itself registered.
#[test]
fn c0002_from_import_unregistered_symbol() {
    assert_diagnostic_matches_fixture("c0002_from_import_unregistered_symbol");
}

// Issue #142: known Python 3.14 callable builtins that this compiler version
// does not implement (e.g. `ValueError`, `Exception`, `int`, `range`) are
// classified as `C0001` (capability gap), not `T0021` (name-resolution
// failure). The builtin genuinely exists in Python 3.14 -- the compiler just
// does not implement it yet. User-defined functions always take priority: a
// `def ValueError(...)` is called correctly, not classified as C0001. The
// same classification applies in the private-helper inference path.

#[test]
fn c0001_callable_builtin_value_error() {
    assert_diagnostic_matches_fixture("c0001_callable_builtin_value_error");
    assert_json_diagnostic_matches_fixture("c0001_callable_builtin_value_error");
}

#[test]
fn c0001_callable_builtin_exception() {
    assert_diagnostic_matches_fixture("c0001_callable_builtin_exception");
}

#[test]
fn c0001_callable_builtin_private_helper() {
    assert_diagnostic_matches_fixture("c0001_callable_builtin_private_helper");
}

// Issue #197: the published quick-start example's own type error. README.md
// and site/index.html document what `pycc check` prints when the quick-start
// program is given a `str` where its `fib` takes an `int`; this fixture is the
// compiler-generated original those documents are bound to, so a renderer
// change that would falsify them fails here first. Its rendering is the
// D-083 placeholder shape (`1:1`, one-character caret repeating the full
// message, no `help:` line); D-043 owns the real-span/help work.
#[test]
fn quick_start_type_error() {
    assert_diagnostic_matches_fixture("quick_start_type_error");
}

// Issue #890: every `C0001` message HIR lowering emits names the rejected
// construct in Python terms (`pycc_ast::expr_kind_name`/`stmt_kind_name`)
// instead of dumping the AST node's Rust `Debug` form or naming no
// construct at all. One rejected construct per fixture (D-219: HIR reports
// one diagnostic per failing top-level item, so a second construct in the
// same item would never be exercised).
#[test]
fn c0001_attribute_annotation() {
    assert_diagnostic_matches_fixture("c0001_attribute_annotation");
}

#[test]
fn c0001_string_annotation() {
    assert_diagnostic_matches_fixture("c0001_string_annotation");
}

#[test]
fn c0001_multi_target_assign() {
    assert_diagnostic_matches_fixture("c0001_multi_target_assign");
}

#[test]
fn c0001_chain_assign_empty_container() {
    assert_diagnostic_matches_fixture("c0001_chain_assign_empty_container");
}

#[test]
fn c0001_starred_unpack_target() {
    assert_diagnostic_matches_fixture("c0001_starred_unpack_target");
}

#[test]
fn c0001_attribute_ann_assign_target() {
    assert_diagnostic_matches_fixture("c0001_attribute_ann_assign_target");
}

#[test]
fn c0001_tuple_for_target() {
    assert_diagnostic_matches_fixture("c0001_tuple_for_target");
}

#[test]
fn c0001_for_iterable_not_name_or_call() {
    assert_diagnostic_matches_fixture("c0001_for_iterable_not_name_or_call");
}

#[test]
fn c0001_for_call_not_bare_name() {
    assert_diagnostic_matches_fixture("c0001_for_call_not_bare_name");
}

#[test]
fn c0001_call_of_call() {
    assert_diagnostic_matches_fixture("c0001_call_of_call");
}

#[test]
fn c0001_comprehension_iterable_not_name_or_call() {
    assert_diagnostic_matches_fixture("c0001_comprehension_iterable_not_name_or_call");
}

#[test]
fn c0001_protocol_body_assign() {
    assert_diagnostic_matches_fixture("c0001_protocol_body_assign");
}

#[test]
fn c0001_protocol_body_ellipsis() {
    assert_diagnostic_matches_fixture("c0001_protocol_body_ellipsis");
}

#[test]
fn c0001_function_local_import() {
    assert_diagnostic_matches_fixture("c0001_function_local_import");
}

#[test]
fn c0001_nested_def() {
    assert_diagnostic_matches_fixture("c0001_nested_def");
}

#[test]
fn c0001_nested_class() {
    assert_diagnostic_matches_fixture("c0001_nested_class");
}

// #1211: `(a or b).upper()` lowers now, so the receiver reaches the checker,
// which refuses a method call on `str`.
#[test]
fn t0043_boolop_receiver() {
    assert_diagnostic_matches_fixture("t0043_boolop_receiver");
}

// #1211: an `and`/`or` value join with no common type (pycc has no unions).
#[test]
fn t0021_boolop_no_common_type() {
    assert_diagnostic_matches_fixture("t0021_boolop_no_common_type");
}

// #1211: a container has no truth test in codegen, so it is no operand.
#[test]
fn t0021_boolop_container_operand() {
    assert_diagnostic_matches_fixture("t0021_boolop_container_operand");
}

// #1211: an instance whose class defines `__bool__`/`__len__` is refused.
#[test]
fn t0021_boolop_truth_dunder_operand() {
    assert_diagnostic_matches_fixture("t0021_boolop_truth_dunder_operand");
}

// #1211: a walrus in a short-circuited operand would bind conditionally.
#[test]
fn c0001_boolop_walrus_right_operand() {
    assert_diagnostic_matches_fixture("c0001_boolop_walrus_right_operand");
}

// #1212: a walrus in a chained-comparison operand at index 2 or later runs
// only when every earlier link held, so it would bind conditionally.
#[test]
fn c0001_compare_chain_walrus_later_operand() {
    assert_diagnostic_matches_fixture("c0001_compare_chain_walrus_later_operand");
}

// #1212: each chain link is typed like a single comparison.
#[test]
fn t0021_compare_chain_link_type() {
    assert_diagnostic_matches_fixture("t0021_compare_chain_link_type");
}

// #1212: `in`/`not in` inside a chain keeps its `C0001`.
#[test]
fn c0001_compare_chain_in_operator() {
    assert_diagnostic_matches_fixture("c0001_compare_chain_in_operator");
}

// #1211 / D-205: `x is not None and ...` does not narrow `x` in the right
// operand.
#[test]
fn t0021_boolop_no_narrowing() {
    assert_diagnostic_matches_fixture("t0021_boolop_no_narrowing");
}

#[test]
fn c0001_ellipsis_expression() {
    assert_diagnostic_matches_fixture("c0001_ellipsis_expression");
}

#[test]
fn c0001_get_zero_args_on_non_dict() {
    assert_diagnostic_matches_fixture("c0001_get_zero_args_on_non_dict");
}

// #898 (Part 1 of #881): a relative import from a file whose directory has
// no `__init__.py` is what CPython itself rejects, so it is `T0021`, not one
// of pycc's own capability gaps.
#[test]
fn t0021_relative_import_outside_package() {
    assert_diagnostic_matches_fixture("t0021_relative_import_outside_package");
}

/// Issue #890: no checked-in diagnostic fixture may carry an AST node's
/// Rust `Debug` form. Every `ruff_python_ast` node's `Debug` output contains
/// `node_index: NodeIndex(`, so scanning for that marker is sufficient; this
/// enforces `docs/DIAGNOSTICS.md`'s `C0001` sentence independently of the
/// `debug_assert!` inside `pycc_hir`'s own `unsupported()` helper.
#[test]
fn no_diagnostic_fixture_renders_an_ast_debug_dump() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/diagnostics");
    let mut scanned = 0;
    // #1224: fixtures that need their own `pycc.toml` live in a
    // subdirectory, so the scan recurses.
    let mut pending = vec![dir.clone()];
    while let Some(current) = pending.pop() {
        for entry in std::fs::read_dir(&current).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                pending.push(path);
            } else if path.extension().is_some_and(|ext| ext == "txt") {
                let text = std::fs::read_to_string(&path).unwrap();
                assert!(
                    !text.contains("NodeIndex("),
                    "{} renders an AST node's Debug form",
                    path.display()
                );
                scanned += 1;
            }
        }
    }
    assert!(
        scanned > 0,
        "no .expected.txt fixtures found in {}",
        dir.display()
    );
}

// D-228 (issue #918): parameterized container type annotations. There is no
// fixture auto-discovery in this harness -- every `.py`/`.expected.txt` pair
// needs its own registration below or it silently never runs.

#[test]
fn t0053_list_two_args() {
    assert_diagnostic_matches_fixture("t0053_list_two_args");
}

#[test]
fn t0054_set_element_unhashable() {
    assert_diagnostic_matches_fixture("t0054_set_element_unhashable");
}

#[test]
fn t0054_set_add_unhashable() {
    assert_diagnostic_matches_fixture("t0054_set_add_unhashable");
}

#[test]
fn t0055_unpack_too_many_values() {
    assert_diagnostic_matches_fixture("t0055_unpack_too_many_values");
}

#[test]
fn t0055_unpack_not_enough_values() {
    assert_diagnostic_matches_fixture("t0055_unpack_not_enough_values");
}

#[test]
fn t0053_set_two_args() {
    assert_diagnostic_matches_fixture("t0053_set_two_args");
}

#[test]
fn t0053_dict_one_arg() {
    assert_diagnostic_matches_fixture("t0053_dict_one_arg");
}

#[test]
fn t0053_dict_three_args() {
    assert_diagnostic_matches_fixture("t0053_dict_three_args");
}

#[test]
fn t0053_tuple_empty() {
    assert_diagnostic_matches_fixture("t0053_tuple_empty");
}

#[test]
fn t0053_tuple_variadic_ellipsis() {
    assert_diagnostic_matches_fixture("t0053_tuple_variadic_ellipsis");
}

#[test]
fn t0053_local_annotation_arity() {
    assert_diagnostic_matches_fixture("t0053_local_annotation_arity");
}

// `docs/DIAGNOSTICS.md`'s quality bar puts the arity family among the ones
// that publish structured `help` (D-152). The human format never renders it,
// so the `.expected.txt` fixtures above cannot see it: each of `T0053`'s three
// construction arms therefore also has a `.expected.json` fixture pinning the
// `help` array, and the empty-`help` regression this covers is exactly what a
// human-format-only fixture would have shipped silently (review finding on
// this pull request).

#[test]
fn t0053_list_two_args_json_publishes_help() {
    assert_json_diagnostic_matches_fixture("t0053_list_two_args");
}

#[test]
fn t0053_dict_one_arg_json_publishes_help() {
    assert_json_diagnostic_matches_fixture("t0053_dict_one_arg");
}

#[test]
fn t0053_tuple_empty_json_publishes_help() {
    assert_json_diagnostic_matches_fixture("t0053_tuple_empty");
}

#[test]
fn t0053_tuple_variadic_ellipsis_json_publishes_help() {
    assert_json_diagnostic_matches_fixture("t0053_tuple_variadic_ellipsis");
}

#[test]
fn t0034_list_str_annotation() {
    assert_diagnostic_matches_fixture("t0034_list_str_annotation");
}

#[test]
fn t0034_nested_list_annotation() {
    assert_diagnostic_matches_fixture("t0034_nested_list_annotation");
}

#[test]
fn t0036_dict_int_key_annotation() {
    assert_diagnostic_matches_fixture("t0036_dict_int_key_annotation");
}

#[test]
fn t0038_set_str_annotation() {
    assert_diagnostic_matches_fixture("t0038_set_str_annotation");
}

#[test]
fn t0039_tuple_str_element_annotation() {
    assert_diagnostic_matches_fixture("t0039_tuple_str_element_annotation");
}

#[test]
fn t0042_type_param_in_container_annotation() {
    assert_diagnostic_matches_fixture("t0042_type_param_in_container_annotation");
}

// `Optional[T]`'s own `T0049` gate (`crates/pycc_hir/src/func.rs`) predates
// this change but was unreachable for a container inner type: `list[int]`
// could not be lowered at all, so `list[int] | None` failed on the inner
// annotation with `C0001`. Lowering the container annotation makes the
// composed form reach `T0049` for the first time, rendering a container name
// into that message, so both directions of the composition are pinned here.
#[test]
fn t0049_optional_of_container() {
    assert_diagnostic_matches_fixture("t0049_optional_of_container");
}

#[test]
fn t0034_list_of_optional() {
    assert_diagnostic_matches_fixture("t0034_list_of_optional");
}

// The recursive `annotation_to_ty(arg, ..)?` inside
// `container_annotation_to_ty` propagates a failure from the *inner*
// annotation before any element-type gate runs, so the reported span is the
// inner annotation's own, not the whole subscript's. Pinned here because it is
// the one error path in that function reached by neither `T0053` (arity, which
// runs first) nor `T0034`/`T0036`/`T0038`/`T0039`/`T0042` (element gates, which
// run after this returns).
#[test]
fn c0001_bare_container_inside_container() {
    assert_diagnostic_matches_fixture("c0001_bare_container_inside_container");
}

#[test]
fn c0001_bare_list_annotation() {
    assert_diagnostic_matches_fixture("c0001_bare_list_annotation");
}

#[test]
fn c0001_bare_tuple_annotation() {
    assert_diagnostic_matches_fixture("c0001_bare_tuple_annotation");
}

#[test]
fn c0001_bare_dict_annotation() {
    assert_diagnostic_matches_fixture("c0001_bare_dict_annotation");
}

#[test]
fn c0001_protocol_container_attribute() {
    assert_diagnostic_matches_fixture("c0001_protocol_container_attribute");
}

// #934: the issue's own program verbatim. Exactly one diagnostic is expected:
// the CLI lowers each top-level item once (D-219), the type checker never
// runs after an HIR failure, and `main`'s `p: P = make()` is not an HIR
// cascade because calls are not resolved at HIR.
#[test]
fn c0001_protocol_return_annotation() {
    assert_diagnostic_matches_fixture("c0001_protocol_return_annotation");
}

// #948: the same gate reached through a *self-referential* protocol member.
// `def clone(self) -> P` inside `class P(Protocol)` used to resolve to
// `Ty::Instance("P")` -- the enclosing-class-name arm of `annotation_to_ty`
// ran before the `class_defs` protocol lookup -- so the gate never saw a
// `Ty::Protocol` and every conforming concrete class was rejected instead
// with a spurious `T0046` rendered at the module's first line. The caret is
// on the annotation, and the message is #934's own, unchanged.
#[test]
fn c0001_protocol_self_return() {
    assert_diagnostic_matches_fixture("c0001_protocol_self_return");
}

// #931: a subscript on a base that is not a class -- a PEP 695 type
// parameter, a builtin scalar, `Self`, or a non-class `type` alias -- is
// rejected with the same `T0044` instead of silently discarding the type
// argument, in parameter, return, class-attribute and every other annotation
// position.
#[test]
fn t0044_type_param_subscript_annotation() {
    assert_diagnostic_matches_fixture("t0044_type_param_subscript_annotation");
}

#[test]
fn t0044_type_param_subscript_annotation_json_carries_no_help() {
    // D-152: no determinate safe replacement, so the structured `help` list
    // is empty rather than absent.
    assert_json_diagnostic_matches_fixture("t0044_type_param_subscript_annotation");
}

#[test]
fn t0044_builtin_scalar_subscript_annotation() {
    assert_diagnostic_matches_fixture("t0044_builtin_scalar_subscript_annotation");
}

#[test]
fn t0044_self_subscript_annotation() {
    assert_diagnostic_matches_fixture("t0044_self_subscript_annotation");
}

#[test]
fn t0044_type_alias_subscript_annotation() {
    assert_diagnostic_matches_fixture("t0044_type_alias_subscript_annotation");
}

#[test]
fn t0044_subscript_annotation_class_attribute() {
    assert_diagnostic_matches_fixture("t0044_subscript_annotation_class_attribute");
}

#[test]
fn t0044_subscript_annotation_return() {
    assert_diagnostic_matches_fixture("t0044_subscript_annotation_return");
}

// Issue #905: #790's `if TYPE_CHECKING:` constant-fold used to swallow the
// context checks above along with the rest of the guarded body. These three
// fixtures pin the byte-exact rendering of the re-check for one violation
// from each of its three sources -- `lower_stmt`'s shared `return` predicate,
// its shared `break` predicate, and the `lower_expr` delegation.

#[test]
fn l0001_return_inside_finally_type_checking() {
    assert_diagnostic_matches_fixture("l0001_return_inside_finally_type_checking");
}

#[test]
fn l0001_break_outside_loop_type_checking() {
    assert_diagnostic_matches_fixture("l0001_break_outside_loop_type_checking");
}

#[test]
fn l0001_yield_outside_function_type_checking() {
    assert_diagnostic_matches_fixture("l0001_yield_outside_function_type_checking");
}

// Part 1 of #883 (#962): `import <stdlib module> as <alias>`. The
// unregistered-symbol diagnostic names the canonical module and, because
// the receiver was spelled through an alias, the alias too; a project
// module behind `import X as Y` keeps the plain-`import` C0001 verbatim.
#[test]
fn c0001_import_alias_unregistered_symbol() {
    assert_diagnostic_matches_fixture("c0001_import_alias_unregistered_symbol");
}

/// The fixture sits beside a real `geometry.py`, so `import geometry as g`
/// resolves to a project module and keeps #964's C0001. Without that
/// sibling the name is answered as a CPython module, which #1291 admits
/// under an alias.
#[test]
fn c0001_import_alias_project_module() {
    assert_diagnostic_matches_fixture("alias_project_module/c0001_import_alias_project_module");
}

/// #921/#944: calling an enum class with no arguments is `C0001` at the
/// call expression (reported by `pycc_hir`'s per-item scan), not at `1:1`
/// and not a `pycc_types` panic.
#[test]
fn c0001_enum_class_call_no_args() {
    assert_diagnostic_matches_fixture("c0001_enum_class_call_no_args");
}

/// #921/#944: the value-lookup spelling `Color(1)` is the same `C0001` at
/// its own call expression.
#[test]
fn c0001_enum_class_call_value() {
    assert_diagnostic_matches_fixture("c0001_enum_class_call_value");
}

// #1021 (D-245): `T0003` is the code an empty `[]`/`{}` reports once the
// pre-check resolution pass finds no annotation, no existing binding, and no
// forward producer. Before #1021 it was registered but never emitted, and
// these programs reported `T0021` citing #927 instead.

/// A local with no annotation and no producer: the binding's name appears in
/// the message, which is the only locator a `T0003` gets (`HirStmt::Assign`
/// carries no span, so the render is at `1:1`).
#[test]
fn t0003_unresolved_local_list() {
    assert_diagnostic_matches_fixture("t0003_unresolved_local_list");
}

/// The dict spelling of the same shape.
#[test]
fn t0003_unresolved_local_dict() {
    assert_diagnostic_matches_fixture("t0003_unresolved_local_dict");
}

/// #1265: an unannotated `self.xs = []` in `__init__` whose slot no source
/// types -- no inherited slot and no `self.xs.append(v)` in the class's own
/// methods -- names the attribute and the class (the span-less `1:1` every
/// container `T0003` renders at).
#[test]
fn t0003_unresolved_attribute_list() {
    assert_diagnostic_matches_fixture("t0003_unresolved_attribute_list");
}

/// #1265: the unannotated `self.d = {}` stays `C0001`, naming the annotated
/// spelling, the class-body declaration spelling (#1266), and the #891
/// producer it waits for.
#[test]
fn c0001_unannotated_attribute_empty_dict() {
    assert_diagnostic_matches_fixture("c0001_unannotated_attribute_empty_dict");
}

/// #1266: a value-less class-body annotation is an instance attribute
/// declaration its own `__init__` must establish; one it never assigns names
/// both fixes, including the class-constant spelling for a scalar.
#[test]
fn c0001_unestablished_instance_declaration() {
    assert_diagnostic_matches_fixture("c0001_unestablished_instance_declaration");
}

/// #1266: a declared type with no instance-slot representation names the
/// admitted set.
#[test]
fn c0001_instance_declaration_type() {
    assert_diagnostic_matches_fixture("c0001_instance_declaration_type");
}

/// A non-binding position. `infer_expr_in` has no expected type and there is
/// no name to scan forward from, so the message keeps its generic " here"
/// wording rather than naming a binding.
#[test]
fn t0003_call_argument_has_no_binding() {
    assert_diagnostic_matches_fixture("t0003_call_argument_has_no_binding");
}

/// `xs: list[int] = [[]]`: the outer literal is a non-empty `ListLiteral`, so
/// the pass never looks at it; the inner `[]` is an element position with no
/// annotation, no binding, and no producer of its own. The message therefore
/// keeps the generic `" here"` wording rather than naming `xs`: `xs` itself is
/// validly annotated, and the unresolvable node is the inner literal
/// (#1021 review round 1).
#[test]
fn t0003_nested_empty_list() {
    assert_diagnostic_matches_fixture("t0003_nested_empty_list");
}

/// `xs: list[int] = {}`: the annotation resolves to a list shape while the
/// literal is a dict. The pass refuses to rewrite `{}` into a typed empty
/// list -- that would silently repair a program the user got wrong -- so the
/// literal survives to the check phase and reports `T0003`.
#[test]
fn t0003_annotation_shape_does_not_match_the_literal() {
    assert_diagnostic_matches_fixture("t0003_annotation_shape_does_not_match_the_literal");
}

/// `x: int = []`: an annotation that is neither a `list[...]` nor a
/// `dict[...]` supplies no element type at all, so source (1) yields nothing
/// and the literal falls through to `T0003`.
#[test]
fn t0003_scalar_annotation_supplies_no_element_type() {
    assert_diagnostic_matches_fixture("t0003_scalar_annotation_supplies_no_element_type");
}

/// A syntactic producer whose own operands do not infer (`d[missing] = 1`)
/// supplies no key/value types either: the producer scan swallows the
/// inference failure rather than guessing, and the literal reports `T0003`.
#[test]
fn t0003_producer_value_does_not_infer() {
    assert_diagnostic_matches_fixture("t0003_producer_value_does_not_infer");
}

/// Module-level statements are outside the pass's scope (it walks function
/// bodies), so a module-level `xs = []` still reports `T0003`.
#[test]
fn t0003_module_level_empty_list() {
    assert_diagnostic_matches_fixture("t0003_module_level_empty_list");
}

/// The producers-vs-consumers rule: `for x in xs` *reads* an element type
/// that must already be known, so it cannot supply one. Only `append` and
/// `d[k] = v` are producers.
#[test]
fn t0003_consumer_use_is_not_a_producer() {
    assert_diagnostic_matches_fixture("t0003_consumer_use_is_not_a_producer");
}

/// #1021 review round 3: a producer is recognized only in statement position.
/// `y = xs.append(1)` is a valid value expression that binds `None`, and it
/// compiles when the list's element type is already known -- but it does not
/// resolve an empty literal, so this program reports `T0003`. The exclusion is
/// deliberate (D-245 item 8): the rewrite side is direct-assignment-only too,
/// and a generic expression walker would move the boundary rather than remove
/// it. This fixture is what keeps the documented exclusion honest.
#[test]
fn t0003_value_position_append_is_not_a_producer() {
    assert_diagnostic_matches_fixture("t0003_value_position_append_is_not_a_producer");
}

/// #1021 review round 2: `bind_local_types_in_stmt`'s `AnnAssign` fallback is
/// deliberately broad -- it seeds the declared annotation on *any* inference
/// failure, not only on an empty literal. That is what lets an earlier
/// `xs = []` resolve from the later annotation, so checking reaches and
/// reports the real defect (`undefined_name`) instead of stopping at a
/// `T0003` about a container the program does in fact annotate. Narrowing the
/// fallback to the empty-literal shape was considered in round 2 and refuted:
/// it replaces this message with a spurious `T0003` and fixes nothing, since
/// a D-105-incompatible annotation such as `list[str]` is independently
/// rejected with its own better-spanned `T0034` whether or not any empty
/// literal precedes it.
#[test]
fn t0021_annotated_broken_value_still_reports_the_real_defect() {
    assert_diagnostic_matches_fixture("t0021_annotated_broken_value_still_reports_the_real_defect");
}

/// D-228: an inferred element type passes through exactly the same
/// `check_container_ty` gate a written annotation does, so an inferred
/// `list[str]` is `T0034` (D-105), not a silently accepted list.
#[test]
fn t0034_inferred_list_str_is_not_compiled_yet() {
    assert_diagnostic_matches_fixture("t0034_inferred_list_str_is_not_compiled_yet");
}

/// The dict half of the same gate: an inferred `dict[int, int]` is `T0036`
/// (D-122).
#[test]
fn t0036_inferred_dict_int_int_is_not_compiled_yet() {
    assert_diagnostic_matches_fixture("t0036_inferred_dict_int_int_is_not_compiled_yet");
}

// Part 1 of #1026: a CPython `import` binds the module object and nothing
// more, so every operation on the binding is refused.

/// `I0404` on the plainest read there is -- matching on the bound module.
/// (The fixture passed the module to `print` until #1340 admitted
/// rendering a CPython object.) The choke-point set behind this one fixture
/// (an expression-position read, a `for`/comprehension iterable, and a call
/// of the name) is exercised shape by shape in
/// `tests/issue_1080_foreign_object.rs`.
#[test]
fn i0404_foreign_module_operation() {
    assert_diagnostic_matches_fixture("i0404_foreign_module_operation");
}

/// Rebinding an imported module is a second, non-foreign binding of the
/// name, which Part 1 of #1026 refuses outright rather than supporting
/// positionally (PR 1c of #1080). Pinned in a fixture because the
/// diagnostic must point at the *shadowing* statement, not at the import:
/// the import is the binding the user almost certainly meant to keep.
#[test]
fn c0001_foreign_module_shadowed_import() {
    assert_diagnostic_matches_fixture("c0001_foreign_module_shadowed_import");
}

/// #1209 (Part 1 of #1018): every capability refusal an augmented assignment
/// gets is spelled for `op=` and spans the augmented statement, instead of
/// the generic "statement kind not supported yet" or an assignment-worded
/// message from the rewritten `target = target op value`.
#[test]
fn c0001_aug_assign_operator() {
    assert_diagnostic_matches_fixture("c0001_aug_assign_operator");
}

#[test]
fn c0001_aug_assign_computed_attribute_base() {
    assert_diagnostic_matches_fixture("c0001_aug_assign_computed_attribute_base");
}

#[test]
fn c0001_aug_assign_computed_subscript_base() {
    assert_diagnostic_matches_fixture("c0001_aug_assign_computed_subscript_base");
}

#[test]
fn c0001_aug_assign_slice() {
    assert_diagnostic_matches_fixture("c0001_aug_assign_slice");
}

#[test]
fn c0001_aug_assign_computed_index() {
    assert_diagnostic_matches_fixture("c0001_aug_assign_computed_index");
}

// #1224 (D-128): `I0402`, the interop policy's rejection of a CPython-backed
// import. The same source is rejected by `--pure` and by `--interop-policy
// deny`; the message differs only in the policy's source.
#[test]
fn i0402_pure_rejects_foreign_import() {
    assert_check_snapshots(
        "i0402_pure_rejects_foreign_import",
        "i0402_pure_rejects_foreign_import",
        &["--pure"],
    );
}

#[test]
fn i0402_deny_rejects_foreign_import() {
    assert_check_snapshots(
        "i0402_deny_rejects_foreign_import",
        "i0402_deny_rejects_foreign_import",
        &["--interop-policy", "deny"],
    );
}

/// `--interop-policy allowlist` with no manifest has an empty allow set.
#[test]
fn i0402_allowlist_without_roots() {
    assert_check_snapshots(
        "i0402_allowlist_without_roots",
        "i0402_allowlist_without_roots",
        &["--interop-policy", "allowlist"],
    );
}

/// `interop_allowlist/pycc.toml` configures `allowlist` with `allow =
/// ["json"]`, so `import pprint` is rejected and the message names the
/// manifest. The subdirectory keeps the manifest away from every other
/// fixture in `tests/diagnostics/`.
#[test]
fn i0402_allowlist_unlisted_root() {
    assert_check_snapshots(
        "interop_allowlist/i0402_allowlist_unlisted_root",
        "interop_allowlist/i0402_allowlist_unlisted_root",
        &[],
    );
}

/// An explicit CLI policy wins over the configured one (D-128 rule 2).
#[test]
fn i0402_cli_deny_overrides_manifest_allowlist() {
    assert_check_snapshots(
        "interop_allowlist/i0402_allowlist_unlisted_root",
        "interop_allowlist/i0402_cli_deny_overrides_manifest_allowlist",
        &["--interop-policy", "deny"],
    );
}

// #1378: `from typing import Any` resolves, but `Any` in an annotation is
// still the single `T0002` refusal -- no `C0002` import error beside it.
#[test]
fn t0002_any_after_typing_import() {
    assert_diagnostic_matches_fixture("t0002_any_after_typing_import");
}

// #1378: a legacy `typing` alias's arity message names the spelling written.
#[test]
fn t0053_legacy_dict_one_arg() {
    assert_diagnostic_matches_fixture("t0053_legacy_dict_one_arg");
}

// The human format never renders `help`, so the JSON twin pins that the
// structured advice also names the spelling written (`Dict[str, int]`).
#[test]
fn t0053_legacy_dict_one_arg_json_publishes_help() {
    assert_json_diagnostic_matches_fixture("t0053_legacy_dict_one_arg");
}
