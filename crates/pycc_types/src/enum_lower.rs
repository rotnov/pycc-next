//! PEP 435 enum lowering: enum-member attribute typing and enum-loop
//! unrolling (#379 / PR-19).
//!
//! Extracted verbatim from `crates/pycc_types/src/lib.rs` per AGENTS.md's
//! file-decomposition rule and D-185's per-file tracking issue (#544): this
//! is one cohesion-driven seam of that file, not a rewrite. Every diagnostic
//! message, every check, and every comment is unchanged -- the only edits
//! are the ones the module boundary forces (visibility keywords and `use`
//! lines). [`monomorphize`](crate::monomorphize)'s own module doc comment
//! already named this exact seam when it was extracted, calling out
//! [`enum_member_attr_type`] and the `unroll_enum_loops` family as
//! "the enum-lowering pass, a separate seam with its own boundary" that it
//! deliberately left behind for a later extraction.
//!
//! The seam is cohesive because every item here exists to serve PEP 435
//! enum support end to end: typing an enum member access
//! (`enum_member_attr_type`, called from [`infer_expr_in`](crate::infer_expr_in)),
//! type-checking an enum-iteration loop body in both module and function
//! scope ([`check_enum_loop_body_module`] / [`check_enum_loop_body_function`],
//! called from `check_stmt` and its function-scope counterpart in
//! `lib.rs`, so both stay `pub(crate)` and are re-exported at the crate
//! root), and the
//! post-type-check, post-monomorphization rewrite that expands every
//! `HirStmt::ForList` over an enum class into its unrolled equivalent
//! ([`unroll_enum_loops`], called once from
//! [`check_and_resolve`](crate::check_and_resolve), after that function's
//! own call to `monomorphize`: the unrolling only needs to run once the
//! HIR is fully type-checked and every class's `enum_members` table is
//! final, and it must not run before `monomorphize` resolves generic
//! calls, or a generic function whose body iterates an enum would still
//! carry unresolved `Ty::Param` positions when the unrolled copies are
//! spliced in). `build_enum_member_table` and `unroll_enum_loops_in_stmts`
//! are private helpers of `unroll_enum_loops` alone.
//!
//! Everything else in `lib.rs` that these functions call into --
//! `check_assignment`, `join_loop_body`, the `narrow` module's
//! re-entrant-loop helpers -- stays where it was; this module reaches them
//! as an ordinary descendant of the crate root, exactly like
//! `class::binding` and `exception` reach `class`'s own `pub(crate)`
//! `check_call_args`.

use crate::narrow;
use crate::{BindingState, Environment};
use pycc_diag::Diagnostic;
use pycc_hir::{ForeignImportSite, HirExpr, HirItem, HirModule, HirStmt, ImportBinding, Ty};
use std::collections::HashMap;

use super::{check_assignment, join_loop_body};

/// #379 (PR-19): Return the type of an enum member accessed by name
/// (`Color.RED`), or `None` if `class_def` is not an enum class or `attr`
/// is not one of its members. Extracted from `infer_expr_in` to isolate
/// the enum-specific code paths (see cargo-llvm-cov#276 for the coverage
/// instantiation issue).
pub(crate) fn enum_member_attr_type(
    class_def: &crate::HirClassDef,
    class_name: &str,
    attr: &str,
) -> Option<Ty> {
    if !class_def.enum_members.is_empty()
        && class_def.enum_members.iter().any(|(name, _)| name == attr)
    {
        Some(Ty::Instance(Box::new(class_name.to_string())))
    } else {
        None
    }
}

/// #379 (PR-19): Type-check an enum class iteration loop body. Binds `var`
/// to `Ty::Instance(list)`, checks each body statement, then joins the body
/// environment back. Extracted from `check_stmt` and
/// `check_stmt_in_function` to isolate the enum-specific code paths (see
/// cargo-llvm-cov#276 for the coverage instantiation issue). Two variants
/// exist (module-scope and function-scope) to avoid generic closure
/// monomorphization producing separate coverage records per call site.
pub(crate) fn check_enum_loop_body_module(
    env: &mut Environment,
    var: &str,
    list: &str,
    body: &[HirStmt],
) -> Result<(), Diagnostic> {
    let var_ty = Ty::Instance(Box::new(list.to_string()));
    let was_definite = matches!(env.binding_state(var), Some(BindingState::Definitely(_)));
    check_assignment(env, var, var_ty)?;
    let mut body_env = env.clone();
    // Issue #769 follow-up (D-068 re-review round 3): an enum loop with
    // more than one member re-runs `body` once per member, same
    // re-entrant shape as any other loop -- see `narrow::apply_kill_prescan`.
    narrow::apply_kill_prescan(&mut body_env, body);
    narrow::apply_delete_prescan(&mut body_env, body, Some(var));
    narrow::check_stmt_sequence(&mut body_env, body)?;
    join_loop_body(env, &body_env);
    if !was_definite && let Some(ty) = env.lookup_any(var) {
        env.bind_maybe(var.to_string(), ty);
    }
    Ok(())
}

/// #379 (PR-19): Function-scope variant of `check_enum_loop_body_module`.
/// Checks each body statement via `check_stmt_in_function` with the
/// enclosing function's `local_names` and `return_ty`.
pub(crate) fn check_enum_loop_body_function(
    env: &mut Environment,
    var: &str,
    list: &str,
    body: &[HirStmt],
    local_names: &[&str],
    return_ty: Ty,
) -> Result<(), Diagnostic> {
    let var_ty = Ty::Instance(Box::new(list.to_string()));
    let was_definite = matches!(env.binding_state(var), Some(BindingState::Definitely(_)));
    check_assignment(env, var, var_ty)?;
    let mut body_env = env.clone();
    // Issue #769 follow-up (D-068 re-review round 3): see
    // `check_enum_loop_body_module`'s identical comment.
    narrow::apply_kill_prescan(&mut body_env, body);
    narrow::apply_delete_prescan(&mut body_env, body, Some(var));
    narrow::check_stmt_sequence_in_function(&mut body_env, local_names, body, return_ty.clone())?;
    join_loop_body(env, &body_env);
    if !was_definite && let Some(ty) = env.lookup_any(var) {
        env.bind_maybe(var.to_string(), ty);
    }
    Ok(())
}

/// #379 (PR-19): Build a lookup table mapping enum class name to its
/// member names (in source order). Extracted from `unroll_enum_loops` to
/// isolate the enum-specific code paths (see cargo-llvm-cov#276 for the
/// coverage instantiation issue).
fn build_enum_member_table(
    class_defs: &[(String, crate::HirClassDef)],
) -> HashMap<&str, Vec<&String>> {
    class_defs
        .iter()
        .filter(|(_, cd)| !cd.enum_members.is_empty())
        .map(|(name, cd)| {
            (
                name.as_str(),
                cd.enum_members.iter().map(|(mn, _)| mn).collect(),
            )
        })
        .collect()
}

/// The rewrite walks both top-level items and function bodies (a
/// `ForList`-over-enum can appear inside a function). Inside a function
/// body, the enclosing `Vec<HirStmt>` is spliced in place: the unrolled
/// statements replace the original `ForList` statement at its position.
///
/// Limitations (matching the plan's risk-1): no `break`/`continue`/`else`
/// in an enum-loop body (already unimplemented for all v0.3 loops, so no
/// real loss); code size is linear in member-count × body-size (acceptable
/// for v0.3 fixtures). A module with no enum classes is returned unchanged.
pub(crate) fn unroll_enum_loops(mut hir: HirModule) -> Result<HirModule, Diagnostic> {
    // Fast path: if no class is an enum class, there is nothing to unroll.
    // The item list is returned untouched, so every recorded foreign-import
    // position stays valid without a remap.
    let has_enum = hir
        .class_defs
        .iter()
        .any(|(_, cd)| !cd.enum_members.is_empty());
    if !has_enum {
        return Ok(hir);
    }
    // Build a lookup table: enum class name -> member names (in source order).
    let enum_members = build_enum_member_table(&hir.class_defs);
    // Walk top-level items and function bodies, splicing unrolled statements.
    // `produced[i]` is how many items original item `i` became, which is what
    // the recorded foreign-import positions are remapped through below.
    let mut new_items: Vec<HirItem> = Vec::with_capacity(hir.items.len());
    let mut produced: Vec<usize> = Vec::with_capacity(hir.items.len());
    for item in hir.items.drain(..) {
        match item {
            HirItem::TopLevelStmt(stmt) => {
                let mut unrolled =
                    unroll_enum_loops_in_stmts(std::slice::from_ref(&stmt), &enum_members);
                produced.push(unrolled.len());
                for s in unrolled.drain(..) {
                    new_items.push(HirItem::TopLevelStmt(s));
                }
            }
            HirItem::Function {
                name,
                params,
                return_ty,
                body,
            } => {
                let body = unroll_enum_loops_in_stmts(&body, &enum_members);
                produced.push(1);
                new_items.push(HirItem::Function {
                    name,
                    params,
                    return_ty,
                    body,
                });
            }
        }
    }
    hir.items = new_items;
    hir.imports = remap_foreign_import_positions(&hir.imports, &produced);
    Ok(hir)
}

/// Recomputes each foreign import's recorded item position against a list
/// this pass rewrote, where original item `i` became `produced[i]` items.
///
/// A `ForeignImportSite::Item` index is a position in the item list as it
/// stood when the `import` lowered, and `pycc_mir::splice_foreign_imports`
/// reads it as a position in the final list. Unrolling a top-level
/// `for c in Color:` replaces one item with one per member, so every
/// position recorded after such a loop refers to the wrong item unless it is
/// recomputed -- the import would land *inside* the unrolled sequence,
/// running before top-level statements that precede it in source. The new
/// position is the number of items the prefix before it produced, which is
/// where the import still belongs: everything that preceded it, however many
/// items it became, still precedes it.
///
/// A position past the end cannot arise: this pass never drops an item
/// (`produced[i] >= 1` for every `i`, since a non-enum statement unrolls to
/// itself), and `min(produced.len())` clamps the trailing-import position
/// that records the item count itself.
fn remap_foreign_import_positions(
    imports: &[ImportBinding],
    produced: &[usize],
) -> Vec<ImportBinding> {
    imports
        .iter()
        .map(|binding| match binding {
            ImportBinding::Foreign {
                local_name,
                module_path,
                from,
                site,
                span,
            } => ImportBinding::Foreign {
                local_name: local_name.clone(),
                module_path: module_path.clone(),
                from: from.clone(),
                // A block import has no item position (#1291).
                site: match site {
                    ForeignImportSite::Item(index) => ForeignImportSite::Item(
                        produced[..(*index).min(produced.len())].iter().sum(),
                    ),
                    site @ ForeignImportSite::Block { .. } => *site,
                },
                span: *span,
            },
            other => other.clone(),
        })
        .collect()
}

/// Helper for `unroll_enum_loops`: walks a `Vec<HirStmt>`, splicing any
/// `ForList`-over-enum into its unrolled equivalent. Recurses into nested
/// statement bodies (if/while/for bodies) so an enum loop inside a nested
/// block is also unrolled. Returns a new `Vec<HirStmt>` with all enum loops
/// expanded.
fn unroll_enum_loops_in_stmts(
    stmts: &[HirStmt],
    enum_members: &HashMap<&str, Vec<&String>>,
) -> Vec<HirStmt> {
    let mut result: Vec<HirStmt> = Vec::new();
    for stmt in stmts {
        match stmt {
            HirStmt::ForList { var, list, body } => {
                // Check if `list` is an enum class name.
                if let Some(members) = enum_members.get(list.as_str()) {
                    // Unroll: for each member, emit `var = <enum>.<member>`
                    // then the body (cloned).
                    let body = body.clone();
                    for member_name in members {
                        result.push(HirStmt::Assign {
                            target: var.clone(),
                            value: HirExpr::AttrGet {
                                base: Box::new(HirExpr::Name(list.clone())),
                                attr: (*member_name).clone(),
                            },
                        });
                        result.extend(body.iter().cloned());
                    }
                } else {
                    // Not an enum loop -- keep as-is, but recurse into the
                    // body in case it contains a nested enum loop.
                    result.push(HirStmt::ForList {
                        var: var.clone(),
                        list: list.clone(),
                        body: unroll_enum_loops_in_stmts(body, enum_members),
                    });
                }
            }
            // Recurse into nested statement bodies.
            HirStmt::If { test, body, orelse } => {
                result.push(HirStmt::If {
                    test: test.clone(),
                    body: unroll_enum_loops_in_stmts(body, enum_members),
                    orelse: unroll_enum_loops_in_stmts(orelse, enum_members),
                });
            }
            HirStmt::While { test, body } => {
                result.push(HirStmt::While {
                    test: test.clone(),
                    body: unroll_enum_loops_in_stmts(body, enum_members),
                });
            }
            HirStmt::ForRange {
                var,
                start,
                stop,
                step,
                body,
            } => {
                result.push(HirStmt::ForRange {
                    var: var.clone(),
                    start: start.clone(),
                    stop: stop.clone(),
                    step: step.clone(),
                    body: unroll_enum_loops_in_stmts(body, enum_members),
                });
            }
            // PR 3c of #1082 (Part 3 of #1026): `for x in <object>:` carries
            // a body exactly like every other loop above, so an enum `for`
            // loop nested inside it must be unrolled here too. Without this
            // arm the `other` catch-all below would clone the node whole and
            // a `ForList`-over-enum inside it would reach MIR lowering
            // unexpanded, panicking on the enum class name's missing type.
            HirStmt::ForObject { var, iter, body } => {
                result.push(HirStmt::ForObject {
                    var: var.clone(),
                    iter: iter.clone(),
                    body: unroll_enum_loops_in_stmts(body, enum_members),
                });
            }
            HirStmt::Try {
                body,
                handlers,
                orelse,
                finalbody,
            } => {
                result.push(HirStmt::Try {
                    body: unroll_enum_loops_in_stmts(body, enum_members),
                    handlers: handlers
                        .iter()
                        .map(|h| pycc_hir::HirExceptHandler {
                            exc_type: h.exc_type.clone(),
                            name: h.name.clone(),
                            body: unroll_enum_loops_in_stmts(&h.body, enum_members),
                        })
                        .collect(),
                    orelse: unroll_enum_loops_in_stmts(orelse, enum_members),
                    finalbody: unroll_enum_loops_in_stmts(finalbody, enum_members),
                });
            }
            // `except*` (PEP 654, #542) shares `Try`'s recursion shape --
            // an enum `for` loop nested in a `try*` body, an `except*`
            // handler, `else`, or `finally` must be unrolled exactly like
            // its `Try` counterpart above, or a `ForList`-over-enum left
            // inside a `try*` would reach MIR lowering unexpanded.
            HirStmt::TryStar {
                body,
                handlers,
                orelse,
                finalbody,
            } => {
                result.push(HirStmt::TryStar {
                    body: unroll_enum_loops_in_stmts(body, enum_members),
                    handlers: handlers
                        .iter()
                        .map(|h| pycc_hir::HirExceptHandler {
                            exc_type: h.exc_type.clone(),
                            name: h.name.clone(),
                            body: unroll_enum_loops_in_stmts(&h.body, enum_members),
                        })
                        .collect(),
                    orelse: unroll_enum_loops_in_stmts(orelse, enum_members),
                    finalbody: unroll_enum_loops_in_stmts(finalbody, enum_members),
                });
            }
            // Every statement kind that carries a nested statement body is
            // enumerated above. The remaining kinds are body-less, so they
            // cannot contain a nested enum `for` loop and are cloned whole.
            // `HirStmt::Match` is the one exception: it carries case bodies
            // and reaches this catch-all, so an enum `for` loop inside a
            // `case` arm is not unrolled. That gap is pre-existing -- it
            // predates this arm and this pull request -- and is deliberately
            // not fixed here.
            other => result.push(other.clone()),
        }
    }
    result
}
