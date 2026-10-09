//! `typing.TYPE_CHECKING` marker tests for the type-checking crate root.
//!
//! Extracted verbatim from `tests/typing_cast.rs` under AGENTS.md's
//! decomposability rule (part of #695, which tracks decomposing the oversized
//! `tests.rs` these child modules came out of). These are the #790 tests that
//! pin how the `TYPE_CHECKING` marker behaves when it is used as a *value*
//! rather than as a static condition. As a child module this still sees the
//! parent's private items directly through `use super::*`, so nothing needed
//! widened visibility; only the tests' location changed.

use super::*;

// -- #790: `typing.TYPE_CHECKING` used as a value -----------------------

#[test]
fn bare_type_checking_name_used_as_a_value_is_not_defined() {
    // #790: `TYPE_CHECKING` is recognized *purely syntactically* as the
    // bare-name test of an `if`/`elif` (`pycc_hir::stmt::is_type_checking_guard`,
    // matching this module's existing bare-name `Final` precedent) --
    // `from typing import TYPE_CHECKING` never actually binds the bare
    // name into the environment for value use, exactly like every other
    // bare stdlib marker name. Referencing it as a value therefore fails
    // as an ordinary undefined name (T0021), not the dedicated
    // `TypeCheckingMarker` diagnostic below -- only the qualified
    // `typing.TYPE_CHECKING` spelling resolves through the registry for
    // value use (see `qualified_type_checking_marker_used_as_value_is_t0021`).
    let err = check_source("from typing import TYPE_CHECKING\nx = TYPE_CHECKING\nprint(x)\n")
        .unwrap_err();
    assert_eq!(err.code, "T0021");
    assert!(
        err.message.contains("is not defined"),
        "expected an undefined-name message, got: {}",
        err.message
    );
}

#[test]
fn qualified_type_checking_marker_used_as_value_is_t0021() {
    // #790: the qualified `typing.TYPE_CHECKING` spelling used as a value
    // (as opposed to an `if`/`elif` test) is rejected the same way as the
    // bare name -- `expr.rs`'s textual `math.sqrt`-shaped attribute
    // resolution routes it to the same registry entry and the same
    // `TypeCheckingMarker` arm.
    let err = check_source("import typing\nx = typing.TYPE_CHECKING\nprint(x)\n").unwrap_err();
    assert_eq!(err.code, "T0021");
    assert!(
        err.message.contains("compile-time marker"),
        "expected a TYPE_CHECKING-specific message, got: {}",
        err.message
    );
}

#[test]
fn qualified_type_checking_marker_as_value_in_private_helper_is_t0021() {
    // #790: the same `TypeCheckingMarker` arm in collect_expr_constraints'
    // Name handler (constraints.rs, the solver path) -- module-level and
    // top-level-function references above both resolve through the
    // validation pass (`expr.rs`), so this exercises the separate solver
    // path a private helper's body takes instead.
    let err = check_source(
        "import typing\ndef _helper() -> int:\n    x = typing.TYPE_CHECKING\n    return 1\nprint(_helper())\n",
    )
    .unwrap_err();
    assert_eq!(err.code, "T0021");
    assert!(
        err.message.contains("compile-time marker"),
        "expected a TYPE_CHECKING-specific message, got: {}",
        err.message
    );
}

#[test]
fn qualified_type_checking_marker_called_is_t0021() {
    // #791 D-068 review finding: the call-site marker guard in expr.rs's
    // infer_expr_in (the `Function` let-else fallthrough) previously fell
    // through to the generic `marker_is_not_a_value` message for
    // `TypeCheckingMarker` instead of the dedicated
    // `type_checking_marker_is_not_a_value` diagnostic its own doc comment
    // claims to produce -- `typing.TYPE_CHECKING(...)` must be rejected
    // with the TYPE_CHECKING-specific guidance, mirroring the `CastMarker`
    // precedent (`typing_cast::qualified_cast_marker_called_is_t0021`).
    let err = check_source("import typing\nx = typing.TYPE_CHECKING()\n").unwrap_err();
    assert_eq!(err.code, "T0021");
    assert!(
        err.message.contains("compile-time marker"),
        "expected a TYPE_CHECKING-specific message, got: {}",
        err.message
    );
}

#[test]
fn qualified_type_checking_marker_called_inside_an_annotated_function_is_t0021() {
    // #791 D-068 review finding: same call-site arm, reached via the
    // validation pass (a call inside a fully annotated public function)
    // rather than the solver path -- mirrors the `typing_cast` module's
    // `qualified_cast_marker_called_inside_an_annotated_function_is_t0021`.
    let err = check_source(
        "import typing\ndef f() -> int:\n    x = typing.TYPE_CHECKING()\n    return 1\nprint(f())\n",
    )
    .unwrap_err();
    assert_eq!(err.code, "T0021");
    assert!(
        err.message.contains("compile-time marker"),
        "expected a TYPE_CHECKING-specific message, got: {}",
        err.message
    );
}

#[test]
fn qualified_type_checking_marker_called_in_private_helper_is_t0021() {
    // #791 D-068 review finding: the same call-site `TypeCheckingMarker`
    // branch in constraints.rs's collect_expr_constraints (the solver
    // path) -- mirrors
    // `typing_cast::qualified_cast_marker_called_in_private_helper_is_t0021`.
    let err = check_source(
        "import typing\ndef _helper() -> int:\n    x = typing.TYPE_CHECKING()\n    return 1\n_helper()\n",
    )
    .unwrap_err();
    assert_eq!(err.code, "T0021");
    assert!(
        err.message.contains("compile-time marker"),
        "expected a TYPE_CHECKING-specific message, got: {}",
        err.message
    );
}
