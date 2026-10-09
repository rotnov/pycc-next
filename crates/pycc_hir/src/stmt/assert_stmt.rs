//! The `assert` statement (#1369).
//!
//! `docs/RUNTIME.md`'s "The `assert` statement" section is the canonical
//! statement of the semantics. In short: `assert test, msg` is rewritten at
//! the AST level into
//!
//! ```python
//! if test:
//!     pass
//! else:
//!     raise AssertionError(msg)   # AssertionError("") when there is no msg
//! ```
//!
//! and handed back to `lower_stmt`, the same shape `aug_assign` uses. That
//! gives the statement CPython's evaluation order by construction: `test` is
//! evaluated once, with exactly the truthiness an `if` test has
//! (`lower_condition`), and `msg` is evaluated only on the failing path.
//! Every check the `Stmt::If` and `Stmt::Raise` arms run applies unchanged:
//! a walrus in `test` is admitted as it is in an `if` test, and one in `msg`
//! is refused by the walrus placement check exactly as it is in a `raise`
//! operand.
//!
//! pycc has no `-O` flag, so an `assert` is never stripped: `__debug__` is
//! always `True` for a compiled program.
//!
//! The rewrite names `AssertionError` by spelling, while CPython's
//! `LOAD_ASSERTION_ERROR` always reaches the builtin class. The two agree
//! whenever the spelling resolves to the builtin, and every program in
//! which it could resolve elsewhere is refused before code generation. Two
//! refusals are dedicated to it. A top-level `class`/`def`/assignment
//! binding of any builtin exception name withholds the builtin classes,
//! which `module::lower_module` reports, in source order among the other
//! per-item diagnostics, on the first top-level item containing an
//! `assert`. A function-local binding cannot hold a class, so
//! the call is `T0021` "bound to a non-callable value". Other binding
//! forms, such as a module-level `for` target, an `except ... as` name or
//! an import alias, are refused by diagnostics that already existed;
//! `docs/RUNTIME.md` lists the ones pinned by tests.

use pycc_ast::{
    Arguments, ElifElseClause, Expr, ExprCall, ExprContext, ExprName, ExprStringLiteral, Stmt,
    StmtAssert, StmtIf, StmtPass, StmtRaise, StringLiteral, StringLiteralFlags, StringLiteralValue,
};

/// Rewrites `assert_stmt` into the synthetic `Stmt::If` described in the
/// module docs. Every synthesized node carries the whole statement's range,
/// except the message, which keeps its own.
pub(super) fn desugar_assert(assert_stmt: &StmtAssert) -> Stmt {
    let range = assert_stmt.range;
    let message = match &assert_stmt.msg {
        Some(msg) => msg.as_ref().clone(),
        None => Expr::StringLiteral(ExprStringLiteral {
            node_index: Default::default(),
            range,
            value: StringLiteralValue::single(StringLiteral {
                range,
                node_index: Default::default(),
                value: "".into(),
                flags: StringLiteralFlags::empty(),
            }),
        }),
    };
    let raise = Stmt::Raise(StmtRaise {
        node_index: Default::default(),
        range,
        exc: Some(Box::new(Expr::Call(ExprCall {
            node_index: Default::default(),
            range,
            func: Box::new(Expr::Name(ExprName {
                node_index: Default::default(),
                range,
                id: "AssertionError".into(),
                ctx: ExprContext::Load,
            })),
            arguments: Arguments {
                range,
                node_index: Default::default(),
                args: Box::new([message]),
                keywords: Default::default(),
            },
        }))),
        cause: None,
    });
    Stmt::If(StmtIf {
        node_index: Default::default(),
        range,
        test: assert_stmt.test.clone(),
        body: vec![Stmt::Pass(StmtPass {
            node_index: Default::default(),
            range,
        })]
        .into(),
        elif_else_clauses: vec![ElifElseClause {
            range,
            node_index: Default::default(),
            test: None,
            body: vec![raise].into(),
        }],
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Parses `source` (one `assert` statement) and returns its node.
    fn assert_node(source: &str) -> StmtAssert {
        let module = pycc_parser::parse(source).expect("test fixture must parse");
        module.body[0]
            .as_assert_stmt()
            .expect("an assert statement")
            .clone()
    }

    /// Takes the rewrite apart and returns `(test, the raised call's single
    /// argument)`, asserting every other part of the shape on the way: an
    /// `if` whose body is one `pass`, one bare `else:` clause holding one
    /// `raise AssertionError(<arg>)` with no cause.
    fn rewrite_parts(source: &str) -> (Expr, Expr) {
        let rewritten = desugar_assert(&assert_node(source));
        let if_stmt = rewritten.as_if_stmt().expect("an `if` statement");
        assert!(matches!(if_stmt.body.as_slice(), [Stmt::Pass(_)]));
        assert_eq!(if_stmt.elif_else_clauses.len(), 1, "exactly one clause");
        let clause = &if_stmt.elif_else_clauses[0];
        assert!(clause.test.is_none(), "the clause must be a bare `else:`");
        assert_eq!(clause.body.len(), 1, "the `else` body is one statement");
        let raise = clause.body[0].as_raise_stmt().expect("a `raise`");
        assert!(raise.cause.is_none());
        let exc = raise.exc.as_deref().expect("`raise` of a value");
        let call = exc.as_call_expr().expect("`raise` of a call");
        let func = call.func.as_name_expr().expect("a call of a bare name");
        assert_eq!(func.id.as_str(), "AssertionError");
        assert!(call.arguments.keywords.is_empty());
        assert_eq!(call.arguments.args.len(), 1, "one positional argument");
        (*if_stmt.test.clone(), call.arguments.args[0].clone())
    }

    #[test]
    fn an_assert_with_a_message_raises_that_message_on_the_else_path() {
        let node = assert_node("assert x > 0, f\"bad {x}\"\n");
        let (test, arg) = rewrite_parts("assert x > 0, f\"bad {x}\"\n");
        assert_eq!(test, *node.test);
        assert_eq!(arg, *node.msg.expect("the fixture has a message"));
    }

    #[test]
    fn an_assert_without_a_message_raises_an_empty_message() {
        let node = assert_node("assert ok\n");
        let (test, arg) = rewrite_parts("assert ok\n");
        assert_eq!(test, *node.test);
        let literal = arg.as_string_literal_expr().expect("a string literal");
        assert_eq!(literal.value.to_str(), "");
    }
}
