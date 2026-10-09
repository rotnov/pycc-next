//! Context validation for the body of an `if TYPE_CHECKING:` guard (#905).
//!
//! #790 constant-folds a `TYPE_CHECKING`-guarded body away before either
//! `lower_expr` or `lower_body` ever sees it, which is what lets the guard
//! wrap constructs this compiler does not implement. The side effect it did
//! not intend is that the fold also swallowed the *context* checks
//! `lower_stmt` performs -- a guarded `return` inside a `finally`, a guarded
//! `break` with no loop, a guarded `yield` at module scope -- all of which
//! CPython rejects as a `SyntaxError` at compile time, whether or not the
//! branch ever runs. This module re-checks the guarded body for exactly
//! those violations.
//!
//! # Contract
//!
//! Report only `context_invalid` (`L0001`) diagnostics. Stay silent
//! wherever `lower_stmt` would have produced `unsupported` (`C0001`). The
//! guard's purpose is to let a body contain constructs pycc does not
//! implement; a walker that reported `C0001` would defeat #790 entirely.
//!
//! Two mechanisms keep that contract from drifting away from `lower_stmt`:
//!
//! 1. **Shared predicates.** The `return`/`break`/`continue` rules live here
//!    ([`return_context_violation`], [`break_context_violation`],
//!    [`continue_context_violation`]) and are called by `lower_stmt`'s own
//!    arms as well as by this walker, so the eight messages exist once. The
//!    `yield`/`yield from` rules are not restated at all: a bare expression
//!    statement is handed to the real [`lower_expr`], and only an `L0001`
//!    from it is forwarded. `expr.rs` raises `context_invalid` in exactly
//!    two places (`'yield' outside function` and `'yield from' outside
//!    function`), so that delegation can neither miss a violation nor
//!    over-report a capability gap.
//! 2. **The recursion gate.** A nested body is walked only after the real
//!    lowering helpers accept the *non-body* parts of the statement that
//!    encloses it -- a `while`/`if` test through `lower_expr`, a `for`'s
//!    `else`/target/iterable through the same shape checks `lower_for`
//!    applies, an `except` handler's type through the same shape check
//!    `lower_except_handler` applies. Where real lowering would abort with a
//!    `C0001` before reaching the body, this walker stops too, so it can
//!    never report an `L0001` that unguarded code would not have reached.
//!
//! # Deliberate silences
//!
//! The walk is syntactic and statement-level, so several shapes stay
//! accepted under the guard (each is recorded as a residual gap in
//! `docs/decisions/D-223-*.md` and `docs/RUNTIME.md`):
//!
//! - a module-scope `return` (CPython's fatal error there is `'return'
//!   outside function`, which is pycc's separate `T0024` pass);
//! - a `yield` that is not the whole of an expression statement (`x =
//!   (yield 3)` is a `Stmt::Assign`, which this walker does not visit);
//! - the body of a nested `def`/`class` under the guard;
//! - a `from __future__ import ...` (#919's own `L0001`s, out of scope
//!   there);
//! - `match` case bodies, and every body whose enclosing statement's
//!   non-body parts do not lower (see the recursion gate above).

use super::ExceptStarCtx;
use crate::expr::keyword_bind::SignatureTable;
use crate::expr::{lower_expr, lower_range_call};
use crate::{ImportBinding, context_invalid};
use pycc_ast::{ExceptHandler, Expr, Stmt};
use pycc_diag::Diagnostic;

/// The `L0001` message a `return` earns in this context, or `None` when
/// `lower_stmt` would not reject it. Shared with `lower_stmt`'s
/// `Stmt::Return` arm; see that arm for why each conjunct is what it is.
pub(super) fn return_context_violation(
    in_function: bool,
    in_finally: bool,
    except_star: ExceptStarCtx,
) -> Option<&'static str> {
    if except_star != ExceptStarCtx::Outside && in_function {
        return Some("'return' in an 'except*' block");
    }
    if in_finally && in_function {
        return Some("'return' in a 'finally' block");
    }
    None
}

/// The `L0001` message a `break` earns in this context, or `None` when the
/// `break` is valid Python that `lower_stmt` rejects only as a `C0001`
/// capability gap. Shared with `lower_stmt`'s `Stmt::Break` arm.
pub(super) fn break_context_violation(
    in_loop: bool,
    in_finally: bool,
    except_star: ExceptStarCtx,
) -> Option<&'static str> {
    if except_star == ExceptStarCtx::InsideUnshielded {
        return Some("'break' in an 'except*' block");
    }
    if in_finally && in_loop {
        return Some("'break' in a 'finally' block");
    }
    if !in_loop {
        return Some("'break' outside loop");
    }
    None
}

/// The `L0001` message a `continue` earns in this context, or `None` when
/// the `continue` is valid Python that `lower_stmt` rejects only as a
/// `C0001` capability gap. Shared with `lower_stmt`'s `Stmt::Continue` arm.
pub(super) fn continue_context_violation(
    in_loop: bool,
    in_finally: bool,
    except_star: ExceptStarCtx,
) -> Option<&'static str> {
    if except_star == ExceptStarCtx::InsideUnshielded {
        return Some("'continue' in an 'except*' block");
    }
    if in_finally && in_loop {
        return Some("'continue' in a 'finally' block");
    }
    if !in_loop {
        return Some("'continue' not properly in loop");
    }
    None
}

/// The context a guarded statement sits in. These are exactly `lower_stmt`'s
/// own threaded parameters, bundled so the walk can update one of them at a
/// time with struct-update syntax.
#[derive(Clone, Copy)]
struct Ctx<'a> {
    in_loop: bool,
    in_function: bool,
    in_finally: bool,
    except_star: ExceptStarCtx,
    class_name: Option<&'a str>,
    /// The module's import table (Part 1 of #883, #962), forwarded to
    /// `lower_expr` so an aliased stdlib receiver inside a guarded body
    /// lowers exactly as it would outside one.
    imports: &'a [ImportBinding],
    /// The module's keyword-bindable signature table (Part 1 of #884,
    /// #1125), forwarded alongside `imports` so a keyword call inside a
    /// guarded body lowers exactly as it would outside one. This field and
    /// `for_iterable_lowers`'s own parameter move in lockstep with
    /// `for_loop.rs`'s `lower_range_call` call: both re-lower the same
    /// expression shapes and must see the same table.
    signatures: &'a SignatureTable,
}

/// Re-checks a `TYPE_CHECKING`-guarded body for the context violations the
/// #790 fold would otherwise swallow. The flags are the fold site's own
/// incoming context, passed through unchanged: the guard itself is an `if`,
/// and an `if` neither enters a loop nor a function nor leaves a `finally`.
#[allow(clippy::too_many_arguments)]
pub(super) fn check_guarded_body(
    body: &[Stmt],
    in_loop: bool,
    in_function: bool,
    in_finally: bool,
    except_star: ExceptStarCtx,
    class_name: Option<&str>,
    imports: &[ImportBinding],
    signatures: &SignatureTable,
) -> Result<(), Diagnostic> {
    check_body(
        body,
        Ctx {
            in_loop,
            in_function,
            in_finally,
            except_star,
            class_name,
            imports,
            signatures,
        },
    )
}

fn check_body(body: &[Stmt], ctx: Ctx<'_>) -> Result<(), Diagnostic> {
    for stmt in body {
        check_stmt(stmt, ctx)?;
    }
    Ok(())
}

fn check_stmt(stmt: &Stmt, ctx: Ctx<'_>) -> Result<(), Diagnostic> {
    match stmt {
        Stmt::Return(_) => {
            if let Some(message) =
                return_context_violation(ctx.in_function, ctx.in_finally, ctx.except_star)
            {
                return Err(context_invalid(message, pycc_ast::stmt_range(stmt)));
            }
        }
        Stmt::Break(_) => {
            if let Some(message) =
                break_context_violation(ctx.in_loop, ctx.in_finally, ctx.except_star)
            {
                return Err(context_invalid(message, pycc_ast::stmt_range(stmt)));
            }
        }
        Stmt::Continue(_) => {
            if let Some(message) =
                continue_context_violation(ctx.in_loop, ctx.in_finally, ctx.except_star)
            {
                return Err(context_invalid(message, pycc_ast::stmt_range(stmt)));
            }
        }
        Stmt::Expr(expr_stmt) => {
            // Delegation rather than a restatement of `expr.rs`'s two
            // `yield` rules: this is literally what `lower_stmt`'s own
            // `Stmt::Expr` arm does, so the message and the span are exact
            // by construction. Only an `L0001` is forwarded -- a `C0001`
            // from an unsupported expression is precisely what the guard
            // exists to permit.
            if let Err(diagnostic) = lower_expr(
                &expr_stmt.value,
                ctx.in_function,
                ctx.class_name,
                ctx.imports,
                ctx.signatures,
            ) && diagnostic.code == "L0001"
            {
                return Err(diagnostic);
            }
        }
        Stmt::If(if_stmt) => return check_if(if_stmt, ctx),
        Stmt::While(while_stmt) => {
            if !while_stmt.orelse.is_empty() {
                return Ok(());
            }
            if lower_expr(
                &while_stmt.test,
                ctx.in_function,
                ctx.class_name,
                ctx.imports,
                ctx.signatures,
            )
            .is_err()
            {
                return Ok(());
            }
            return check_body(&while_stmt.body, loop_ctx(ctx));
        }
        Stmt::For(for_stmt) => {
            // `lower_for` checks `is_async` *first*, before the `else`,
            // target and iterable gates below -- so an `async for` is a
            // violation even when its own iterable would never lower.
            if for_stmt.is_async {
                return Err(context_invalid(
                    "'async for' outside async function",
                    for_stmt.range,
                ));
            }
            if !for_stmt.orelse.is_empty() {
                return Ok(());
            }
            if !matches!(for_stmt.target.as_ref(), Expr::Name(_)) {
                return Ok(());
            }
            if !for_iterable_lowers(
                &for_stmt.iter,
                ctx.in_function,
                ctx.class_name,
                ctx.imports,
                ctx.signatures,
            ) {
                return Ok(());
            }
            return check_body(&for_stmt.body, loop_ctx(ctx));
        }
        Stmt::Try(try_stmt) => {
            check_body(&try_stmt.body, ctx)?;
            let handler_ctx = Ctx {
                except_star: if try_stmt.is_star {
                    ExceptStarCtx::InsideUnshielded
                } else {
                    ctx.except_star
                },
                ..ctx
            };
            for handler in &try_stmt.handlers {
                let ExceptHandler::ExceptHandler(handler) = handler;
                if !except_handler_type_lowers(handler.type_.as_deref()) {
                    // `lower_stmt` collects the handlers before it reaches
                    // `orelse`/`finalbody`, so a handler type it rejects
                    // aborts the whole `try` -- stop here rather than
                    // walking clauses real lowering would never see.
                    return Ok(());
                }
                check_body(&handler.body, handler_ctx)?;
            }
            check_body(&try_stmt.orelse, ctx)?;
            return check_body(
                &try_stmt.finalbody,
                Ctx {
                    in_finally: true,
                    ..ctx
                },
            );
        }
        // Everything else is either context-neutral (assignments, imports,
        // `pass`, `raise`, ...) or a body this walker deliberately does not
        // enter: `match` cases, and nested `def`/`class` bodies, which start
        // a fresh context of their own. See the module doc comment.
        _ => {}
    }
    Ok(())
}

/// The context for a `while`/`for` body: inside a loop, shielded from an
/// enclosing `finally`, and with the `except*` context demoted -- exactly
/// what `lower_stmt`'s `Stmt::While` arm and `lower_for` thread.
fn loop_ctx(ctx: Ctx<'_>) -> Ctx<'_> {
    Ctx {
        in_loop: true,
        in_finally: false,
        except_star: ctx.except_star.shielded_by_loop(),
        ..ctx
    }
}

fn check_if(if_stmt: &pycc_ast::StmtIf, ctx: Ctx<'_>) -> Result<(), Diagnostic> {
    if lower_expr(
        &if_stmt.test,
        ctx.in_function,
        ctx.class_name,
        ctx.imports,
        ctx.signatures,
    )
    .is_err()
    {
        return Ok(());
    }
    check_body(&if_stmt.body, ctx)?;
    for clause in &if_stmt.elif_else_clauses {
        // An `else` has no test; an `elif` whose test does not lower aborts
        // the rest of the chain in `lower_elif_else_clauses`, so it aborts
        // the walk of the rest of the chain here too.
        if let Some(test) = &clause.test
            && lower_expr(
                test,
                ctx.in_function,
                ctx.class_name,
                ctx.imports,
                ctx.signatures,
            )
            .is_err()
        {
            return Ok(());
        }
        check_body(&clause.body, ctx)?;
    }
    Ok(())
}

/// Whether `lower_for` would accept this iterable and go on to lower the
/// loop body. Mirrors the iterable arms of `lower_for`, including
/// `lower_range_call`'s own rejections of the `range(...)` arguments.
///
/// Lockstep with `for_loop::lower_for_range` (Part 1 of #884, #1125): both
/// lower the same `range(...)` call and must be handed the same
/// `signatures` table.
fn for_iterable_lowers(
    iter: &Expr,
    in_function: bool,
    class_name: Option<&str>,
    imports: &[ImportBinding],
    signatures: &SignatureTable,
) -> bool {
    match iter {
        Expr::Name(_) => true,
        Expr::Call(call) => {
            matches!(call.func.as_ref(), Expr::Name(callee) if callee.id.as_str() == "range")
                && call.arguments.keywords.is_empty()
                && lower_range_call(call, in_function, class_name, imports, signatures).is_ok()
        }
        _ => false,
    }
}

/// Whether `lower_except_handler` would accept this handler's exception type
/// and go on to lower the handler body: absent, a bare name, or a non-empty
/// tuple of bare names.
fn except_handler_type_lowers(exception_type: Option<&Expr>) -> bool {
    match exception_type {
        None | Some(Expr::Name(_)) => true,
        Some(Expr::Tuple(tuple)) => {
            !tuple.elts.is_empty() && tuple.elts.iter().all(|elt| matches!(elt, Expr::Name(_)))
        }
        Some(_) => false,
    }
}

#[cfg(test)]
mod tests {
    //! Every branch of the walker is exercised through the real public
    //! entry point (`pycc_parser::parse` + `lower_checked`) rather than by
    //! hand-built AST nodes, and in-crate rather than through the CLI:
    //! `cargo llvm-cov` groups regions by definition site per instantiation,
    //! so an integration test that spawns the `pycc` binary contributes
    //! nothing to this crate's own region coverage (D-014, see
    //! `docs/TESTING.md`).

    /// Asserts the guarded body produces exactly this `L0001` message.
    fn assert_guarded_context_error(source: &str, expected_message: &str) {
        let module = pycc_parser::parse(source).expect("test fixture must parse");
        let diagnostic = crate::lower_checked(&module).expect_err("lowering must fail");

        assert_eq!(diagnostic.code, "L0001");
        assert_eq!(diagnostic.message, expected_message);
        assert!(diagnostic.span.is_some());
    }

    /// Asserts the guarded body is folded away silently -- the #790
    /// contract: the walker reports nothing where `lower_stmt` would have
    /// produced a `C0001`, and nothing where the context is legal.
    fn assert_guarded_body_accepted(source: &str) {
        let module = pycc_parser::parse(source).expect("test fixture must parse");
        crate::lower_checked(&module).expect("lowering must succeed");
    }

    // -- The eleven context violations the #790 fold used to swallow --

    #[test]
    fn a_guarded_return_in_a_finally_block_is_context_invalid() {
        assert_guarded_context_error(
            "def f() -> int:\n    try:\n        pass\n    finally:\n        if TYPE_CHECKING:\n            return 1\n    return 0\n",
            "'return' in a 'finally' block",
        );
    }

    #[test]
    fn a_guarded_return_in_an_except_star_block_is_context_invalid() {
        assert_guarded_context_error(
            "def f() -> int:\n    try:\n        pass\n    except* ValueError:\n        if TYPE_CHECKING:\n            return 1\n    return 0\n",
            "'return' in an 'except*' block",
        );
    }

    #[test]
    fn a_guarded_break_outside_a_loop_is_context_invalid() {
        assert_guarded_context_error("if TYPE_CHECKING:\n    break\n", "'break' outside loop");
    }

    #[test]
    fn a_guarded_break_in_a_finally_block_is_context_invalid() {
        assert_guarded_context_error(
            "while True:\n    try:\n        pass\n    finally:\n        if TYPE_CHECKING:\n            break\n",
            "'break' in a 'finally' block",
        );
    }

    #[test]
    fn a_guarded_break_in_an_except_star_block_is_context_invalid() {
        assert_guarded_context_error(
            "try:\n    pass\nexcept* ValueError:\n    if TYPE_CHECKING:\n        break\n",
            "'break' in an 'except*' block",
        );
    }

    #[test]
    fn a_guarded_continue_outside_a_loop_is_context_invalid() {
        assert_guarded_context_error(
            "if TYPE_CHECKING:\n    continue\n",
            "'continue' not properly in loop",
        );
    }

    #[test]
    fn a_guarded_continue_in_a_finally_block_is_context_invalid() {
        assert_guarded_context_error(
            "while True:\n    try:\n        pass\n    finally:\n        if TYPE_CHECKING:\n            continue\n",
            "'continue' in a 'finally' block",
        );
    }

    #[test]
    fn a_guarded_continue_in_an_except_star_block_is_context_invalid() {
        assert_guarded_context_error(
            "try:\n    pass\nexcept* ValueError:\n    if TYPE_CHECKING:\n        continue\n",
            "'continue' in an 'except*' block",
        );
    }

    #[test]
    fn a_guarded_yield_outside_a_function_is_context_invalid() {
        // Delegated to the real `lower_expr` rather than restated here, so
        // the message and span are `expr.rs`'s own.
        assert_guarded_context_error(
            "if TYPE_CHECKING:\n    yield 1\n",
            "'yield' outside function",
        );
    }

    #[test]
    fn a_guarded_yield_from_outside_a_function_is_context_invalid() {
        assert_guarded_context_error(
            "if TYPE_CHECKING:\n    yield from xs\n",
            "'yield from' outside function",
        );
    }

    #[test]
    fn a_guarded_async_for_is_context_invalid() {
        // `lower_for` checks `is_async` before its own `else`/target/
        // iterable gates, so the walker must too -- this loop's iterable
        // would not otherwise be recursed into.
        assert_guarded_context_error(
            "if TYPE_CHECKING:\n    async for x in [1]:\n        pass\n",
            "'async for' outside async function",
        );
    }

    // -- The second fold site: `elif TYPE_CHECKING:` --

    #[test]
    fn an_elif_type_checking_guard_is_checked_too() {
        assert_guarded_context_error(
            "if True:\n    x = 1\nelif TYPE_CHECKING:\n    break\n",
            "'break' outside loop",
        );
    }

    // -- Recursion into nested bodies --

    #[test]
    fn a_violation_nested_under_a_guarded_if_is_reported() {
        assert_guarded_context_error(
            "if TYPE_CHECKING:\n    if True:\n        break\n",
            "'break' outside loop",
        );
    }

    #[test]
    fn a_violation_nested_under_a_guarded_elif_is_reported() {
        assert_guarded_context_error(
            "if TYPE_CHECKING:\n    if True:\n        x = 1\n    elif False:\n        break\n",
            "'break' outside loop",
        );
    }

    #[test]
    fn a_violation_nested_under_a_guarded_else_is_reported() {
        assert_guarded_context_error(
            "if TYPE_CHECKING:\n    if True:\n        x = 1\n    else:\n        break\n",
            "'break' outside loop",
        );
    }

    #[test]
    fn a_violation_nested_under_a_guarded_while_is_reported() {
        // The `while` body resets `in_finally` and sets `in_loop`, so the
        // violation has to come from a `finally` entered inside the loop.
        assert_guarded_context_error(
            "def f() -> int:\n    if TYPE_CHECKING:\n        while True:\n            try:\n                pass\n            finally:\n                return 1\n    return 0\n",
            "'return' in a 'finally' block",
        );
    }

    #[test]
    fn a_violation_nested_under_a_guarded_for_over_a_name_is_reported() {
        assert_guarded_context_error(
            "def f() -> int:\n    if TYPE_CHECKING:\n        for x in xs:\n            try:\n                pass\n            finally:\n                return 1\n    return 0\n",
            "'return' in a 'finally' block",
        );
    }

    #[test]
    fn a_violation_nested_under_a_guarded_for_over_a_range_is_reported() {
        assert_guarded_context_error(
            "def f() -> int:\n    if TYPE_CHECKING:\n        for i in range(3):\n            try:\n                pass\n            finally:\n                return 1\n    return 0\n",
            "'return' in a 'finally' block",
        );
    }

    #[test]
    fn a_violation_in_a_guarded_try_body_is_reported() {
        assert_guarded_context_error(
            "if TYPE_CHECKING:\n    try:\n        break\n    except ValueError:\n        pass\n",
            "'break' outside loop",
        );
    }

    #[test]
    fn a_violation_in_a_guarded_except_handler_body_is_reported() {
        assert_guarded_context_error(
            "if TYPE_CHECKING:\n    try:\n        pass\n    except ValueError:\n        break\n",
            "'break' outside loop",
        );
    }

    #[test]
    fn a_violation_in_a_guarded_bare_except_handler_body_is_reported() {
        assert_guarded_context_error(
            "if TYPE_CHECKING:\n    try:\n        pass\n    except:\n        break\n",
            "'break' outside loop",
        );
    }

    #[test]
    fn a_violation_in_a_guarded_multi_type_except_handler_body_is_reported() {
        assert_guarded_context_error(
            "if TYPE_CHECKING:\n    try:\n        pass\n    except (ValueError, TypeError):\n        break\n",
            "'break' outside loop",
        );
    }

    #[test]
    fn a_violation_in_a_guarded_except_star_handler_body_is_reported() {
        // The walker's own `try*`: its handlers are entered with
        // `ExceptStarCtx::InsideUnshielded`, exactly as `lower_stmt` does.
        assert_guarded_context_error(
            "if TYPE_CHECKING:\n    try:\n        pass\n    except* ValueError:\n        break\n",
            "'break' in an 'except*' block",
        );
    }

    #[test]
    fn a_violation_in_a_guarded_try_else_body_is_reported() {
        assert_guarded_context_error(
            "if TYPE_CHECKING:\n    try:\n        pass\n    except ValueError:\n        pass\n    else:\n        break\n",
            "'break' outside loop",
        );
    }

    // -- The deliberate silences (the #790 contract) --

    #[test]
    fn a_guarded_body_with_an_unsupported_statement_stays_silent() {
        // The regression guard for #790 itself: a capability gap under the
        // guard must never become a diagnostic.
        assert_guarded_body_accepted("if TYPE_CHECKING:\n    x = lambda: 1\n");
    }

    #[test]
    fn a_guarded_expression_statement_that_does_not_lower_stays_silent() {
        assert_guarded_body_accepted("if TYPE_CHECKING:\n    lambda: 1\n");
    }

    #[test]
    fn a_guarded_expression_statement_that_lowers_is_accepted() {
        assert_guarded_body_accepted("if TYPE_CHECKING:\n    print(1)\n");
    }

    #[test]
    fn a_guarded_return_at_module_scope_stays_silent() {
        // Residual gap: CPython's fatal error here is `'return' outside
        // function`, which is pycc's separate `T0024` pass.
        assert_guarded_body_accepted("if TYPE_CHECKING:\n    return 1\n");
    }

    #[test]
    fn a_legal_guarded_return_inside_a_function_stays_silent() {
        assert_guarded_body_accepted(
            "def f() -> int:\n    if TYPE_CHECKING:\n        return 1\n    return 0\n",
        );
    }

    #[test]
    fn a_guarded_break_with_a_real_enclosing_loop_stays_silent() {
        // Valid Python that pycc does not implement yet -- a `C0001`, which
        // the guard exists to permit.
        assert_guarded_body_accepted("while True:\n    if TYPE_CHECKING:\n        break\n");
    }

    #[test]
    fn a_guarded_continue_with_a_real_enclosing_loop_stays_silent() {
        assert_guarded_body_accepted("while True:\n    if TYPE_CHECKING:\n        continue\n");
    }

    #[test]
    fn a_break_inside_a_loop_entered_under_the_guard_stays_silent() {
        assert_guarded_body_accepted("if TYPE_CHECKING:\n    while True:\n        break\n");
    }

    #[test]
    fn a_guarded_while_else_body_is_not_walked() {
        assert_guarded_body_accepted(
            "if TYPE_CHECKING:\n    while True:\n        break\n    else:\n        pass\n",
        );
    }

    #[test]
    fn a_guarded_while_whose_test_does_not_lower_is_not_walked() {
        assert_guarded_body_accepted("if TYPE_CHECKING:\n    while (lambda: 1):\n        break\n");
    }

    #[test]
    fn a_guarded_nested_if_whose_test_does_not_lower_is_not_walked() {
        assert_guarded_body_accepted("if TYPE_CHECKING:\n    if (lambda: 1):\n        break\n");
    }

    #[test]
    fn a_guarded_elif_whose_test_does_not_lower_is_not_walked() {
        assert_guarded_body_accepted(
            "if TYPE_CHECKING:\n    if True:\n        x = 1\n    elif (lambda: 1):\n        break\n",
        );
    }

    #[test]
    fn a_guarded_for_else_body_is_not_walked() {
        assert_guarded_body_accepted(
            "if TYPE_CHECKING:\n    for x in xs:\n        break\n    else:\n        pass\n",
        );
    }

    #[test]
    fn a_guarded_for_with_a_tuple_target_is_not_walked() {
        assert_guarded_body_accepted("if TYPE_CHECKING:\n    for a, b in xs:\n        break\n");
    }

    #[test]
    fn a_guarded_for_over_a_literal_list_is_not_walked() {
        assert_guarded_body_accepted("if TYPE_CHECKING:\n    for x in [1, 2]:\n        break\n");
    }

    #[test]
    fn a_guarded_for_over_a_non_range_call_is_not_walked() {
        assert_guarded_body_accepted("if TYPE_CHECKING:\n    for x in items():\n        break\n");
    }

    #[test]
    fn a_guarded_for_over_a_call_with_a_non_name_callee_is_not_walked() {
        assert_guarded_body_accepted(
            "if TYPE_CHECKING:\n    for x in obj.items():\n        break\n",
        );
    }

    #[test]
    fn a_guarded_for_over_a_range_with_keywords_is_not_walked() {
        assert_guarded_body_accepted(
            "if TYPE_CHECKING:\n    for x in range(1, stop=2):\n        break\n",
        );
    }

    #[test]
    fn a_guarded_for_over_a_range_whose_arguments_do_not_lower_is_not_walked() {
        assert_guarded_body_accepted("if TYPE_CHECKING:\n    for x in range():\n        break\n");
    }

    #[test]
    fn a_guarded_except_handler_with_a_non_name_type_is_not_walked() {
        assert_guarded_body_accepted(
            "if TYPE_CHECKING:\n    try:\n        pass\n    except errors.Boom:\n        break\n",
        );
    }

    #[test]
    fn a_guarded_except_handler_with_an_empty_tuple_type_is_not_walked() {
        assert_guarded_body_accepted(
            "if TYPE_CHECKING:\n    try:\n        pass\n    except ():\n        break\n",
        );
    }

    #[test]
    fn a_guarded_except_handler_with_a_non_name_tuple_element_is_not_walked() {
        assert_guarded_body_accepted(
            "if TYPE_CHECKING:\n    try:\n        pass\n    except (ValueError, errors.Boom):\n        break\n",
        );
    }

    #[test]
    fn a_guarded_try_after_a_handler_type_that_does_not_lower_is_not_walked() {
        // `lower_stmt` collects the handlers before `orelse`/`finalbody`, so
        // a rejected handler type aborts the whole `try` -- the walker stops
        // at the same point rather than reporting from a clause real
        // lowering would never reach.
        assert_guarded_body_accepted(
            "if TYPE_CHECKING:\n    try:\n        pass\n    except errors.Boom:\n        pass\n    else:\n        break\n",
        );
    }

    #[test]
    fn a_guarded_nested_function_body_is_not_walked() {
        assert_guarded_body_accepted("if TYPE_CHECKING:\n    def g() -> None:\n        break\n");
    }

    #[test]
    fn a_guarded_match_case_body_is_not_walked() {
        assert_guarded_body_accepted(
            "if TYPE_CHECKING:\n    match x:\n        case 1:\n            break\n",
        );
    }

    #[test]
    fn a_guarded_if_elif_else_chain_with_no_violation_is_accepted() {
        assert_guarded_body_accepted(
            "if TYPE_CHECKING:\n    if True:\n        x = 1\n    elif False:\n        y = 2\n    else:\n        z = 3\n",
        );
    }

    #[test]
    fn a_guarded_yield_in_a_compound_expression_stays_silent() {
        // Residual gap: `x = (yield 3)` is a `Stmt::Assign`, which this
        // statement-level walker does not visit.
        assert_guarded_body_accepted("if TYPE_CHECKING:\n    x = (yield 3)\n");
    }
}
