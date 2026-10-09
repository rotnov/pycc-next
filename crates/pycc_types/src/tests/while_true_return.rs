//! #1370: a `while` loop whose test is a constant true value never
//! completes normally, so a function that ends in one and leaves it only
//! through `return` or a raise does not fall off its end. Any other loop
//! test keeps the `T0022` "can exit without returning" diagnostic.

use super::*;

fn code_of(src: &str) -> &'static str {
    parse_check(src).unwrap_err().code
}

fn loop_returning(test: HirExpr) -> Vec<HirStmt> {
    vec![HirStmt::While {
        test,
        body: vec![HirStmt::Return(Some(HirExpr::IntLiteral(1)))],
    }]
}

/// The issue's own program: `while True:` left only by `return`.
#[test]
fn a_while_true_left_only_by_return_does_not_fall_through() {
    let src = "\
def f(x: int) -> int:
    while True:
        if x > 0:
            return x
        x += 1
";
    assert!(parse_check(src).is_ok());
}

/// `while 1:` is the other spelling CPython treats as constant true, and a
/// raise is as terminal as a return.
#[test]
fn while_one_and_a_raising_exit_do_not_fall_through() {
    let src = "\
def f(x: int) -> int:
    while 1:
        x -= 1
        if x < 0:
            raise ValueError(\"negative\")
        if x == 3:
            return x
";
    assert!(parse_check(src).is_ok());
}

/// A constant-true loop nested in one branch of an `if` makes that branch
/// terminal; the other branch still has to return on its own.
#[test]
fn a_while_true_inside_one_branch_counts_for_that_branch_only() {
    let both = "\
def f(c: bool) -> int:
    if c:
        while True:
            return 1
    else:
        return 2
";
    assert!(parse_check(both).is_ok());
    let one = "\
def f(c: bool) -> int:
    if c:
        while True:
            return 1
    else:
        pass
";
    assert_eq!(code_of(one), "T0022");
}

/// A loop whose test may be false keeps `T0022`, even when its body always
/// returns: `while False:` and `while 0:` never run the body, and a
/// computed, string, or float test is not recognized as constant.
#[test]
fn any_other_loop_test_still_falls_through() {
    for test in ["False", "0", "x", "x > 0", "\"a\"", "1.0"] {
        let src = format!("def f(x: int) -> int:\n    while {test}:\n        return 1\n");
        assert_eq!(code_of(&src), "T0022", "while {test}:");
    }
}

/// A try body that ends in a constant-true loop never falls through the
/// `try`, so only the handler's path reaches the statement after it: a name
/// the handler binds is definitely bound there.
#[test]
fn a_try_body_ending_in_while_true_leaves_only_the_handler_path() {
    let src = "\
def f(x: int) -> int:
    try:
        while True:
            return x
    except ValueError:
        y = 1
    return y
";
    assert!(parse_check(src).is_ok());
}

/// The predicate itself, on each test shape the two helpers distinguish.
#[test]
fn block_always_returns_recognizes_only_constant_true_loop_tests() {
    assert!(block_always_returns(&loop_returning(HirExpr::BoolLiteral(
        true
    ))));
    assert!(block_always_returns(&loop_returning(HirExpr::IntLiteral(
        1
    ))));
    assert!(block_always_returns(&loop_returning(HirExpr::IntLiteral(
        -7
    ))));
    assert!(!block_always_returns(&loop_returning(
        HirExpr::BoolLiteral(false)
    )));
    assert!(!block_always_returns(&loop_returning(HirExpr::IntLiteral(
        0
    ))));
    assert!(!block_always_returns(&loop_returning(HirExpr::Name(
        "x".to_string()
    ))));
}
