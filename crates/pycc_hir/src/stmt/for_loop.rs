//! The `Stmt::For` arm of `lower_stmt`, extracted from `stmt.rs` per
//! AGENTS.md's file-decomposition rule (issue #890), following
//! `stmt/exception.rs`'s precedent. The body is the original arm's,
//! unchanged.

use super::{ExceptStarCtx, lower_body, lower_range_call};
use crate::class::ClassAnnotationInfo;
use crate::expr::keyword_bind::SignatureTable;
use crate::{HirStmt, ImportBinding, Ty, context_invalid, unsupported};
use pycc_ast::{Expr, StmtFor};
use pycc_diag::Diagnostic;

/// Lowers a `for` loop: `for x in <name>:` to `HirStmt::ForList`, `for x
/// in range(...)` to `HirStmt::ForRange`, and the two foreign-`object`
/// producer shapes `for x in o.attr:` / `for x in o.method(...):` to
/// `HirStmt::ForObject` (PR 3c of #1082); every other iterable shape is a
/// `C0001`. `async for` is context-invalid (D-148) because no `async def`
/// body is ever lowered today.
#[allow(clippy::too_many_arguments)]
pub(super) fn lower_for(
    for_stmt: &StmtFor,
    aliases: &[(String, Ty)],
    in_function: bool,
    except_star: ExceptStarCtx,
    class_name: Option<&str>,
    type_param: Option<&str>,
    class_defs: &[ClassAnnotationInfo],
    imports: &[ImportBinding],
    signatures: &SignatureTable,
) -> Result<HirStmt, Diagnostic> {
    if for_stmt.is_async {
        // `async for` is only valid Python syntax inside an `async
        // def` body, but `lower_function` unconditionally rejects
        // any `async def` (D-141's own `def.is_async` check, earlier
        // in this file) before its body is ever lowered -- so this
        // arm can only ever be reached from a synchronous function
        // body or from module scope, never from inside a real async
        // function. There is therefore no reachable "valid Python,
        // just not implemented yet" case here today: every
        // occurrence is context-invalid, exactly like a top-level
        // `break`/`continue` (see the `Stmt::Break`/`Stmt::Continue`
        // arms above). Revisit this once/if async function support
        // lands -- it would reopen a genuine valid-but-unimplemented
        // case this arm cannot distinguish from today (D-148).
        return Err(context_invalid(
            "'async for' outside async function",
            for_stmt.range,
        ));
    }
    if !for_stmt.orelse.is_empty() {
        return Err(unsupported("for/else is not supported yet", for_stmt.range));
    }
    let Expr::Name(var) = for_stmt.target.as_ref() else {
        return Err(unsupported(
            format!(
                "only a bare name for-target is supported so far, got {}",
                pycc_ast::expr_kind_name(&for_stmt.target)
            ),
            pycc_ast::expr_range(&for_stmt.target),
        ));
    };
    // A bare-name iterable is `for v in some_list:` (D-105) or
    // `for k in some_dict:` (PR-11 Task 3, D-123) -- resolved to
    // `Ty::List`, `Ty::Dict`, or rejected by pycc_types, not here;
    // HIR only records the syntactic shape.
    if let Expr::Name(list_name) = for_stmt.iter.as_ref() {
        return Ok(HirStmt::ForList {
            var: var.id.to_string(),
            list: list_name.id.as_str().to_string(),
            body: lower_body(
                &for_stmt.body,
                aliases,
                true,
                in_function,
                // See the `Stmt::While` arm above -- the same
                // CPython-verified shielding rule applies to a
                // `for` loop's body.
                false,
                // #795 (PEP 654): and the same `except*` demotion.
                except_star.shielded_by_loop(),
                class_name,
                type_param,
                class_defs,
                imports,
                signatures,
            )?,
        });
    }
    // `for x in o.attr:` -- one of the two producer shapes admitted as a
    // possible foreign `object` iterable (PR 3c of #1082). HIR has no
    // types, so this admits *every* attribute iterable; `pycc_types`
    // refuses a non-`Ty::Object` one with `I0404`. Written as an `if let`
    // ahead of the `Expr::Call` binding below so that every other
    // iterable shape keeps reaching its existing `C0001` verbatim.
    if let Expr::Attribute(_) = for_stmt.iter.as_ref() {
        return lower_for_object(
            for_stmt,
            var.id.as_str(),
            aliases,
            in_function,
            except_star,
            class_name,
            type_param,
            class_defs,
            imports,
            signatures,
        );
    }
    let Expr::Call(call) = for_stmt.iter.as_ref() else {
        return Err(unsupported(
            format!(
                "only `for x in range(...)` or `for x in <list>` is supported so far, got {} as the iterable",
                pycc_ast::expr_kind_name(&for_stmt.iter)
            ),
            pycc_ast::expr_range(&for_stmt.iter),
        ));
    };
    // `for x in o.method(...):` -- the second admitted producer shape.
    if let Expr::Attribute(_) = call.func.as_ref() {
        return lower_for_object(
            for_stmt,
            var.id.as_str(),
            aliases,
            in_function,
            except_star,
            class_name,
            type_param,
            class_defs,
            imports,
            signatures,
        );
    }
    let Expr::Name(callee) = call.func.as_ref() else {
        return Err(unsupported(
            format!(
                "only `for x in range(...)` is supported so far, got a call whose callee is {}",
                pycc_ast::expr_kind_name(&call.func)
            ),
            pycc_ast::expr_range(&call.func),
        ));
    };
    if callee.id.as_str() != "range" {
        return Err(unsupported(
            format!(
                "only iterating over `range(...)` is supported so far, got `{}`",
                callee.id
            ),
            call.range,
        ));
    }
    if !call.arguments.keywords.is_empty() {
        return Err(unsupported(
            "keyword arguments to range() are not supported yet",
            call.range,
        ));
    }
    // Lockstep with `type_checking::for_iterable_lowers` (Part 1 of #884,
    // #1125): that function re-lowers this same `range(...)` call to decide
    // whether a `TYPE_CHECKING`-guarded `for` would lower. Both must pass the
    // same `signatures` table, or the two answers diverge for a `range()`
    // argument that is itself a keyword call.
    let (start, stop, step) = lower_range_call(call, in_function, class_name, imports, signatures)?;
    Ok(HirStmt::ForRange {
        var: var.id.to_string(),
        start,
        stop,
        step,
        body: lower_body(
            &for_stmt.body,
            aliases,
            true,
            in_function,
            // See the `Stmt::While` arm above -- the same
            // CPython-verified shielding rule applies here too.
            false,
            // #795 (PEP 654): and the same `except*` demotion.
            except_star.shielded_by_loop(),
            class_name,
            type_param,
            class_defs,
            imports,
            signatures,
        )?,
    })
}

/// Lowers one of the two admitted foreign-`object` iterable shapes
/// (`for x in o.attr:` and `for x in o.method(...):`) to
/// `HirStmt::ForObject`. Shared by both `lower_for` call sites so the two
/// shapes cannot drift apart. The body lowering is `ForList`'s verbatim,
/// including the loop flag, the CPython-verified shielding argument and
/// the #795 `except*` demotion.
#[allow(clippy::too_many_arguments)]
fn lower_for_object(
    for_stmt: &StmtFor,
    var: &str,
    aliases: &[(String, Ty)],
    in_function: bool,
    except_star: ExceptStarCtx,
    class_name: Option<&str>,
    type_param: Option<&str>,
    class_defs: &[ClassAnnotationInfo],
    imports: &[ImportBinding],
    signatures: &SignatureTable,
) -> Result<HirStmt, Diagnostic> {
    Ok(HirStmt::ForObject {
        var: var.to_string(),
        iter: Box::new(crate::expr::lower_expr(
            for_stmt.iter.as_ref(),
            in_function,
            class_name,
            imports,
            signatures,
        )?),
        body: lower_body(
            &for_stmt.body,
            aliases,
            true,
            in_function,
            false,
            except_star.shielded_by_loop(),
            class_name,
            type_param,
            class_defs,
            imports,
            signatures,
        )?,
    })
}
