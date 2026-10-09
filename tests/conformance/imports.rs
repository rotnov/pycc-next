//! Conformance cohort: `import` statement semantics.
//!
//! A `#[path]`-declared submodule of the `tests/conformance.rs` harness (see
//! its `harness_modules!` block). The helpers, `pycc_bin`, and
//! `oracle_python_bin` are the root's private items, visible here through
//! `use super::*;`. Every fixture stays flat under `tests/fixtures/` (D-102).

use super::*;

// #1280: a multi-name `import math, math as m` binds both names, each to
// the same module, and both reach the same libm calls and constant at
// module level and inside a function body.
#[test]
#[ignore = "requires a pinned python3.14 (CPython 3.14.7) oracle on PATH"]
fn import_multi_math_matches_cpython_3_14_7_byte_for_byte() {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/conformance_import_multi_math.py");
    let (debug_pycc, debug_cpython) = run_conformance_fixture_with_profile(
        "conformance_import_multi_math_debug",
        &fixture,
        false,
    );
    assert_eq!(
        debug_pycc, debug_cpython,
        "pycc (--debug) and CPython 3.14.7 disagree on tests/fixtures/conformance_import_multi_math.py"
    );
    let (release_pycc, release_cpython) = run_conformance_fixture_with_profile(
        "conformance_import_multi_math_release",
        &fixture,
        true,
    );
    assert_eq!(
        release_pycc, release_cpython,
        "pycc (--release) and CPython 3.14.7 disagree on tests/fixtures/conformance_import_multi_math.py"
    );
}
