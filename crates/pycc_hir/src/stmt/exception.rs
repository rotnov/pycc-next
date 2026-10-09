//! Lowering for `except` handler clauses.
//!
//! piece of threaded context, and the first that is not a `bool`. It records
//! where the statement being lowered sits relative to the nearest enclosing
//! `except*` clause body, and it governs `Stmt::Return`, `Stmt::Break`, and
//! `Stmt::Continue` -- but, unlike `in_finally`, *not* uniformly:
//!
//! - `Stmt::Return` is rejected in **both** inside states, so an intervening
//!   loop does not rescue it.
//! - `Stmt::Break`/`Stmt::Continue` are rejected only in
//!   `InsideUnshielded`; a loop entered *within* the clause body demotes the
//!   context to `InsideLoopShielded` and they lower normally from there.
//!
//! That asymmetry is CPython's, not a design choice, and it is the reason
//! this piece of context needs three states where `in_finally` needs two.
//! Verified directly against CPython 3.14.6 by `compile()`-ing each shape:
//! `for i in range(3): break` inside an `except*` clause compiles, while the
//! same loop containing `return 1` still raises `SyntaxError: 'break',
//! 'continue' and 'return' cannot appear in an except* block`. (Note that a
//! *naive* reading predicts the opposite grouping from PEP 765's, where the
//! loop shields all three -- the two rules genuinely differ, so neither flag
//! can be derived from the other.)
//!
//! Two further differences from `in_finally`:
//!
//! - It is **propagated** into a `finally`, never cleared: CPython rejects a
//!   `return` in a `finally` that is itself nested inside an `except*`
//!   clause body. A try-star's own `finalbody` is not inside its own
//!   handlers, so the incoming value is already `Outside` there and
//!   `return`-in-a-try-star's-`finally` stays accepted (as PEP 765's own
//!   `L0001` rejection, not this one).
//! - When both restrictions apply, the `except*` message wins. That matches
//!   CPython's own precedence: the `except*` failure is a fatal
//!   `SyntaxError` while the PEP 765 restriction is only a
//!   `SyntaxWarning`, so the `except*` guards run first in all three arms.
//!
//! Like `in_loop`/`in_finally`, entering a function body resets it -- to the
//! constant `ExceptStarCtx::Outside`, never a conditional on the enclosing
//! context, since a `return` inside a `def` nested in an `except*` body is
//! accepted by CPython and a conditional reset would be an unreachable
//! branch under this repository's 100%-region coverage gate.
//!
//! `Stmt::Return`'s guard carries an `in_function` conjunct for the same
//! reason the PEP 765 one does: at module scope CPython's fatal error is the
//! pre-existing `SyntaxError: 'return' outside function`, so pycc defers to
//! its own `T0024`. `Stmt::Break`/`Stmt::Continue` carry no matching
//! `in_loop` conjunct, because CPython reports the `except*` error even when
//! no enclosing loop exists at all.

use super::lower_body;
use crate::class::ClassAnnotationInfo;
use crate::expr::keyword_bind::SignatureTable;
use crate::{HirExceptHandler, ImportBinding, Ty, unsupported};
use pycc_ast::{ExceptHandler, Expr};
use pycc_diag::Diagnostic;

/// #795 (PEP 654): where the statement being lowered sits relative to the
/// nearest enclosing `except*` clause body. This is this module's fourth
/// piece of threaded context, and unlike the three `bool`s beside it, it has
/// three states rather than two -- an intervening loop *demotes* the context
/// instead of clearing it, because CPython shields a `break`/`continue` with
/// such a loop but does not shield a `return`.
///
/// Verified directly against CPython 3.14.6's own compiler (see the module
/// doc comment above for the full table): `for i in range(3): break` inside
/// an `except*` clause body compiles, while the same loop containing
/// `return 1` still raises `SyntaxError: 'break', 'continue' and 'return'
/// cannot appear in an except* block`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum ExceptStarCtx {
    /// Not inside any `except*` clause body -- the state every function body
    /// and every module top level starts in.
    Outside,
    /// Inside an `except*` clause body with no intervening loop: `return`,
    /// `break`, and `continue` are all rejected.
    InsideUnshielded,
    /// Inside an `except*` clause body, but behind at least one loop entered
    /// within it: `break`/`continue` are shielded and lower normally, while
    /// `return` is still rejected.
    InsideLoopShielded,
}

impl ExceptStarCtx {
    /// The value to thread into the body of a `while`/`for` loop entered
    /// from this context. `Outside` stays `Outside` (a loop does not put
    /// anything inside an `except*`); either inside state becomes
    /// `InsideLoopShielded`, which is idempotent for a second nested loop.
    pub(crate) fn shielded_by_loop(self) -> Self {
        match self {
            Self::Outside => Self::Outside,
            Self::InsideUnshielded | Self::InsideLoopShielded => Self::InsideLoopShielded,
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) fn lower_except_handler(
    handler: &ExceptHandler,
    aliases: &[(String, Ty)],
    in_loop: bool,
    in_function: bool,
    in_finally: bool,
    // #795 (PEP 654): already derived by the caller, because `is_star` is a
    // property of the enclosing `StmtTry` rather than of this handler --
    // `InsideUnshielded` for an `except*` clause, the incoming context
    // unchanged for a plain `except` clause.
    except_star: ExceptStarCtx,
    class_name: Option<&str>,
    type_param: Option<&str>,
    class_defs: &[ClassAnnotationInfo],
    imports: &[ImportBinding],
    signatures: &SignatureTable,
) -> Result<HirExceptHandler, Diagnostic> {
    let pycc_ast::ExceptHandler::ExceptHandler(handler) = handler;
    let exc_type = handler
        .type_
        .as_deref()
        .map(|exception_type| match exception_type {
            Expr::Name(name) => Ok(vec![name.id.as_str().to_string()]),
            // PEP 758: `except A, B:` (bare comma) and `except (A, B):`
            // (parenthesized) both lower to `Expr::Tuple` -- only the
            // `parenthesized` flag differs, which HIR discards. Every
            // element must be a bare name, and the list must be non-empty
            // (`except ():` is otherwise syntactically valid and would
            // reach MIR/codegen's non-empty-tag-set invariant unchecked).
            Expr::Tuple(tuple) => {
                if tuple.elts.is_empty() {
                    return Err(unsupported(
                        "an `except` handler must name at least one exception type",
                        pycc_ast::expr_range(exception_type),
                    ));
                }
                tuple
                    .elts
                    .iter()
                    .map(|elt| match elt {
                        Expr::Name(name) => Ok(name.id.as_str().to_string()),
                        _ => Err(unsupported(
                            "each element of a multi-type except handler must be a bare name",
                            pycc_ast::expr_range(elt),
                        )),
                    })
                    .collect()
            }
            _ => Err(unsupported(
                "only a bare-name exception type is supported so far in except handlers",
                pycc_ast::expr_range(exception_type),
            )),
        })
        .transpose()?;
    let name = handler
        .name
        .as_ref()
        .map(|name| name.id.as_str().to_string());
    let body = lower_body(
        &handler.body,
        aliases,
        in_loop,
        in_function,
        in_finally,
        except_star,
        class_name,
        type_param,
        class_defs,
        imports,
        signatures,
    )?;
    Ok(HirExceptHandler {
        exc_type,
        name,
        body,
    })
}

#[cfg(test)]
mod tests {
    use crate::{HirExpr, HirStmt};

    // -- Exception handling HIR tests (#382, PR-22 Part 1) --

    /// Test helper: extract a `Try` statement from a top-level HIR item,
    /// panicking if the item is not a `Try`.  The panic arm is covered by
    /// `expect_top_level_try_panics_on_non_try`.
    fn expect_top_level_try(
        item: &crate::HirItem,
    ) -> (
        &[HirStmt],
        &[crate::HirExceptHandler],
        &[HirStmt],
        &[HirStmt],
    ) {
        match item {
            crate::HirItem::TopLevelStmt(HirStmt::Try {
                body,
                handlers,
                orelse,
                finalbody,
            }) => (body, handlers, orelse, finalbody),
            _ => panic!("expected Try"),
        }
    }

    /// Test helper: extract a `Raise` statement from a top-level HIR item,
    /// panicking if the item is not a `Raise`.  The panic arm is covered by
    /// `expect_top_level_raise_panics_on_non_raise`.
    fn expect_top_level_raise(item: &crate::HirItem) -> (&Option<HirExpr>, &Option<HirExpr>) {
        match item {
            crate::HirItem::TopLevelStmt(HirStmt::Raise { exc, cause }) => (exc, cause),
            _ => panic!("expected Raise"),
        }
    }

    /// Test helper: extract a `Raise` statement from a `HirStmt`,
    /// panicking if the statement is not a `Raise`.  The panic arm is
    /// covered by `expect_raise_stmt_panics_on_non_raise`.
    fn expect_raise_stmt(stmt: &HirStmt) -> (&Option<HirExpr>, &Option<HirExpr>) {
        match stmt {
            HirStmt::Raise { exc, cause } => (exc, cause),
            _ => panic!("expected Raise"),
        }
    }

    #[test]
    #[should_panic(expected = "expected Try")]
    fn expect_top_level_try_panics_on_non_try() {
        expect_top_level_try(&crate::HirItem::TopLevelStmt(HirStmt::ExprStmt(
            HirExpr::IntLiteral(0),
        )));
    }

    #[test]
    #[should_panic(expected = "expected Raise")]
    fn expect_top_level_raise_panics_on_non_raise() {
        expect_top_level_raise(&crate::HirItem::TopLevelStmt(HirStmt::ExprStmt(
            HirExpr::IntLiteral(0),
        )));
    }

    #[test]
    #[should_panic(expected = "expected Raise")]
    fn expect_raise_stmt_panics_on_non_raise() {
        expect_raise_stmt(&HirStmt::ExprStmt(HirExpr::IntLiteral(0)));
    }

    #[test]
    fn lower_try_except_lowers_successfully() {
        let module = pycc_parser::parse("try:\n    x = 1\nexcept ValueError:\n    y = 2\n")
            .expect("test fixture must parse");
        let hir = crate::lower_checked(&module).expect("lowering must succeed");
        let items = &hir.items;
        assert_eq!(items.len(), 1);
        let (body, handlers, orelse, finalbody) = expect_top_level_try(&items[0]);
        assert_eq!(body.len(), 1);
        assert_eq!(handlers.len(), 1);
        assert_eq!(handlers[0].exc_type, Some(vec!["ValueError".to_string()]));
        assert!(handlers[0].name.is_none());
        assert_eq!(handlers[0].body.len(), 1);
        assert!(orelse.is_empty());
        assert!(finalbody.is_empty());
    }

    #[test]
    fn lower_try_except_as_binding_lowers_successfully() {
        let module = pycc_parser::parse("try:\n    x = 1\nexcept ValueError as e:\n    y = 2\n")
            .expect("test fixture must parse");
        let hir = crate::lower_checked(&module).expect("lowering must succeed");
        let (_, handlers, _, _) = expect_top_level_try(&hir.items[0]);
        assert_eq!(handlers[0].exc_type, Some(vec!["ValueError".to_string()]));
        assert_eq!(handlers[0].name.as_deref(), Some("e"));
    }

    #[test]
    fn lower_try_bare_except_lowers_successfully() {
        let module = pycc_parser::parse("try:\n    x = 1\nexcept:\n    y = 2\n")
            .expect("test fixture must parse");
        let hir = crate::lower_checked(&module).expect("lowering must succeed");
        let (_, handlers, _, _) = expect_top_level_try(&hir.items[0]);
        assert!(handlers[0].exc_type.is_none());
        assert!(handlers[0].name.is_none());
    }

    #[test]
    fn lower_try_multiple_handlers_lowers_successfully() {
        let module = pycc_parser::parse(
            "try:\n    x = 1\nexcept ValueError:\n    y = 2\nexcept KeyError:\n    z = 3\n",
        )
        .expect("test fixture must parse");
        let hir = crate::lower_checked(&module).expect("lowering must succeed");
        let (_, handlers, _, _) = expect_top_level_try(&hir.items[0]);
        assert_eq!(handlers.len(), 2);
        assert_eq!(handlers[0].exc_type, Some(vec!["ValueError".to_string()]));
        assert_eq!(handlers[1].exc_type, Some(vec!["KeyError".to_string()]));
    }

    #[test]
    fn lower_try_else_finally_lowers_successfully() {
        let module = pycc_parser::parse(
            "try:\n    x = 1\nexcept ValueError:\n    y = 2\nelse:\n    z = 3\nfinally:\n    w = 4\n",
        ).expect("test fixture must parse");
        let hir = crate::lower_checked(&module).expect("lowering must succeed");
        let (_, _, orelse, finalbody) = expect_top_level_try(&hir.items[0]);
        assert_eq!(orelse.len(), 1);
        assert_eq!(finalbody.len(), 1);
    }

    #[test]
    fn lower_raise_lowers_successfully() {
        let module =
            pycc_parser::parse("raise ValueError(\"bad\")\n").expect("test fixture must parse");
        let hir = crate::lower_checked(&module).expect("lowering must succeed");
        let (exc, cause) = expect_top_level_raise(&hir.items[0]);
        assert!(exc.is_some());
        assert!(cause.is_none());
    }

    #[test]
    fn lower_raise_from_lowers_successfully() {
        let module = pycc_parser::parse("raise ValueError(\"bad\") from RuntimeError(\"cause\")\n")
            .expect("test fixture must parse");
        let hir = crate::lower_checked(&module).expect("lowering must succeed");
        let (exc, cause) = expect_top_level_raise(&hir.items[0]);
        assert!(exc.is_some());
        assert!(cause.is_some());
    }

    #[test]
    fn lower_raise_from_none_drops_the_cause() {
        // PEP 409: `from None` suppresses implicit context chaining, whose
        // only observable effect is traceback rendering. pycc emits no
        // traceback, so the suppression marker lowers to "no cause" rather
        // than to a `None`-valued cause expression.
        let module = pycc_parser::parse("raise ValueError(\"bad\") from None\n")
            .expect("test fixture must parse");
        let hir = crate::lower_checked(&module).expect("lowering must succeed");
        let (exc, cause) = expect_top_level_raise(&hir.items[0]);
        assert!(exc.is_some());
        assert!(cause.is_none());
    }

    #[test]
    fn lower_bare_raise_lowers_successfully() {
        let module = pycc_parser::parse("try:\n    x = 1\nexcept ValueError:\n    raise\n")
            .expect("test fixture must parse");
        let hir = crate::lower_checked(&module).expect("lowering must succeed");
        let (_, handlers, _, _) = expect_top_level_try(&hir.items[0]);
        let (exc, cause) = expect_raise_stmt(&handlers[0].body[0]);
        assert!(exc.is_none());
        assert!(cause.is_none());
    }

    /// Part 3 of #382 (#542, PEP 654): a helper mirroring
    /// `expect_top_level_try` for the `TryStar` variant.
    fn expect_top_level_try_star(
        item: &crate::HirItem,
    ) -> (
        &[HirStmt],
        &[crate::HirExceptHandler],
        &[HirStmt],
        &[HirStmt],
    ) {
        match item {
            crate::HirItem::TopLevelStmt(HirStmt::TryStar {
                body,
                handlers,
                orelse,
                finalbody,
            }) => (body, handlers, orelse, finalbody),
            _ => panic!("expected TryStar"),
        }
    }

    #[test]
    #[should_panic(expected = "expected TryStar")]
    fn expect_top_level_try_star_panics_on_a_non_try_star_item() {
        // Covers the helper's own failure branch: it is never hit by any
        // lowering-success test above, since those all lower fixtures that
        // do produce a `TryStar` item, so exercise it directly here.
        let module = pycc_parser::parse("try:\n    x = 1\nexcept ValueError:\n    y = 2\n")
            .expect("test fixture must parse");
        let hir = crate::lower_checked(&module).expect("plain except must lower");
        expect_top_level_try_star(&hir.items[0]);
    }

    #[test]
    fn lower_try_star_lowers_to_try_star_with_named_type() {
        let module = pycc_parser::parse("try:\n    x = 1\nexcept* ValueError:\n    y = 2\n")
            .expect("test fixture must parse");
        let hir = crate::lower_checked(&module).expect("except* with a named type must lower");
        let (_, handlers, _, _) = expect_top_level_try_star(&hir.items[0]);
        assert_eq!(handlers[0].exc_type, Some(vec!["ValueError".to_string()]));
    }

    #[test]
    fn lower_try_star_with_as_binding_lowers_successfully() {
        let module = pycc_parser::parse("try:\n    x = 1\nexcept* ValueError as e:\n    y = 2\n")
            .expect("test fixture must parse");
        let hir = crate::lower_checked(&module).expect("except* as e must lower");
        let (_, handlers, _, _) = expect_top_level_try_star(&hir.items[0]);
        assert_eq!(handlers[0].name, Some("e".to_string()));
    }

    #[test]
    fn lower_try_star_bare_except_star_is_rejected_at_parse_time() {
        // CPython itself rejects a typeless `except*:` as a `SyntaxError`
        // (PEP 654 requires every `except*` clause to name a type), and
        // ruff's parser enforces the same grammar rule -- this never
        // reaches HIR lowering, so pycc reports the same class of error
        // (a parse failure) that CPython would, rather than a separate
        // pycc-specific diagnostic.
        let module = pycc_parser::parse("try:\n    x = 1\nexcept*:\n    y = 2\n");
        assert!(module.is_err());
    }

    #[test]
    fn lower_except_with_parenthesized_multi_type_accepts_all_names_in_order() {
        // PEP 758: `except (ValueError, TypeError):` names two exception
        // types in a single handler -- it must be accepted, and the names
        // preserved in source order.
        let module =
            pycc_parser::parse("try:\n    x = 1\nexcept (ValueError, TypeError):\n    y = 2\n")
                .expect("test fixture must parse");
        let hir = crate::lower_checked(&module).expect("lowering must succeed");
        let (_, handlers, _, _) = expect_top_level_try(&hir.items[0]);
        assert_eq!(
            handlers[0].exc_type,
            Some(vec!["ValueError".to_string(), "TypeError".to_string()])
        );
    }

    #[test]
    fn lower_except_with_bare_comma_multi_type_matches_parenthesized_form() {
        // PEP 758: `except ValueError, TypeError:` (no parentheses) lowers
        // to the same `exc_type` as the parenthesized form -- only the
        // (HIR-discarded) `parenthesized` flag differs at the AST level.
        let module =
            pycc_parser::parse("try:\n    x = 1\nexcept ValueError, TypeError:\n    y = 2\n")
                .expect("test fixture must parse");
        let hir = crate::lower_checked(&module).expect("lowering must succeed");
        let (_, handlers, _, _) = expect_top_level_try(&hir.items[0]);
        assert_eq!(
            handlers[0].exc_type,
            Some(vec!["ValueError".to_string(), "TypeError".to_string()])
        );
    }

    #[test]
    fn lower_except_with_mixed_bare_and_parenthesized_handlers() {
        // A bare-comma handler and a parenthesized handler in the same
        // `try` must not branch on the discarded `parenthesized` flag.
        let module = pycc_parser::parse(
            "try:\n    x = 1\nexcept ValueError, TypeError:\n    y = 2\nexcept (KeyError, IndexError):\n    z = 3\n",
        )
        .expect("test fixture must parse");
        let hir = crate::lower_checked(&module).expect("lowering must succeed");
        let (_, handlers, _, _) = expect_top_level_try(&hir.items[0]);
        assert_eq!(
            handlers[0].exc_type,
            Some(vec!["ValueError".to_string(), "TypeError".to_string()])
        );
        assert_eq!(
            handlers[1].exc_type,
            Some(vec!["KeyError".to_string(), "IndexError".to_string()])
        );
    }

    #[test]
    fn lower_except_with_three_or_more_types_accepts_all() {
        let module = pycc_parser::parse(
            "try:\n    x = 1\nexcept (ValueError, TypeError, KeyError):\n    y = 2\n",
        )
        .expect("test fixture must parse");
        let hir = crate::lower_checked(&module).expect("lowering must succeed");
        let (_, handlers, _, _) = expect_top_level_try(&hir.items[0]);
        assert_eq!(
            handlers[0].exc_type,
            Some(vec![
                "ValueError".to_string(),
                "TypeError".to_string(),
                "KeyError".to_string()
            ])
        );
    }

    #[test]
    fn lower_except_single_element_tuple_matches_bare_name_form_all_spellings() {
        // `except (A,):` and `except A,:` (single-element tuple, both
        // spellings) must behave exactly like `except A:`.
        for source in [
            "try:\n    x = 1\nexcept ValueError:\n    y = 2\n",
            "try:\n    x = 1\nexcept (ValueError,):\n    y = 2\n",
            "try:\n    x = 1\nexcept ValueError,:\n    y = 2\n",
        ] {
            let module = pycc_parser::parse(source).expect("test fixture must parse");
            let hir = crate::lower_checked(&module).expect("lowering must succeed");
            let (_, handlers, _, _) = expect_top_level_try(&hir.items[0]);
            assert_eq!(
                handlers[0].exc_type,
                Some(vec!["ValueError".to_string()]),
                "source {source:?} must lower to a single-element type list"
            );
        }
    }

    #[test]
    fn lower_except_with_empty_tuple_type_rejects_with_c0001() {
        // `except ():` parses successfully as an empty tuple type, but
        // must be rejected -- otherwise MIR/codegen's non-empty
        // handler-tag-set invariant would be violated.
        let module = pycc_parser::parse("try:\n    x = 1\nexcept ():\n    y = 2\n")
            .expect("test fixture must parse");
        let err = crate::lower_checked(&module).unwrap_err();
        assert_eq!(err.code, "C0001");
        assert!(err.message.contains("at least one exception type"));
    }

    #[test]
    fn lower_except_with_non_name_tuple_element_rejects_with_c0001() {
        for source in [
            "try:\n    x = 1\nexcept (ValueError, \"not a name\"):\n    y = 2\n",
            "try:\n    x = 1\nexcept (ValueError, some_call()):\n    y = 2\n",
            "try:\n    x = 1\nexcept (ValueError, *rest):\n    y = 2\n",
        ] {
            let module = pycc_parser::parse(source).expect("test fixture must parse");
            let err = crate::lower_checked(&module).unwrap_err();
            assert_eq!(err.code, "C0001", "source {source:?} must be rejected");
            assert!(
                err.message.contains("bare name"),
                "source {source:?}: unexpected message {:?}",
                err.message
            );
        }
    }

    #[test]
    fn lower_except_with_non_name_non_tuple_type_rejects_with_c0001() {
        for source in [
            "try:\n    x = 1\nexcept some.Attribute:\n    y = 2\n",
            "try:\n    x = 1\nexcept some_call():\n    y = 2\n",
        ] {
            let module = pycc_parser::parse(source).expect("test fixture must parse");
            let err = crate::lower_checked(&module).unwrap_err();
            assert_eq!(err.code, "C0001", "source {source:?} must be rejected");
            assert!(
                err.message
                    .contains("only a bare-name exception type is supported"),
                "source {source:?}: unexpected message {:?}",
                err.message
            );
        }
    }

    #[test]
    fn lower_try_with_unsupported_body_expr_rejects() {
        // An unsupported expression in the try body should propagate
        // the lowering error through the `?` operator.
        let module =
            pycc_parser::parse("try:\n    x = lambda y: y\nexcept ValueError:\n    pass\n")
                .expect("test fixture must parse");
        let result = crate::lower_checked(&module);
        assert!(
            result.is_err(),
            "lowering should fail for unsupported expression in try body"
        );
    }

    #[test]
    fn lower_try_with_unsupported_handler_body_rejects() {
        // An unsupported expression in the handler body should propagate
        // the lowering error through the `?` operator.
        let module =
            pycc_parser::parse("try:\n    x = 1\nexcept ValueError:\n    y = lambda z: z\n")
                .expect("test fixture must parse");
        let result = crate::lower_checked(&module);
        assert!(
            result.is_err(),
            "lowering should fail for unsupported expression in handler body"
        );
    }

    #[test]
    fn lower_try_with_unsupported_else_body_rejects() {
        // An unsupported expression in the else body should propagate
        // the lowering error through the `?` operator.
        let module = pycc_parser::parse(
            "try:\n    x = 1\nexcept ValueError:\n    pass\nelse:\n    y = lambda z: z\n",
        )
        .expect("test fixture must parse");
        let result = crate::lower_checked(&module);
        assert!(
            result.is_err(),
            "lowering should fail for unsupported expression in else body"
        );
    }

    #[test]
    fn lower_try_with_unsupported_finally_body_rejects() {
        // An unsupported expression in the finally body should propagate
        // the lowering error through the `?` operator.
        let module = pycc_parser::parse(
            "try:\n    x = 1\nexcept ValueError:\n    pass\nfinally:\n    y = lambda z: z\n",
        )
        .expect("test fixture must parse");
        let result = crate::lower_checked(&module);
        assert!(
            result.is_err(),
            "lowering should fail for unsupported expression in finally body"
        );
    }

    #[test]
    fn lower_raise_with_unsupported_exc_rejects() {
        // An unsupported expression as the raise exc should propagate
        // the lowering error through the `?` operator (line 459).
        let module = pycc_parser::parse("raise lambda: None\n").expect("test fixture must parse");
        let result = crate::lower_checked(&module);
        assert!(
            result.is_err(),
            "lowering should fail for unsupported expression in raise exc"
        );
    }

    #[test]
    fn lower_raise_from_with_unsupported_cause_rejects() {
        // An unsupported expression as the raise cause should propagate
        // the lowering error through the `?` operator (line 464).
        let module = pycc_parser::parse("raise ValueError(\"bad\") from lambda: None\n")
            .expect("test fixture must parse");
        let result = crate::lower_checked(&module);
        assert!(
            result.is_err(),
            "lowering should fail for unsupported expression in raise cause"
        );
    }
}

/// #795 (PEP 654): the `ExceptStarCtx` threading added to this module's
/// sibling `stmt.rs`, exercised through this crate's *own* instrumented
/// build. `tests/diagnostics/l0001_*_except_star.py` pins the rendered
/// diagnostics through the public CLI, but `cargo llvm-cov` scores regions
/// per instantiation grouped by definition location, so a branch reached
/// only from an integration test still reports as missed for this crate's
/// `--cfg test` instance -- these tests close that gap (the same rationale
/// `crates/pycc_types/src/exception/except_star_tests.rs` records at
/// length).
#[cfg(test)]
mod except_star_context_tests {
    fn lower(source: &str) -> Result<crate::HirModule, pycc_diag::Diagnostic> {
        let module = pycc_parser::parse(source).expect("test fixture must parse");
        crate::lower_checked(&module)
    }

    fn expect_error(source: &str) -> pycc_diag::Diagnostic {
        lower(source).expect_err("source must be rejected during lowering")
    }

    #[test]
    fn a_return_directly_in_an_except_star_clause_is_rejected() {
        let diagnostic = expect_error(
            "def f() -> int:\n    try:\n        pass\n    except* ValueError:\n        return 1\n    return 0\n",
        );
        assert_eq!(diagnostic.code, "L0001");
        assert_eq!(diagnostic.message, "'return' in an 'except*' block");
    }

    #[test]
    fn a_return_behind_a_for_loop_in_an_except_star_clause_is_still_rejected() {
        // `InsideLoopShielded`: a loop shields `break`/`continue` but never
        // `return` -- verified against CPython 3.14.6.
        let diagnostic = expect_error(
            "def f() -> int:\n    try:\n        pass\n    except* ValueError:\n        for i in range(3):\n            return 1\n    return 0\n",
        );
        assert_eq!(diagnostic.message, "'return' in an 'except*' block");
    }

    #[test]
    fn a_return_behind_a_while_loop_in_an_except_star_clause_is_still_rejected() {
        // The `Stmt::While` arm's own demotion site, distinct from the two
        // in `stmt/for_loop.rs` above.
        let diagnostic = expect_error(
            "def f() -> int:\n    try:\n        pass\n    except* ValueError:\n        while True:\n            return 1\n    return 0\n",
        );
        assert_eq!(diagnostic.message, "'return' in an 'except*' block");
    }

    #[test]
    fn a_return_behind_a_for_range_loop_in_an_except_star_clause_is_still_rejected() {
        // `lower_for`'s `ForRange` demotion site (the `ForList` site is
        // covered by `a_break_behind_a_for_list_loop_...` below).
        let diagnostic = expect_error(
            "def f() -> int:\n    try:\n        pass\n    except* ValueError:\n        for i in range(0, 3, 1):\n            return 1\n    return 0\n",
        );
        assert_eq!(diagnostic.message, "'return' in an 'except*' block");
    }

    #[test]
    fn a_break_directly_in_an_except_star_clause_is_rejected() {
        let diagnostic = expect_error(
            "def f() -> None:\n    try:\n        pass\n    except* ValueError:\n        break\n",
        );
        assert_eq!(diagnostic.code, "L0001");
        assert_eq!(diagnostic.message, "'break' in an 'except*' block");
    }

    #[test]
    fn a_continue_directly_in_an_except_star_clause_is_rejected() {
        let diagnostic = expect_error(
            "def f() -> None:\n    try:\n        pass\n    except* ValueError:\n        continue\n",
        );
        assert_eq!(diagnostic.code, "L0001");
        assert_eq!(diagnostic.message, "'continue' in an 'except*' block");
    }

    #[test]
    fn a_break_behind_a_for_range_loop_in_an_except_star_clause_is_shielded() {
        // The demotion's whole point: CPython *accepts* this program, so
        // pycc must fall through to its pre-existing "not implemented yet"
        // `C0001` for loop control flow rather than reporting `L0001`.
        let diagnostic = expect_error(
            "def f() -> None:\n    try:\n        pass\n    except* ValueError:\n        for i in range(3):\n            break\n",
        );
        assert_eq!(diagnostic.code, "C0001");
        assert!(
            diagnostic.message.contains("`break` inside a loop"),
            "unexpected message: {}",
            diagnostic.message
        );
    }

    #[test]
    fn a_break_behind_a_for_list_loop_in_an_except_star_clause_is_shielded() {
        let diagnostic = expect_error(
            "def f(xs: list[int]) -> None:\n    try:\n        pass\n    except* ValueError:\n        for x in xs:\n            break\n",
        );
        assert_eq!(diagnostic.code, "C0001");
    }

    #[test]
    fn a_continue_behind_a_loop_in_an_except_star_clause_is_shielded() {
        let diagnostic = expect_error(
            "def f() -> None:\n    try:\n        pass\n    except* ValueError:\n        for i in range(3):\n            continue\n",
        );
        assert_eq!(diagnostic.code, "C0001");
        assert!(
            diagnostic.message.contains("`continue` inside a loop"),
            "unexpected message: {}",
            diagnostic.message
        );
    }

    #[test]
    fn a_break_in_an_except_star_clause_inside_an_outer_loop_is_still_rejected() {
        // Only a loop entered *within* the clause body shields; an enclosing
        // loop outside the `try` does not.
        let diagnostic = expect_error(
            "def f() -> None:\n    while True:\n        try:\n            pass\n        except* ValueError:\n            break\n",
        );
        assert_eq!(diagnostic.message, "'break' in an 'except*' block");
    }

    #[test]
    fn a_finally_nested_in_an_except_star_clause_propagates_the_context() {
        // Unlike `in_finally`, the `except*` context is propagated into a
        // `finally`, never cleared -- and it wins over the PEP 765 message,
        // matching CPython's own precedence.
        let diagnostic = expect_error(
            "def f() -> None:\n    while True:\n        try:\n            pass\n        except* ValueError:\n            try:\n                pass\n            finally:\n                break\n",
        );
        assert_eq!(diagnostic.message, "'break' in an 'except*' block");
    }

    #[test]
    fn a_try_star_s_own_finally_may_still_return() {
        // The incoming context is `Outside` for a try-star's own
        // `finalbody`, so propagating (rather than clearing) it leaves this
        // pre-existing PEP 765 behavior untouched: the `finally` message,
        // not the `except*` one.
        let diagnostic = expect_error(
            "def f() -> int:\n    try:\n        pass\n    except* ValueError:\n        pass\n    finally:\n        return 1\n",
        );
        assert_eq!(diagnostic.message, "'return' in a 'finally' block");
    }

    #[test]
    fn a_return_in_an_except_star_clause_at_module_level_lowers_successfully() {
        // The `in_function` conjunct's false arm: with no enclosing
        // function, lowering succeeds and the pre-existing `T0024`
        // type-check reports `'return' outside a function` instead (see
        // `tests/diagnostics/d0024_return_in_except_star_at_module_level.py`).
        lower("try:\n    pass\nexcept* ValueError:\n    return 1\n")
            .expect("lowering must defer to the pre-existing T0024 check");
    }

    #[test]
    fn a_nested_function_in_an_except_star_clause_may_return() {
        // The constant `ExceptStarCtx::Outside` on function entry: CPython
        // accepts a `return` inside a `def` nested in an `except*` body.
        // Nested `def`s are not supported yet, so this asserts the *reason*
        // is the unsupported-nesting `C0001`, not `L0001`.
        let diagnostic = expect_error(
            "def f() -> None:\n    try:\n        pass\n    except* ValueError:\n        def g() -> int:\n            return 1\n",
        );
        assert_ne!(diagnostic.code, "L0001");
    }

    #[test]
    fn a_plain_except_clause_propagates_rather_than_setting_the_context() {
        // `handler_except_star`'s `else` arm: an ordinary `try`/`except`
        // nested inside an `except*` clause body does not shield its
        // contents, and an ordinary `try`/`except` outside one does not
        // manufacture a context either.
        let diagnostic = expect_error(
            "def f() -> int:\n    try:\n        pass\n    except* ValueError:\n        try:\n            pass\n        except TypeError:\n            return 1\n    return 0\n",
        );
        assert_eq!(diagnostic.message, "'return' in an 'except*' block");
        lower("def f() -> int:\n    try:\n        pass\n    except ValueError:\n        return 1\n    return 0\n")
            .expect("a plain `except` clause must still accept a `return`");
    }

    #[test]
    fn an_ordinary_loop_outside_any_except_star_stays_outside() {
        // `shielded_by_loop`'s `Outside` arm.
        lower("def f() -> None:\n    for i in range(3):\n        print(i)\n")
            .expect("an ordinary loop must lower successfully");
    }
}
