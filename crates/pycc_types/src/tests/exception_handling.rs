//! Exception-handling unit tests for the type-checking crate root.
//!
//! Extracted verbatim from `tests.rs` under AGENTS.md's decomposability rule
//! (part of #695, which tracks decomposing that oversized file). These are the
//! #382 tests that exercise the exception-handling checks themselves, together
//! with the two `expect_top_level_*` assertion helpers only they use; the
//! `contains_return`/`introduces_bindings` traversal-coverage tests that walk
//! `try`/`try*` statements stay in the parent's own `#382 coverage tests` run. The `parse_check`
//! and `parse_check_resolve` helpers stay in the parent, because tests that
//! remain there call them too and a parent cannot see a child's private items.
//! As a child module this still sees the parent's private items directly
//! through `use super::*`, so nothing needed widened visibility.

use super::*;

/// Test helper: assert a top-level HIR item is a `Try`, panicking
/// otherwise.  The panic arm is covered by
/// `expect_top_level_try_panics_on_non_try`.
fn expect_top_level_try(item: &HirItem) {
    match item {
        HirItem::TopLevelStmt(HirStmt::Try { .. }) => {}
        _ => panic!("expected Try"),
    }
}

/// Test helper: assert a top-level HIR item is a `Raise`, panicking
/// otherwise.  The panic arm is covered by
/// `expect_top_level_raise_panics_on_non_raise`.
fn expect_top_level_raise(item: &HirItem) {
    match item {
        HirItem::TopLevelStmt(HirStmt::Raise { .. }) => {}
        _ => panic!("expected Raise"),
    }
}

#[test]
#[should_panic(expected = "expected Try")]
fn expect_top_level_try_panics_on_non_try() {
    expect_top_level_try(&HirItem::TopLevelStmt(HirStmt::ExprStmt(
        HirExpr::IntLiteral(0),
    )));
}

#[test]
#[should_panic(expected = "expected Raise")]
fn expect_top_level_raise_panics_on_non_raise() {
    expect_top_level_raise(&HirItem::TopLevelStmt(HirStmt::ExprStmt(
        HirExpr::IntLiteral(0),
    )));
}

#[test]
fn try_except_checks_successfully() {
    let resolved = parse_check_resolve("try:\n    x = 1\nexcept ValueError:\n    y = 2\n")
        .expect("check should succeed");
    expect_top_level_try(&resolved.items[0]);
}

#[test]
fn try_except_as_binds_exception_instance() {
    let resolved = parse_check_resolve("try:\n    x = 1\nexcept ValueError as e:\n    y = 2\n")
        .expect("check should succeed");
    expect_top_level_try(&resolved.items[0]);
}

#[test]
fn try_bare_except_checks_successfully() {
    let resolved =
        parse_check_resolve("try:\n    x = 1\nexcept:\n    y = 2\n").expect("check should succeed");
    expect_top_level_try(&resolved.items[0]);
}

#[test]
fn unknown_exception_handler_is_t0021() {
    let err = parse_check("try:\n    pass\nexcept UnknownError:\n    pass\n").unwrap_err();
    assert_eq!(err.code, "T0021");
    assert!(err.message.contains("not a recognized exception class"));
}

#[test]
fn try_else_finally_checks_successfully() {
    let resolved = parse_check_resolve(
        "try:\n    x = 1\nexcept ValueError:\n    y = 2\nelse:\n    z = 3\nfinally:\n    w = 4\n",
    )
    .expect("check should succeed");
    expect_top_level_try(&resolved.items[0]);
}

#[test]
fn raise_value_error_checks_successfully() {
    let resolved =
        parse_check_resolve("raise ValueError(\"bad\")\n").expect("check should succeed");
    expect_top_level_raise(&resolved.items[0]);
}

#[test]
fn raise_from_checks_successfully() {
    let resolved = parse_check_resolve("raise ValueError(\"bad\") from RuntimeError(\"cause\")\n")
        .expect("check should succeed");
    expect_top_level_raise(&resolved.items[0]);
}

#[test]
fn bare_raise_outside_handler_is_t0021() {
    let err = parse_check("raise\n").unwrap_err();
    assert_eq!(err.code, "T0021");
    assert!(err.message.contains("bare `raise`"));
}

#[test]
fn bare_raise_inside_handler_checks_successfully() {
    parse_check("try:\n    x = 1\nexcept ValueError:\n    raise\n").expect("check should succeed");
}

#[test]
fn raise_non_exception_is_t0021() {
    // `raise 42` — int is not an exception instance.
    let err = parse_check("raise 42\n").unwrap_err();
    assert_eq!(err.code, "T0021");
    assert!(err.message.contains("can only raise"));
}

#[test]
fn raise_from_non_exception_cause_is_t0021() {
    // `raise ValueError("x") from 42` — cause is not an exception.
    let err = parse_check("raise ValueError(\"x\") from 42\n").unwrap_err();
    assert_eq!(err.code, "T0021");
    assert!(err.message.contains("cause must be"));
}

#[test]
fn try_in_function_checks_successfully() {
    let src = "\
def f() -> int:
    x = 0
    try:
        x = 1
    except ValueError:
        y = 2
    return x
";
    parse_check(src).expect("check should succeed");
}

#[test]
fn try_with_return_in_body_and_handler() {
    let src = "\
def f() -> int:
    try:
        return 1
    except ValueError:
        return 2
    return 3
";
    parse_check(src).expect("check should succeed");
}

#[test]
fn try_with_return_in_finally_rejects_with_l0001() {
    // PEP 765 (#738, Part 1 of #543): a `return` that escapes a `finally`
    // block is rejected during HIR lowering, before the type checker ever
    // sees it -- so this goes through `pycc_hir::lower_checked` directly,
    // not the parent module's `super::parse_check`/`super::parse_check_resolve`
    // helpers (both of which `.expect()` lowering to succeed and would panic
    // on this input).
    // The remaining PEP 765 edge cases (break/continue in finally, loops
    // defined inside finally, nested try/finally, nested def negative
    // control) live in `crates/pycc_hir/src/tests.rs` instead, alongside
    // the existing `break`/`continue`-outside-loop `L0001` tests they
    // mirror.
    let src = "\
def f() -> int:
    try:
        x = 1
    except ValueError:
        y = 2
    finally:
        return 3
";
    let module = pycc_parser::parse(src).expect("test fixture must parse");
    let err = pycc_hir::lower_checked(&module).unwrap_err();
    assert_eq!(err.code, "L0001");
    assert!(err.message.contains("'return' in a 'finally' block"));
}

#[test]
fn try_with_return_in_else() {
    let src = "\
def f() -> int:
    try:
        x = 1
    except ValueError:
        y = 2
    else:
        return 3
    return 4
";
    parse_check(src).expect("check should succeed");
}

#[test]
fn try_with_binding_in_body() {
    let src = "\
def f() -> int:
    x = 0
    try:
        x = 1
    except ValueError:
        y = 2
    else:
        z = 3
    finally:
        w = 4
    return x
";
    parse_check(src).expect("check should succeed");
}

#[test]
fn raise_in_function_checks_successfully() {
    let src = "\
def f() -> int:
    raise ValueError(\"bad\")
";
    parse_check(src).expect("an always-raising function cannot fall through");
}

#[test]
fn generic_call_inside_try_checks_successfully() {
    let src = "\
class C[T]:
    def __init__(self, x: T):
        self.x = x
def f() -> int:
    try:
        c = C[int](1)
        return c.x
    except ValueError:
        return 0
";
    parse_check_resolve(src).expect("check should succeed");
}

#[test]
fn generic_call_inside_raise_in_handler() {
    let src = "\
class C[T]:
    def __init__(self, x: T):
        self.x = x
def f() -> int:
    try:
        c = C[int](1)
        return c.x
    except ValueError:
        raise RuntimeError(\"err\")
";
    parse_check_resolve(src)
        .expect("the body returns and the handler raises, so every path terminates");
}

#[test]
fn try_with_enum_loop_unrolls() {
    let src = "\
from enum import Enum
class Color(Enum):
    RED = 1
    GREEN = 2
x = 0
try:
    for c in Color:
        x = x + 1
except ValueError:
    pass
";
    parse_check_resolve(src).expect("check should succeed");
}

#[test]
fn try_star_with_enum_loop_unrolls() {
    // Mirrors `try_with_enum_loop_unrolls` but for `except*` (#542): covers
    // `check_stmt`'s module-level `HirStmt::TryStar` arm and the
    // `HirStmt::TryStar` arm of `unroll_enum_loops_in_stmts` (which, before
    // #542's coverage pass, silently fell through to the catch-all `other`
    // branch and would have left a nested enum `for` loop un-unrolled).
    let src = "\
from enum import Enum
class Color(Enum):
    RED = 1
    GREEN = 2
x = 0
try:
    for c in Color:
        x = x + 1
except* ValueError:
    pass
";
    parse_check_resolve(src).expect("check should succeed");
}

#[test]
fn non_enum_for_list_inside_try_unrolls() {
    // Covers the `else` branch of `unroll_enum_loops_in_stmts`'s
    // `ForList` arm — a non-enum `for` loop inside a `try` body is
    // kept as-is (not unrolled), but the body is recursed into.
    let src = "\
from enum import Enum
class Color(Enum):
    RED = 1
    GREEN = 2
xs = [1, 2, 3]
x = 0
try:
    for y in xs:
        x = x + y
except ValueError:
    pass
";
    parse_check_resolve(src).expect("check should succeed");
}

#[test]
fn try_except_with_private_helper() {
    let src = "\
def _helper() -> int:
    x = 0
    try:
        x = 1
    except ValueError:
        y = 2
    return x
_helper()
";
    parse_check(src).expect("check should succeed");
}

#[test]
fn raise_with_private_helper() {
    let src = "\
def _helper() -> int:
    raise ValueError(\"bad\")
_helper()
";
    parse_check(src).expect("an always-raising private helper cannot fall through");
}
