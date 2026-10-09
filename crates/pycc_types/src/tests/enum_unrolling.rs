//! Enum-loop unrolling unit tests for the type-checking crate root.
//!
//! Extracted verbatim under AGENTS.md's decomposability rule (part of #695,
//! which tracks decomposing the oversized `tests.rs` these child modules came
//! out of): the first run came from `tests/exception_handling.rs` and the
//! `#379` run below it came directly from `tests.rs`. Together they exercise
//! `unroll_enum_loops` recursing through ordinary statement nesting —
//! functions, `if`, `while`, and `for` — and through the `#379` recursive
//! branches, rather than through the constructs covered by the
//! `exception_handling` child module. The `parse_check_resolve` helper the
//! first run shares stays in the parent, because tests that remain there call
//! it too and a parent cannot see a child's private items; the
//! `parse_and_check` helper below is local to this file, because only the
//! `#379` tests use it. As a child module this still sees the parent's private
//! items directly through `use super::*`, so nothing needed widened
//! visibility; only the tests' location changed.

use super::*;

// -- enum-loop unrolling through ordinary statement nesting -------------

#[test]
fn enum_loop_inside_function_unrolls() {
    // Covers the `HirItem::Function` arm of `unroll_enum_loops` —
    // a function body containing an enum loop is unrolled.
    let src = "\
from enum import Enum
class Color(Enum):
    RED = 1
    GREEN = 2
def f() -> int:
    x = 0
    for c in Color:
        x = x + 1
    return x
f()
";
    parse_check_resolve(src).expect("check should succeed");
}

#[test]
fn enum_loop_with_nested_if_unrolls() {
    // Covers the `HirStmt::If` arm of `unroll_enum_loops_in_stmts` —
    // an `if` inside an enum loop body is recursed into.
    let src = "\
from enum import Enum
class Color(Enum):
    RED = 1
    GREEN = 2
x = 0
for c in Color:
    if x > 0:
        x = x + 1
";
    parse_check_resolve(src).expect("check should succeed");
}

#[test]
fn enum_loop_with_nested_while_unrolls() {
    // Covers the `HirStmt::While` arm of `unroll_enum_loops_in_stmts`.
    let src = "\
from enum import Enum
class Color(Enum):
    RED = 1
    GREEN = 2
x = 0
for c in Color:
    while x < 10:
        x = x + 1
";
    parse_check_resolve(src).expect("check should succeed");
}

#[test]
fn enum_loop_with_nested_for_range_unrolls() {
    // Covers the `HirStmt::ForRange` arm of `unroll_enum_loops_in_stmts`.
    let src = "\
from enum import Enum
class Color(Enum):
    RED = 1
    GREEN = 2
x = 0
for c in Color:
    for i in range(3):
        x = x + 1
";
    parse_check_resolve(src).expect("check should succeed");
}

#[test]
fn plain_if_while_for_range_recurse_when_module_has_an_enum() {
    // `unroll_enum_loops` runs `unroll_enum_loops_in_stmts` over every
    // top-level statement whenever *any* class in the module is an enum --
    // not only over statements that structurally contain a `for ... in
    // <EnumClass>` loop. `enum_loop_with_nested_if_unrolls` (and its
    // `_while`/`_for_range` siblings above) nest the `if`/`while`/`for
    // range` *inside* the enum loop's own body, which the `ForList` enum
    // branch clones verbatim rather than recursing into -- so those tests
    // never actually reach `unroll_enum_loops_in_stmts`'s own
    // `HirStmt::If`/`HirStmt::While`/`HirStmt::ForRange` arms. Reach them
    // here directly with top-level `if`/`while`/`for i in range(..)`
    // statements that sit alongside an enum class definition but are not
    // themselves nested inside any enum loop.
    let src = "\
from enum import Enum
class Color(Enum):
    RED = 1
    GREEN = 2
x = 0
if x > 0:
    x = x + 1
while x < 10:
    x = x + 1
for i in range(3):
    x = x + 1
";
    parse_check_resolve(src).expect("check should succeed");
}

// -- #379: enum loop unrolling recursive branches -------------------

fn parse_and_check(source: &str) -> Result<(), pycc_diag::Diagnostic> {
    let module = pycc_parser::parse(source).expect("test fixture must parse");
    let hir = pycc_hir::lower_checked(&module).expect("test fixture must lower");
    check(&hir)
}

#[test]
fn enum_loop_nested_inside_a_non_enum_for_list_unrolls_correctly() {
    // Exercises `unroll_enum_loops_in_stmts`'s `ForList` non-enum
    // fallback branch (line 6655-6663): a `for x in xs:` loop (not an
    // enum class iteration) whose body contains a nested `for c in
    // Color:` enum loop.
    let result = parse_and_check(
        "class Color(Enum):\n    RED = 1\n    GREEN = 2\nxs = [1, 2]\nfor x in xs:\n    for c in Color:\n        print(c.value)\n",
    );
    assert!(
        result.is_ok(),
        "non-enum for-list with nested enum loop should check"
    );
}

#[test]
fn enum_loop_nested_inside_an_if_unrolls_correctly() {
    // Exercises `unroll_enum_loops_in_stmts`'s `If` branch
    // (lines 6665-6672).
    let result = parse_and_check(
        "class Color(Enum):\n    RED = 1\n    GREEN = 2\nx = 1\nif x:\n    for c in Color:\n        print(c.value)\n",
    );
    assert!(result.is_ok(), "enum loop nested inside if should check");
}

#[test]
fn enum_loop_nested_inside_a_while_unrolls_correctly() {
    // Exercises `unroll_enum_loops_in_stmts`'s `While` branch
    // (lines 6673-6678).
    let result = parse_and_check(
        "class Color(Enum):\n    RED = 1\n    GREEN = 2\ni = 0\nwhile i < 1:\n    for c in Color:\n        print(c.value)\n    i = i + 1\n",
    );
    assert!(result.is_ok(), "enum loop nested inside while should check");
}

#[test]
fn enum_loop_nested_inside_a_for_range_unrolls_correctly() {
    // Exercises `unroll_enum_loops_in_stmts`'s `ForRange` branch
    // (lines 6679-6693).
    let result = parse_and_check(
        "class Color(Enum):\n    RED = 1\n    GREEN = 2\nfor i in range(1):\n    for c in Color:\n        print(c.value)\n",
    );
    assert!(
        result.is_ok(),
        "enum loop nested inside for-range should check"
    );
}

#[test]
fn enum_loop_inside_a_function_body_checks() {
    // Exercises `check_enum_loop_body_function` — the function-body
    // variant of the enum loop check. The `for c in Color:` inside `f`
    // is type-checked by `check_stmt_in_function`, which delegates to
    // `check_enum_loop_body_function`.
    let result = parse_and_check(
        "class Color(Enum):\n    RED = 1\n    GREEN = 2\ndef f() -> None:\n    for c in Color:\n        print(c.value)\nf()\n",
    );
    assert!(result.is_ok(), "enum loop inside a function should check");
}

#[test]
fn enum_loop_with_pre_bound_loop_var_checks() {
    // Exercises the `was_definite = true` path in
    // `check_enum_loop_body_module`: when the loop variable is already
    // definitely bound to the same enum instance type before the enum
    // loop (here via an explicit `c = Color.RED` assignment), the
    // `if !was_definite` branch is skipped (the `bind_maybe` call is
    // not executed).
    let result = parse_and_check(
        "class Color(Enum):\n    RED = 1\n    GREEN = 2\nc = Color.RED\nfor c in Color:\n    print(c.value)\n",
    );
    assert!(
        result.is_ok(),
        "enum loop with pre-bound loop var should check"
    );
}

#[test]
fn enum_loop_in_function_with_pre_bound_loop_var_checks() {
    // Exercises the `was_definite = true` path in
    // `check_enum_loop_body_function`: when the loop variable is already
    // definitely bound to the same enum instance type before the enum
    // loop inside a function body (here via an explicit `c = Color.RED`
    // assignment).
    let result = parse_and_check(
        "class Color(Enum):\n    RED = 1\n    GREEN = 2\ndef f() -> None:\n    c = Color.RED\n    for c in Color:\n        print(c.value)\nf()\n",
    );
    assert!(
        result.is_ok(),
        "enum loop in function with pre-bound loop var should check"
    );
}

#[test]
fn check_and_resolve_unrolls_enum_loops() {
    // Covers `build_enum_member_table` and `unroll_enum_loops` (lines
    // 7281-7294, 7305+) inside this crate's own unit-test binary.
    // The `parse_and_check`-based enum-loop tests moved alongside this
    // one call `check` (validation only, no `unroll_enum_loops`); this test
    // calls `check_and_resolve` which runs the full pipeline
    // including `unroll_enum_loops`.
    let source =
        "class Color(Enum):\n    RED = 1\n    GREEN = 2\nfor c in Color:\n    print(c.value)\n";
    let module = pycc_parser::parse(source).expect("test fixture must parse");
    let hir = pycc_hir::lower_checked(&module).expect("test fixture must lower");
    let resolved = check_and_resolve(&hir);
    assert!(
        resolved.is_ok(),
        "enum loop should resolve and unroll successfully"
    );
    let resolved = resolved.unwrap();
    // After unrolling, the `for c in Color:` loop should be replaced
    // with two unrolled iterations (one per enum member).
    let has_unrolled = resolved.items.iter().any(|item| {
        matches!(
            item,
            HirItem::TopLevelStmt(HirStmt::ExprStmt(HirExpr::Call {
                callee,
                ..
            })) if callee == "print"
        )
    });
    assert!(
        has_unrolled,
        "unrolled enum loop should contain print calls"
    );
}

#[test]
fn enum_loop_body_with_undefined_name_is_rejected() {
    // Exercises the `?` error propagation path in
    // `check_enum_loop_body_module`: when a body statement references an
    // undefined name, `check_stmt` returns an error which propagates
    // through the `?` in the for loop.
    let result = parse_and_check(
        "class Color(Enum):\n    RED = 1\n    GREEN = 2\nfor c in Color:\n    print(undefined_name)\n",
    );
    assert!(
        result.is_err(),
        "enum loop with undefined name in body should be rejected"
    );
}

#[test]
fn enum_loop_body_in_function_with_undefined_name_is_rejected() {
    // Exercises the `?` error propagation path in
    // `check_enum_loop_body_function`: when a body statement inside a
    // function references an undefined name, `check_stmt_in_function`
    // returns an error which propagates through the `?` in the for loop.
    let result = parse_and_check(
        "class Color(Enum):\n    RED = 1\n    GREEN = 2\ndef f() -> None:\n    for c in Color:\n        print(undefined_name)\nf()\n",
    );
    assert!(
        result.is_err(),
        "enum loop in function with undefined name in body should be rejected"
    );
}

#[test]
fn enum_loop_with_incompatibly_typed_loop_var_is_rejected() {
    // Exercises the `?` error propagation path of `check_assignment`
    // in `check_enum_loop_body_module`: when the loop variable is
    // already bound to an incompatible type (here `int`), binding it
    // to `Ty::Instance("Color")` fails with T0023.
    let result = parse_and_check(
        "class Color(Enum):\n    RED = 1\n    GREEN = 2\nc = 0\nfor c in Color:\n    print(c.value)\n",
    );
    assert!(
        result.is_err(),
        "enum loop with incompatible loop var type should be rejected"
    );
}

#[test]
fn enum_loop_in_function_with_incompatibly_typed_loop_var_is_rejected() {
    // Exercises the `?` error propagation path of `check_assignment`
    // in `check_enum_loop_body_function`: when the loop variable is
    // already bound to an incompatible type inside a function body.
    let result = parse_and_check(
        "class Color(Enum):\n    RED = 1\n    GREEN = 2\ndef f() -> None:\n    c = 0\n    for c in Color:\n        print(c.value)\nf()\n",
    );
    assert!(
        result.is_err(),
        "enum loop in function with incompatible loop var type should be rejected"
    );
}
