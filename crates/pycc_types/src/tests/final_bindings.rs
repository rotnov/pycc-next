//! `Final` reassignment checks and typing/dataclasses import composition.

use super::*;

// -- PEP 591 (#383): Final[X] reassignment diagnostic T0045 ------------

#[test]
fn final_annotated_name_with_initial_value_succeeds() {
    let mut env = Environment::new();
    check_stmt(
        &mut env,
        &HirStmt::AnnAssign {
            target: "x".to_string(),
            annotation: Ty::Int,
            value: Some(HirExpr::IntLiteral(1)),
            is_final: true,
        },
    )
    .unwrap();
    assert_eq!(env.lookup("x"), Some(Ty::Int));
}

#[test]
fn final_annotated_name_reassignment_is_t0045() {
    let mut env = Environment::new();
    check_stmt(
        &mut env,
        &HirStmt::AnnAssign {
            target: "x".to_string(),
            annotation: Ty::Int,
            value: Some(HirExpr::IntLiteral(1)),
            is_final: true,
        },
    )
    .unwrap();
    let err = check_stmt(
        &mut env,
        &HirStmt::Assign {
            target: "x".to_string(),
            value: HirExpr::IntLiteral(2),
        },
    )
    .unwrap_err();
    assert_eq!(err.code, "T0045");
}

#[test]
fn final_value_less_declaration_then_first_assignment_succeeds() {
    let mut env = Environment::new();
    check_stmt(
        &mut env,
        &HirStmt::AnnAssign {
            target: "x".to_string(),
            annotation: Ty::Int,
            value: None,
            is_final: true,
        },
    )
    .unwrap();
    check_stmt(
        &mut env,
        &HirStmt::Assign {
            target: "x".to_string(),
            value: HirExpr::IntLiteral(1),
        },
    )
    .unwrap();
    assert_eq!(env.lookup("x"), Some(Ty::Int));
}

#[test]
fn final_value_less_declaration_then_two_assignments_is_t0045() {
    let mut env = Environment::new();
    check_stmt(
        &mut env,
        &HirStmt::AnnAssign {
            target: "x".to_string(),
            annotation: Ty::Int,
            value: None,
            is_final: true,
        },
    )
    .unwrap();
    check_stmt(
        &mut env,
        &HirStmt::Assign {
            target: "x".to_string(),
            value: HirExpr::IntLiteral(1),
        },
    )
    .unwrap();
    let err = check_stmt(
        &mut env,
        &HirStmt::Assign {
            target: "x".to_string(),
            value: HirExpr::IntLiteral(2),
        },
    )
    .unwrap_err();
    assert_eq!(err.code, "T0045");
}

#[test]
fn final_reassignment_in_function_scope_is_t0045() {
    let mut env = Environment::new();
    check_stmt_in_function(
        &mut env,
        &["x"],
        &HirStmt::AnnAssign {
            target: "x".to_string(),
            annotation: Ty::Int,
            value: Some(HirExpr::IntLiteral(1)),
            is_final: true,
        },
        Ty::None,
    )
    .unwrap();
    let err = check_stmt_in_function(
        &mut env,
        &["x"],
        &HirStmt::Assign {
            target: "x".to_string(),
            value: HirExpr::IntLiteral(2),
        },
        Ty::None,
    )
    .unwrap_err();
    assert_eq!(err.code, "T0045");
}

#[test]
fn final_reassignment_via_valued_annassign_is_t0045() {
    let mut env = Environment::new();
    check_stmt(
        &mut env,
        &HirStmt::AnnAssign {
            target: "x".to_string(),
            annotation: Ty::Int,
            value: Some(HirExpr::IntLiteral(1)),
            is_final: true,
        },
    )
    .unwrap();
    let err = check_stmt(
        &mut env,
        &HirStmt::AnnAssign {
            target: "x".to_string(),
            annotation: Ty::Int,
            value: Some(HirExpr::IntLiteral(2)),
            is_final: true,
        },
    )
    .unwrap_err();
    assert_eq!(err.code, "T0045");
}

#[test]
fn final_does_not_block_a_non_final_reassignment_of_a_different_name() {
    // A `Final` declaration on `x` must not affect a different name `y`.
    let mut env = Environment::new();
    check_stmt(
        &mut env,
        &HirStmt::AnnAssign {
            target: "x".to_string(),
            annotation: Ty::Int,
            value: Some(HirExpr::IntLiteral(1)),
            is_final: true,
        },
    )
    .unwrap();
    check_stmt(
        &mut env,
        &HirStmt::Assign {
            target: "y".to_string(),
            value: HirExpr::IntLiteral(2),
        },
    )
    .unwrap();
    assert_eq!(env.lookup("y"), Some(Ty::Int));
}

#[test]
fn child_for_function_clears_a_final_entry_for_a_shadowed_local_name() {
    // A module-level `Final` declaration for `x` must not leak into an
    // unrelated function's own local `x` — mirrors the existing
    // `bindings`/`declared`-clearing behavior.
    let mut module_env = Environment::new();
    check_stmt(
        &mut module_env,
        &HirStmt::AnnAssign {
            target: "x".to_string(),
            annotation: Ty::Int,
            value: Some(HirExpr::IntLiteral(1)),
            is_final: true,
        },
    )
    .unwrap();
    let mut fn_env = module_env.child_for_function(&["x"]);
    // The function's own body assigns to its local `x` — if the
    // module-level `finals` entry leaked through, this would fail with
    // T0045.
    check_stmt_in_function(
        &mut fn_env,
        &["x"],
        &HirStmt::Assign {
            target: "x".to_string(),
            value: HirExpr::IntLiteral(42),
        },
        Ty::None,
    )
    .unwrap();
    // A second assignment in the function body should also succeed
    // (the function-local `x` is not `Final`).
    check_stmt_in_function(
        &mut fn_env,
        &["x"],
        &HirStmt::Assign {
            target: "x".to_string(),
            value: HirExpr::IntLiteral(99),
        },
        Ty::None,
    )
    .unwrap();
}

// -- #579 (Part 3 of #572): the stdlib decorator-marker surface -----
//
// pycc recognizes `@dataclass`/`@override`/`@dataclass_transform()` as
// bare names without an import (the `Final`/`Annotated`/`Enum`
// precedent). What #579 adds is that the *import* CPython itself
// requires now resolves, so the PEP 557/698/3129 conformance fixtures
// can carry it and still run under the pinned oracle. These tests pin
// the composed shape -- import plus decorator in one program -- rather
// than only the registry lookup, which `pycc_std`'s own tests cover.

#[test]
fn dataclasses_import_composes_with_a_dataclass_decorated_class() {
    let result = check_source(
        "from dataclasses import dataclass\n@dataclass\nclass Point:\n    x: int\n    y: int\np = Point(1, 2)\nprint(p)\n",
    );
    assert!(
        result.is_ok(),
        "`from dataclasses import dataclass` plus `@dataclass` must type-check: {result:?}"
    );
}

#[test]
fn typing_override_import_composes_with_an_override_decorated_method() {
    let result = check_source(
        "from typing import override\nclass Base:\n    def __init__(self) -> None:\n        self.x = 0\n    def f(self) -> int:\n        return 1\nclass Derived(Base):\n    def __init__(self) -> None:\n        return\n    @override\n    def f(self) -> int:\n        return 2\nd = Derived()\nprint(d.f())\n",
    );
    assert!(
        result.is_ok(),
        "`from typing import override` plus `@override` must type-check: {result:?}"
    );
}

#[test]
fn typing_dataclass_transform_import_composes_with_its_decorator() {
    // The `@dataclass_transform()`-as-`@dataclass` treatment is a
    // deliberate semantic approximation (D-196, #749) and is why
    // no byte-for-byte fixture exercises it; the import resolving is
    // still in scope here, and this pins it.
    let result = check_source(
        "from typing import dataclass_transform\n@dataclass_transform()\nclass Tagged:\n    tag: int\nt = Tagged(1)\nprint(t.tag)\n",
    );
    assert!(
        result.is_ok(),
        "`from typing import dataclass_transform` plus its decorator must type-check: {result:?}"
    );
}
