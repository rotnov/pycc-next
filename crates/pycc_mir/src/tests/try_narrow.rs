//! #1476: the narrowing overlay across a `try` statement
//! (`stmt::try_stmt`), position by position: `else` continues from the
//! body, `finally` starts from the pre-`try` overlay less every path's
//! rebindings, and after the statement only what every fall-through path
//! keeps (less what `finally` rebinds) stays narrowed. The overlay is
//! shared by `Optional` and `object` narrowing; these tests drive it with
//! `Optional` guards on hand-built HIR, and
//! `tests/issue_1476_isinstance_narrowing.rs` runs the `object` shapes
//! against CPython.

use crate::*;
use pycc_hir::{CmpOpKind, HirExceptHandler, HirExpr, HirItem, HirModule, HirStmt, Ty};

fn is_none_return() -> HirStmt {
    HirStmt::If {
        test: HirExpr::Compare {
            op: CmpOpKind::Is,
            left: Box::new(HirExpr::Name("x".to_string())),
            right: Box::new(HirExpr::NoneLiteral),
        },
        body: vec![HirStmt::Return(None)],
        orelse: vec![],
    }
}

fn print_x() -> HirStmt {
    HirStmt::ExprStmt(HirExpr::Call {
        callee: "print".to_string(),
        args: vec![HirExpr::Name("x".to_string())],
    })
}

fn rebind_x() -> HirStmt {
    HirStmt::Assign {
        target: "x".to_string(),
        value: HirExpr::IntLiteral(5),
    }
}

fn handler(body: Vec<HirStmt>) -> HirExceptHandler {
    HirExceptHandler {
        exc_type: Some(vec!["ValueError".to_string()]),
        name: None,
        body,
    }
}

/// Lowers `def f(x: int | None) -> None: <body>` and returns its MIR body.
fn lower(body: Vec<HirStmt>) -> Vec<MirStmt> {
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![HirItem::Function {
            name: "f".to_string(),
            params: vec![("x".to_string(), Ty::Optional(Box::new(Ty::Int)))],
            return_ty: Ty::None,
            body,
        }],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    let mir = build(&hir);
    let MirItem::Function { body, .. } = mir.items.into_iter().next().unwrap() else {
        panic!("expected the only item to be the lowered function");
    };
    body
}

/// Whether `stmt` is `print(x)` reading `x` through an `OptionalUnwrap`.
fn reads_narrowed(stmt: &MirStmt) -> bool {
    let MirStmt::ExprStmt(MirExpr::Call { args, .. }) = stmt else {
        panic!("expected a lowered `print(x)`, got {stmt:?}");
    };
    match &args[0] {
        MirExpr::OptionalUnwrap(..) => true,
        MirExpr::Name { .. } => false,
        other => panic!("unexpected argument {other:?}"),
    }
}

/// The `else`, `finally`, and after-`try` reads of `x` in `body`, whose
/// `try` statement is at `index`.
fn reads(body: &[MirStmt], index: usize) -> (bool, bool, bool) {
    let (MirStmt::Try {
        orelse, finalbody, ..
    }
    | MirStmt::TryStar {
        orelse, finalbody, ..
    }) = &body[index]
    else {
        panic!("expected a lowered `try` at {index}");
    };
    (
        reads_narrowed(&orelse[0]),
        reads_narrowed(&finalbody[0]),
        reads_narrowed(&body[index + 1]),
    )
}

fn try_stmt(
    star: bool,
    body: Vec<HirStmt>,
    handlers: Vec<HirExceptHandler>,
    finalbody: Vec<HirStmt>,
) -> HirStmt {
    let orelse = vec![print_x()];
    if star {
        HirStmt::TryStar {
            body,
            handlers,
            orelse,
            finalbody,
        }
    } else {
        HirStmt::Try {
            body,
            handlers,
            orelse,
            finalbody,
        }
    }
}

/// A guard in the body narrows `else`, and narrows the code after the `try`
/// when every handler returns; `finally` can run after an exception before
/// the guard, so it is not narrowed. The same for `except*`.
#[test]
fn a_body_guard_narrows_else_and_the_code_after_the_try_when_every_handler_returns() {
    for star in [false, true] {
        let body = lower(vec![
            try_stmt(
                star,
                vec![is_none_return()],
                vec![handler(vec![HirStmt::Return(None)])],
                vec![print_x()],
            ),
            print_x(),
        ]);
        assert_eq!(reads(&body, 0), (true, false, true), "star: {star}");
    }
}

/// A handler that falls through reaches the code after the `try` without
/// the body's guard, so that code is not narrowed.
#[test]
fn a_falling_through_handler_ends_a_body_guard_at_the_try() {
    let body = lower(vec![
        try_stmt(
            false,
            vec![is_none_return()],
            vec![handler(vec![])],
            vec![print_x()],
        ),
        print_x(),
    ]);
    assert_eq!(reads(&body, 0), (true, false, false));
}

/// A body rebinding ends a narrowing from before the `try` in `else` and
/// `finally`; the handler returns, so only the `else` path falls through,
/// and it no longer narrows `x` either.
#[test]
fn a_body_rebinding_ends_an_outer_narrowing_in_else_finally_and_after() {
    let body = lower(vec![
        is_none_return(),
        try_stmt(
            false,
            vec![rebind_x()],
            vec![handler(vec![HirStmt::Return(None)])],
            vec![print_x()],
        ),
        print_x(),
    ]);
    assert_eq!(reads(&body, 1), (false, false, false));
}

/// A body that rebinds the name and then guards it again ends `else` and the
/// code after the `try` narrowed, but not `finally`, which runs between the
/// rebinding and the guard's `return`.
#[test]
fn a_body_that_rebinds_and_guards_again_leaves_finally_unnarrowed() {
    let body = lower(vec![
        is_none_return(),
        try_stmt(
            false,
            vec![rebind_x(), is_none_return()],
            vec![handler(vec![HirStmt::Return(None)])],
            vec![print_x()],
        ),
        print_x(),
    ]);
    assert_eq!(reads(&body, 1), (true, false, true));
}

/// A handler rebinding ends an outer narrowing in `finally`, and after the
/// `try` only when that handler falls through.
#[test]
fn a_handler_rebinding_ends_an_outer_narrowing_after_the_try_only_when_it_falls_through() {
    for (handler_body, after) in [
        (vec![rebind_x(), HirStmt::Return(None)], true),
        (vec![rebind_x()], false),
    ] {
        let body = lower(vec![
            is_none_return(),
            try_stmt(false, vec![], vec![handler(handler_body)], vec![print_x()]),
            print_x(),
        ]);
        assert_eq!(reads(&body, 1), (true, false, after));
    }
}

/// A `finally` rebinding ends an outer narrowing after the `try`; one that
/// rebinds nothing keeps it.
#[test]
fn a_finally_rebinding_ends_an_outer_narrowing_after_the_try() {
    for (finalbody, after) in [
        (vec![print_x(), rebind_x()], false),
        (vec![print_x()], true),
    ] {
        let body = lower(vec![
            is_none_return(),
            try_stmt(false, vec![], vec![], finalbody),
            print_x(),
        ]);
        assert_eq!(reads(&body, 1), (true, true, after));
    }
}

/// With no path falling through, the `finally` end state stands for the
/// (unreachable) code after the `try`.
#[test]
fn with_no_path_falling_through_the_finally_end_state_stands() {
    let body = lower(vec![
        is_none_return(),
        try_stmt(
            false,
            vec![HirStmt::Return(None)],
            vec![handler(vec![HirStmt::Return(None)])],
            vec![print_x()],
        ),
        print_x(),
    ]);
    assert_eq!(reads(&body, 1), (true, true, true));
}
