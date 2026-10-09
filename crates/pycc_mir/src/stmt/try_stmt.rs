//! `try`/`except` and `try`/`except*` lowering, and the narrowing overlay
//! across them.
//!
//! Extracted from `lower_stmt`'s two near-identical `HirStmt::Try`/
//! `HirStmt::TryStar` arms per AGENTS.md's file-decomposition rule (#1476).
//! The arms differed only in what an `as` name binds to, so one walk now
//! serves both, as `pycc_types::constraints::try_stmt` does on the solver
//! side.
//!
//! #1476: the overlay follows the constraint solver's
//! `collect_try_constraints` (whose verdict decides a program per D-220)
//! position by position, so every read the solver types as a narrowed
//! native value lowers to an unbox here and every read it types as
//! `object` does not:
//!
//! - the body starts from the pre-`try` overlay;
//! - each handler starts from the pre-`try` overlay less every name the
//!   body rebinds (it can run after any prefix of the body), and less its
//!   own `as` name;
//! - `else` continues from the body's end state, since it runs only after
//!   the body completed;
//! - `finally` starts from the pre-`try` overlay less every name any path
//!   (body, `else`, a handler's `as` name or body) rebinds;
//! - after the statement, a name stays narrowed only when every path that
//!   falls through (the `else` path unless the body or `else` always
//!   returns, and each handler that does not always return) still narrows
//!   it to the same type and `finally` does not rebind it. With no path
//!   falling through, the `finally` end state stands; nothing after the
//!   `try` runs.
//!
//! Before #1476 every position started from the pre-`try` overlay and the
//! statement restored it afterwards. An `else` or `finally` read after a
//! body rebinding was then unboxed although the solver typed it `object`
//! (a run-time `TypeError` where CPython succeeds), and a guard in the body
//! that the solver carried past the `try` was dropped here (a codegen panic
//! on the native read). The same overlay carries `Optional` narrowing, which
//! the check phase's `exception::try_join::join_try_outcome` joins the same
//! way.

use super::super::{
    HirClassDef, MirExceptHandler, MirStmt, apply_kill_prescan, bind, handler_type_tags,
    join_narrowed, kill_narrowing, lower_scoped_body, narrowed_scope_key, narrowing_snapshot,
    restore_narrowing,
};
use pycc_hir::{HirExceptHandler, HirStmt, Ty, block_always_returns, killed_names};
use std::collections::HashMap;

/// The parts of a `try` statement, and which of the two statement forms it
/// is.
pub(super) struct TryParts<'a> {
    pub(super) body: &'a [HirStmt],
    pub(super) handlers: &'a [HirExceptHandler],
    pub(super) orelse: &'a [HirStmt],
    pub(super) finalbody: &'a [HirStmt],
    /// `true` for `except*` (PEP 654, Part 3 of #382, #542): an `as` binding
    /// always resolves to `ExceptionGroup`, never the named handler type --
    /// `pycc_types::exception::check_try_star_stmt`'s rule at type-checking
    /// time. `false` for a plain `except`, whose `as` name binds the
    /// handler's own exception type.
    pub(super) star: bool,
}

/// Lowers a `try` or `try`/`except*` statement, leaving `scopes`' overlay
/// as the module doc describes.
pub(super) fn lower_try(
    parts: TryParts<'_>,
    scopes: &mut Vec<HashMap<String, Ty>>,
    classes: &HashMap<String, HirClassDef>,
    current_class: Option<&str>,
) -> MirStmt {
    let pre_try = narrowing_snapshot(scopes);
    let (body, body_end) = lower_scoped_body(parts.body, scopes, classes, current_class, None);
    let mut fallthrough = Vec::new();
    let handlers = parts
        .handlers
        .iter()
        .map(|h| {
            // Issue #769 follow-up (D-068 re-review round 3): a handler
            // runs only after *some* prefix of the body already executed,
            // so it must not see a narrowing the body could have killed
            // anywhere within it. Each handler starts from the pre-`try`
            // snapshot, not from the previous handler's pruning.
            restore_narrowing(scopes, pre_try.clone());
            apply_kill_prescan(scopes, parts.body);
            let (handler, end) = lower_handler(h, parts.star, scopes, classes, current_class);
            if !block_always_returns(&h.body) {
                fallthrough.push(end);
            }
            handler
        })
        .collect();
    restore_narrowing(scopes, body_end);
    let (orelse, else_end) = lower_scoped_body(parts.orelse, scopes, classes, current_class, None);
    if !block_always_returns(parts.body) && !block_always_returns(parts.orelse) {
        fallthrough.push(else_end);
    }
    restore_narrowing(scopes, pre_try);
    apply_kill_prescan(scopes, parts.body);
    apply_kill_prescan(scopes, parts.orelse);
    for h in parts.handlers {
        if let Some(name) = &h.name {
            kill_narrowing(scopes, name);
        }
        apply_kill_prescan(scopes, &h.body);
    }
    let (finalbody, finally_end) =
        lower_scoped_body(parts.finalbody, scopes, classes, current_class, None);
    let after = match fallthrough.split_first() {
        Some((first, rest)) => {
            let rest: Vec<&HashMap<String, Ty>> = rest.iter().collect();
            let mut joined = join_narrowed(first, &rest);
            for name in killed_names(parts.finalbody) {
                joined.remove(&narrowed_scope_key(&name));
            }
            joined
        }
        None => finally_end,
    };
    restore_narrowing(scopes, after);
    if parts.star {
        MirStmt::TryStar {
            body,
            handlers,
            orelse,
            finalbody,
        }
    } else {
        MirStmt::Try {
            body,
            handlers,
            orelse,
            finalbody,
        }
    }
}

/// Lowers one handler from the overlay the caller prepared, returning it
/// with the overlay its body ends in.
fn lower_handler(
    h: &HirExceptHandler,
    star: bool,
    scopes: &mut Vec<HashMap<String, Ty>>,
    classes: &HashMap<String, HirClassDef>,
    current_class: Option<&str>,
) -> (MirExceptHandler, HashMap<String, Ty>) {
    // PEP 758 (#740): a handler may name more than one exception type.
    // Union each named type's own tag set, then dedup -- overlapping
    // families (e.g. `OSError` and `ConnectionError` both include tags 10,
    // 19-22) would otherwise double-count.
    let exc_type_tag = h.exc_type.as_ref().map(|names| {
        let mut tags: Vec<u8> = names
            .iter()
            .flat_map(|name| handler_type_tags(name, classes))
            .collect();
        tags.sort_unstable();
        tags.dedup();
        tags
    });
    let binding_type = if star {
        Some("ExceptionGroup".to_string())
    } else {
        h.exc_type
            .as_ref()
            .map(|names| pycc_hir::except_handler_binding_type_name(names))
    };
    let binding_ty = h
        .name
        .as_ref()
        .zip(binding_type)
        .map(|(_, ty)| Ty::Instance(Box::new(ty)));
    if let (Some(name), Some(ty)) = (&h.name, &binding_ty) {
        // The type checker binds `except T as name` only in the handler's
        // cloned environment. MIR maintains its own type scopes, so record
        // the same binding before lowering expressions in the handler body.
        // A bare handler cannot have an `as` name in Python.
        bind(scopes, name.clone(), ty.clone());
        // D-068 re-review of #780 (fourth round): `bind` only overwrites the
        // type scope, never the narrowing sentinel, so a name narrowed
        // before the `try` would otherwise keep lowering reads of it inside
        // the handler to an unwrap or unbox, although it now holds the
        // caught exception.
        kill_narrowing(scopes, name);
    }
    let (body, end) = lower_scoped_body(&h.body, scopes, classes, current_class, None);
    (
        MirExceptHandler {
            exc_type_tag,
            binding_name: h.name.clone(),
            binding_ty,
            body,
        },
        end,
    )
}
