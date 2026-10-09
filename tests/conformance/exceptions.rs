//! Conformance cohort: exception semantics.
//!
//! A `#[path]`-declared submodule of the `tests/conformance.rs` harness (see
//! its `harness_modules!` block). The helpers, `pycc_bin`, and
//! `oracle_python_bin` are the root's private items, visible here through
//! `use super::*;`. Every fixture stays flat under `tests/fixtures/` (D-102).

use super::*;

// #608 (PEP 3110): `try`/`except`/`else`/`finally` control flow, exception
// ordering across multiple handlers, bare `raise` re-raising the active
// exception out to an outer handler, and `finally` running on both the normal
// and the propagating path.
#[test]
#[ignore = "requires a pinned python3.14 (CPython 3.14.7) oracle on PATH"]
fn pep_3110_exceptions_matches_cpython_3_14_7_byte_for_byte() {
    let fixture =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/pep_3110_exceptions.py");
    let (debug_pycc, debug_cpython) =
        run_conformance_fixture_with_profile("pep_3110_exceptions_debug", &fixture, false);
    assert_eq!(
        debug_pycc, debug_cpython,
        "pycc (--debug) and CPython 3.14.7 disagree on tests/fixtures/pep_3110_exceptions.py"
    );
    let (release_pycc, release_cpython) =
        run_conformance_fixture_with_profile("pep_3110_exceptions_release", &fixture, true);
    assert_eq!(
        release_pycc, release_cpython,
        "pycc (--release) and CPython 3.14.7 disagree on tests/fixtures/pep_3110_exceptions.py"
    );
}

// #608 (PEP 409): `raise X from Y` explicit cause chaining and `raise X from
// None` context suppression. Both are observable here only through which
// handler catches the raised exception, because pycc emits no traceback and
// does not populate `__context__` yet (#606).
#[test]
#[ignore = "requires a pinned python3.14 (CPython 3.14.7) oracle on PATH"]
fn pep_0409_raise_from_matches_cpython_3_14_7_byte_for_byte() {
    let fixture =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/pep_0409_raise_from.py");
    let (debug_pycc, debug_cpython) =
        run_conformance_fixture_with_profile("pep_0409_raise_from_debug", &fixture, false);
    assert_eq!(
        debug_pycc, debug_cpython,
        "pycc (--debug) and CPython 3.14.7 disagree on tests/fixtures/pep_0409_raise_from.py"
    );
    let (release_pycc, release_cpython) =
        run_conformance_fixture_with_profile("pep_0409_raise_from_release", &fixture, true);
    assert_eq!(
        release_pycc, release_cpython,
        "pycc (--release) and CPython 3.14.7 disagree on tests/fixtures/pep_0409_raise_from.py"
    );
}

// Part 2 of #543 (#739, PEP 3151): the real `OSError` hierarchy -- `OSError`
// itself, its 11 other direct subclasses, and `ConnectionError`'s 4 further
// subclasses. Covers raising and catching at every tree depth (a direct
// `OSError` child, a `ConnectionError` grandchild caught via `ConnectionError`
// and via the `OSError` root, a sibling handler that does not catch an
// unrelated family member), handler ordering across several `except` clauses
// on one `try`, and `finally` running on the exceptional path. The matrix's
// row (docs/PYTHON_STANDARDS.md line 232) was flipped to `◐` in #753, once
// this fixture's registration was observed passing on a completed green
// Tier-1 run per D-102 (see PYTHON_STANDARDS.md's policy rule 11).
// errno-based construction/dispatch and the
// `errno`/`filename`/`strerror`/`winerror` instance attributes are out of
// scope for this issue (blocked on Part 3 of #541, #703) and are not
// exercised here.
#[test]
#[ignore = "requires a pinned python3.14 (CPython 3.14.7) oracle on PATH"]
fn pep_3151_oserror_matches_cpython_3_14_7_byte_for_byte() {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/pep_3151_oserror.py");
    let (debug_pycc, debug_cpython) =
        run_conformance_fixture_with_profile("pep_3151_oserror_debug", &fixture, false);
    assert_eq!(
        debug_pycc, debug_cpython,
        "pycc (--debug) and CPython 3.14.7 disagree on tests/fixtures/pep_3151_oserror.py"
    );
    let (release_pycc, release_cpython) =
        run_conformance_fixture_with_profile("pep_3151_oserror_release", &fixture, true);
    assert_eq!(
        release_pycc, release_cpython,
        "pycc (--release) and CPython 3.14.7 disagree on tests/fixtures/pep_3151_oserror.py"
    );
}

// Part 3 of #543 (#740, PEP 758): `except A, B:` (bare comma, no
// parentheses) alongside the pre-existing `except (A, B):` parenthesized
// form. Covers both spellings catching each of their named types, an `as`
// binding whose bound value is re-raised and recaught by an outer handler,
// a 3+-type handler, and a non-matching raise propagating out through an
// inner handler to an outer one. The matrix's row (docs/PYTHON_STANDARDS.md
// line 358) was flipped to `◐` in #753, once this fixture's registration
// was observed passing on a completed green Tier-1 run per D-102 (see
// PYTHON_STANDARDS.md's policy rule 11).
#[test]
#[ignore = "requires a pinned python3.14 (CPython 3.14.7) oracle on PATH"]
fn pep_0758_except_noparens_matches_cpython_3_14_7_byte_for_byte() {
    let fixture =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/pep_0758_except_noparens.py");
    let (debug_pycc, debug_cpython) =
        run_conformance_fixture_with_profile("pep_0758_except_noparens_debug", &fixture, false);
    assert_eq!(
        debug_pycc, debug_cpython,
        "pycc (--debug) and CPython 3.14.7 disagree on tests/fixtures/pep_0758_except_noparens.py"
    );
    let (release_pycc, release_cpython) =
        run_conformance_fixture_with_profile("pep_0758_except_noparens_release", &fixture, true);
    assert_eq!(
        release_pycc, release_cpython,
        "pycc (--release) and CPython 3.14.7 disagree on tests/fixtures/pep_0758_except_noparens.py"
    );
}

// Part 3 of #382 (#542, PEP 654, D-202): `except*` clauses and
// `ExceptionGroup` construction/dispatch. Covers a single `except*` clause
// catching a plain (non-group) exception, dispatch across multiple `except*`
// clauses in source order, a group built from two existing bindings with
// each member routed to its own matching clause, an `except* ... as`
// binding, `finally` running on the `except*` path, and `else` running when
// the `try` body raises nothing. This exercises exactly the literal-member-
// list, existing-value-only construction shape D-202 keeps in scope; it does
// not exercise a fresh constructor-call member (`T0021`, rejected before
// codegen), a non-literal member list (`T0021`), a bare unparameterized
// `except*:` (rejected at parse time), or a new exception raised from inside
// an `except*` clause body (D-202's own documented handler-body-raise
// simplification) -- none of those are byte-for-byte oracle comparisons a
// single fixture can usefully exercise, since the first three are
// compile-time rejections and the last has no accepted fixture anywhere in
// this suite.
#[test]
#[ignore = "requires a pinned python3.14 (CPython 3.14.7) oracle on PATH"]
fn pep_0654_except_star_matches_cpython_3_14_7_byte_for_byte() {
    let fixture =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/pep_0654_except_star.py");
    let (debug_pycc, debug_cpython) =
        run_conformance_fixture_with_profile("pep_0654_except_star_debug", &fixture, false);
    assert_eq!(
        debug_pycc, debug_cpython,
        "pycc (--debug) and CPython 3.14.7 disagree on tests/fixtures/pep_0654_except_star.py"
    );
    let (release_pycc, release_cpython) =
        run_conformance_fixture_with_profile("pep_0654_except_star_release", &fixture, true);
    assert_eq!(
        release_pycc, release_cpython,
        "pycc (--release) and CPython 3.14.7 disagree on tests/fixtures/pep_0654_except_star.py"
    );
}

// #1292 (Part 2 of #1282): the appended `ImportError`/`ModuleNotFoundError`
// builtins. Each class is caught by its own name, by `ImportError` and by
// `Exception`; an `except ModuleNotFoundError:` handler lets a plain
// `ImportError` fall to the next handler; a user `ImportError` subclass is
// caught by `except ImportError:`; and `issubclass` follows CPython's real
// `ModuleNotFoundError` -> `ImportError` -> `Exception` hierarchy. Every
// raise uses a literal message. The first caught exception is printed
// twice: #1298 fixed the bound exception's message refcount that once
// limited this fixture to a single print per exception.
#[test]
#[ignore = "requires a pinned python3.14 (CPython 3.14.7) oracle on PATH"]
fn builtin_import_error_matches_cpython_3_14_7_byte_for_byte() {
    let fixture =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/builtin_import_error.py");
    let (debug_pycc, debug_cpython) =
        run_conformance_fixture_with_profile("builtin_import_error_debug", &fixture, false);
    assert_eq!(
        debug_pycc, debug_cpython,
        "pycc (--debug) and CPython 3.14.7 disagree on tests/fixtures/builtin_import_error.py"
    );
    let (release_pycc, release_cpython) =
        run_conformance_fixture_with_profile("builtin_import_error_release", &fixture, true);
    assert_eq!(
        release_pycc, release_cpython,
        "pycc (--release) and CPython 3.14.7 disagree on tests/fixtures/builtin_import_error.py"
    );
}

// #1369: the `assert` statement and the `AssertionError` it raises. A
// passing `assert` does nothing; a failing one raises its message, or an
// empty one; the message is evaluated only on failure; a non-`bool` test
// uses `if` truthiness; a walrus in the test binds a name usable afterwards;
// a user `AssertionError` subclass is caught by `except AssertionError:`;
// and a passing module-level `assert` falls through.
#[test]
#[ignore = "requires a pinned python3.14 (CPython 3.14.7) oracle on PATH"]
fn assert_statement_matches_cpython_3_14_7_byte_for_byte() {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/assert_statement.py");
    let (debug_pycc, debug_cpython) =
        run_conformance_fixture_with_profile("assert_statement_debug", &fixture, false);
    assert_eq!(
        debug_pycc, debug_cpython,
        "pycc (--debug) and CPython 3.14.7 disagree on tests/fixtures/assert_statement.py"
    );
    let (release_pycc, release_cpython) =
        run_conformance_fixture_with_profile("assert_statement_release", &fixture, true);
    assert_eq!(
        release_pycc, release_cpython,
        "pycc (--release) and CPython 3.14.7 disagree on tests/fixtures/assert_statement.py"
    );
}

// #1298: a caught exception's message survives any number of renderings.
// `print(e)` and f-string interpolation each borrow the message the
// exception owns, and a raise whose message is a variable, an attribute or
// a parameter gives the exception its own reference, so the message
// outlives its original owner being overwritten or going out of scope.
// Covers literal, computed, variable, attribute and escaping-local
// messages; single- and multi-part f-strings; the OSError family; user
// exception classes; a runtime-raised builtin; a bare re-raise; and
// rebinding the handler name. Not a PEP-matrix row.
#[test]
#[ignore = "requires a pinned python3.14 (CPython 3.14.7) oracle on PATH"]
fn exception_message_reuse_matches_cpython_3_14_7_byte_for_byte() {
    let fixture =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/exception_message_reuse.py");
    let (debug_pycc, debug_cpython) =
        run_conformance_fixture_with_profile("exception_message_reuse_debug", &fixture, false);
    assert_eq!(
        debug_pycc, debug_cpython,
        "pycc (--debug) and CPython 3.14.7 disagree on tests/fixtures/exception_message_reuse.py"
    );
    let (release_pycc, release_cpython) =
        run_conformance_fixture_with_profile("exception_message_reuse_release", &fixture, true);
    assert_eq!(
        release_pycc, release_cpython,
        "pycc (--release) and CPython 3.14.7 disagree on tests/fixtures/exception_message_reuse.py"
    );
}

// #1289: a name bound in a `try` body and in every handler that can fall
// through the statement is definitely bound after it, so reading it no
// longer raises `T0041`; covers `else`, `finally`, `as` handlers, `except*`,
// terminating handlers, and the unannotated-helper (solver) path.
#[test]
#[ignore = "requires a pinned python3.14 (CPython 3.14.7) oracle on PATH"]
fn try_except_definite_assign_matches_cpython_3_14_7_byte_for_byte() {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/try_except_definite_assign.py");
    let (debug_pycc, debug_cpython) = run_conformance_fixture_with_profile(
        "try_except_definite_assign_debug",
        &fixture,
        false,
    );
    assert_eq!(
        debug_pycc, debug_cpython,
        "pycc (--debug) and CPython 3.14.7 disagree on tests/fixtures/try_except_definite_assign.py"
    );
    let (release_pycc, release_cpython) = run_conformance_fixture_with_profile(
        "try_except_definite_assign_release",
        &fixture,
        true,
    );
    assert_eq!(
        release_pycc, release_cpython,
        "pycc (--release) and CPython 3.14.7 disagree on tests/fixtures/try_except_definite_assign.py"
    );
}
