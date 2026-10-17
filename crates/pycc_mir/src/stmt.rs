//! HIR-to-MIR statement lowering (#546): `lower_stmt`, the per-`HirStmt`
//! dispatch that the crate root's `build`/`lower_item` walk drives.

use super::expr::pre_bind_named_expr_targets;
use super::matching::lower_match;
use super::{
    HirClassDef, MirExpr, MirStmt, bind, bind_variable, lookup, lower_expr, lower_raise,
    resolve_comp_source,
};
use pycc_hir::{HirStmt, Ty};
use std::collections::HashMap;

mod comp_assign;
mod for_list;
mod return_tuple;
mod store;
mod try_stmt;

use comp_assign::object_comp_assign;
use return_tuple::coerce_returned_tuple;

/// Blocker fix (D-068 review of #780): shared loop-body lowering helper for
/// `While`/`ForRange`/`ForList`'s three arms below -- lowers `body` via
/// `lower_scoped_body`, then reconciles the narrowing overlay the same way
/// `pycc_types::join_loop_body` does: a name stays narrowed after the loop
/// only if it is narrowed to the same type whether the loop ran zero times
/// (the pre-loop snapshot) or ran the body at least once (the body's own
/// ending state). See `lower_scoped_body`'s doc comment for why this needs
/// a second, explicit `restore_narrowing` call rather than trusting its own
/// internal one.
fn lower_loop_body(
    body: &[HirStmt],
    scopes: &mut Vec<HashMap<String, Ty>>,
    classes: &HashMap<String, HirClassDef>,
    current_class: Option<&str>,
) -> Vec<MirStmt> {
    // Issue #769 follow-up (D-068 re-review round 3): a loop body can be
    // re-entered, so a read inside it can execute after a kill from a
    // prior iteration even though the kill comes later in source order.
    // Prescan-drop every name `body` kills before lowering it -- see
    // `super::apply_kill_prescan`'s doc comment. Applied on top of
    // `scopes`' current state before `lower_scoped_body` takes its own
    // isolating snapshot, so the pruning is visible for the lowering
    // below but the `pre_snapshot` used by the post-loop join already
    // reflects it too (matching `pycc_types::narrow`'s equivalent fix,
    // which mutates the same `env` both the pre-loop test and the loop
    // body itself read from).
    super::apply_kill_prescan(scopes, body);
    let pre_snapshot = super::narrowing_snapshot(scopes);
    let (body, body_end) = super::lower_scoped_body(body, scopes, classes, current_class, None);
    super::restore_narrowing(scopes, super::join_narrowed(&pre_snapshot, &[&body_end]));
    body
}

pub(super) fn lower_stmt(
    stmt: &HirStmt,
    scopes: &mut Vec<HashMap<String, Ty>>,
    classes: &HashMap<String, HirClassDef>,
    current_class: Option<&str>,
) -> MirStmt {
    match stmt {
        HirStmt::ExprStmt(pycc_hir::HirExpr::Call { callee, args })
            if callee == pycc_hir::YIELD_MARKER =>
        {
            MirStmt::Yield(lower_expr(&args[0], scopes, classes, current_class))
        }
        HirStmt::ExprStmt(pycc_hir::HirExpr::Call { callee, args })
            if callee == pycc_hir::GEN_RETURN_MARKER =>
        {
            MirStmt::GenReturn(lower_expr(&args[0], scopes, classes, current_class))
        }
        HirStmt::ExprStmt(pycc_hir::HirExpr::Call { callee, args })
            if callee == pycc_hir::YIELD_FROM_MARKER =>
        {
            crate::generator::lower_yield_from_stmt(&args[0], scopes, classes, current_class)
        }
        HirStmt::ExprStmt(expr) => {
            // PEP 572 (#774): bind before lowering the whole expression --
            // see `pre_bind_named_expr_targets`'s own doc comment for why a
            // walrus value that references an earlier walrus in the very
            // same expression (`(a := 1) + (b := a + 1)`) requires this.
            pre_bind_named_expr_targets(expr, scopes, classes, current_class);
            let expr = lower_expr(expr, scopes, classes, current_class);
            // PEP 572 (#774): a bare expression statement is one of the
            // three placements `pycc_hir::stmt::lower_stmt`'s own
            // `contains_named_expr` restriction permits a walrus in (`(n :=
            // 5)` alone on a line). `name` must be bound into `scopes`
            // even though nothing *within this statement* looks it up,
            // because a later statement in the same block might (`(n :=
            // 5)\nprint(n)`) -- `MirExpr::collect_named_expr_bindings`
            // finds every `NamedExpr` this lowered expression contains, at
            // any nesting depth, so this also covers a walrus buried in a
            // larger expression statement like `f(n := 5)`.
            let mut named_bindings = Vec::new();
            expr.collect_named_expr_bindings(&mut named_bindings);
            for (name, ty) in named_bindings {
                bind_variable(scopes, name, ty);
            }
            MirStmt::ExprStmt(expr)
        }
        HirStmt::Assign { target, value } => {
            // #1476: a local's first binding from a bare narrowed `object`
            // read keeps the object, so the slot is declared `object` as
            // `pycc_types`' first-binding rule declares it. "First" means no
            // earlier binding anywhere in the current frame, which -- the
            // frame being per function, never per branch -- matches the
            // checker's `lookup_any`, maybe-bound sibling branches included.
            let first_object_binding = !scopes
                .last()
                .expect("at least one scope is always present")
                .contains_key(target)
                && super::object_narrow::is_bare_narrowed_object_read(value, scopes);
            let value = lower_expr(value, scopes, classes, current_class);
            let value = if first_object_binding {
                super::object_narrow::object_operand(value)
            } else {
                value
            };
            // D-197 follow-up (#763/#770 review): if `target` is already
            // scoped as `Optional[inner]` -- from an earlier `AnnAssign`,
            // valued or not -- and this plain reassignment's own value
            // does not already report that same `Ty::Optional`, widen it
            // via `MirExpr::OptionalWrap`, mirroring `AnnAssign`'s own
            // `Some(value)` arm below exactly. Without this, `x: int |
            // None; x = 5` would lower `x = 5` as a bare `Ty::Int` value
            // with nothing to say `x` was ever declared `Optional[int]`,
            // and the very next line below (`bind_variable`'s `or_insert`
            // is a no-op once a scope entry exists, but `pycc_codegen`'s
            // own `collect_stmt_bindings` derives a target's *storage
            // slot* type from the first `MirStmt::Assign` value it sees,
            // entirely independently of this lowering pass's `scopes`)
            // would predeclare `x`'s slot as plain `Ty::Int` -- confirmed
            // empirically as the root cause of a codegen panic on `x is
            // None` (a non-`Optional` operand reaching an `is`/`is not`
            // comparison that `pycc_types::check`'s T0021 is supposed to
            // have ruled out, but here the source is entirely valid PEP
            // 604 code; only this lowering pass was dropping the
            // annotation). `lookup`'s own panic-on-miss behavior is
            // deliberately not used here: a target with no prior scope
            // entry at all (the ordinary, non-`Optional` first-assignment
            // case) must fall through unchanged.
            let value = match scopes
                .iter()
                .rev()
                .find_map(|scope| scope.get(target).cloned())
            {
                Some(Ty::Optional(inner)) if value.ty() != Ty::Optional(inner.clone()) => {
                    MirExpr::OptionalWrap(Box::new(value), inner)
                }
                // An `int`, `bool` or `float` rebinding an `int | float`
                // name keeps the name's `int | float` slot.
                Some(Ty::Num) if value.ty() != Ty::Num => MirExpr::NumWrap(Box::new(value)),
                // Part 2 of #1387 (#1475): a native value rebinding an
                // `object`-typed name is boxed into the name's
                // `PyObject *` slot. Only the current frame's own binding
                // counts: a function body's first `x = 1` is a fresh local
                // even when the module binds an `object` `x`, exactly as
                // `pycc_types`' `function_local_names` treats it, so it
                // must not box against the outer frame.
                _ => match scopes
                    .last()
                    .expect("at least one scope is always present")
                    .get(target)
                {
                    Some(slot_ty) => crate::object_box::box_into(value, slot_ty),
                    None => value,
                },
            };
            // The first assignment fixes a binding's representation.
            // In particular, assigning `bool` to an existing `int` is
            // accepted by the type checker but must not silently change the
            // later MIR name type from tagged i64 to i8.
            bind_variable(scopes, target.clone(), value.ty());
            // Issue #769 (Part 2 of #747): a reassignment kills any
            // narrowing currently recorded for `target` -- mirroring
            // `pycc_types::lib::check_assignment`'s own unconditional
            // `env.narrowed.remove(target)` at the checker layer. Without
            // this, a read of `target` later in the same narrowed branch
            // (after this assignment overwrote it) would keep emitting a
            // stale `OptionalUnwrap` for a value the checker never proved
            // present.
            super::kill_narrowing(scopes, target);
            MirStmt::Assign {
                target: target.clone(),
                value,
            }
        }
        HirStmt::AnnAssign {
            target,
            annotation,
            value: Some(value),
            is_final: _,
        } => {
            let value = if *annotation == Ty::Int {
                // An `IntEnum` member stored in an `int` name is its value.
                crate::expr::int_enum::lower_as_int(value, scopes, classes, current_class)
            } else {
                lower_expr(value, scopes, classes, current_class)
            };
            // `pycc_types::is_assignable` accepts an annotated initializer
            // in exactly two shapes: an exact type match, or a `bool`
            // initializer under an `int` annotation (`bool` is an `int`
            // subtype -- the only widening `is_assignable` allows). Unlike
            // plain `Assign` (whose bound type and lowered value type are
            // always the same, since both come from `value`), `pycc_types`
            // itself binds its checker `env` to the *annotation's* type for
            // `AnnAssign` (`check_assignment(env, target, *annotation)`,
            // not the initializer's inferred type) specifically so a later
            // annotated re-declaration is checked consistently -- see its
            // own comment citing this exact invariant. D-074's "first
            // assignment fixes a binding's representation" rule then
            // requires this lowering to agree, or a later plain
            // reassignment (`x: int = True; x = 5`) would silently widen
            // into a slot still permanently sized for `bool` (confirmed
            // empirically before this fix: the program above printed `11`,
            // the raw tagged-int bit pattern truncated through an `i8`
            // slot, instead of `5`). Keep the static `int` slot without
            // manufacturing arithmetic: `True + 0` is the integer `1`, but
            // an annotated boundary must retain the runtime identity
            // `True`.
            let value = if value.ty() == *annotation {
                value
            } else if value.ty() == Ty::Bool && *annotation == Ty::Int {
                // `bool` initializer under an `int` annotation — widen
                // to `int` via `IntBoundary` (D-141), preserving the
                // runtime `bool` identity while reporting `Ty::Int`.
                MirExpr::IntBoundary(Box::new(value))
            } else if let Some(arity) = float_tuple_annotation_arity(annotation)
                .filter(|_| matches!(value.ty(), Ty::Object))
            {
                // Part 4 of #1026 (PR 4c of #1083): a foreign CPython object
                // under a fixed-arity all-`float` tuple annotation. Unlike
                // the two widenings above this is a *converting* node with
                // a failure edge, not a representation fix-up -- see
                // `MirExpr::ObjUnpackFloatTuple`'s own documentation. The
                // arity comes back from the predicate itself rather than
                // from a second destructuring of `annotation`: this is the
                // one place it is still visible, and a separate `let
                // Ty::Tuple(..) = annotation else { unreachable!() }` would
                // be an unreachable line the D-242 coverage gate cannot
                // account for.
                MirExpr::ObjUnpackFloatTuple {
                    base: Box::new(value),
                    arity,
                }
            } else if *annotation == Ty::Object {
                // Part 2 of #1387: a native initializer under an `object`
                // annotation is boxed, so the slot is the `PyObject *` one
                // (the value is not `object` here: the first arm took that).
                crate::object_box::box_into(value, annotation)
            } else if *annotation == Ty::Num {
                // `int | float`: the slot is the `int | float` word.
                MirExpr::NumWrap(Box::new(value))
            } else if matches!(annotation, Ty::List(element) if **element == Ty::Num)
                && let MirExpr::ListLiteral(elements) = value
            {
                // `xs: list[int | float] = [1, 2]`: a display of numbers
                // under that annotation holds `int | float` words.
                MirExpr::ListLiteral(
                    elements
                        .into_iter()
                        .map(|element| match element.ty() {
                            Ty::Num => element,
                            _ => MirExpr::NumWrap(Box::new(element)),
                        })
                        .collect(),
                )
            } else if let Ty::Optional(inner) = annotation {
                // `T | None` (PEP 604, D-197, #763, Part 1 of #747): a bare
                // `None` initializer, or a bare `inner`-typed (or
                // `inner`-assignable, e.g. `bool` under `Optional[int]`)
                // initializer, under an `Optional[inner]` annotation widens
                // via `OptionalWrap` -- mirroring `IntBoundary`'s identical
                // "fix `.ty()` so `collect_stmt_bindings` derives the right
                // slot representation" reason immediately above; see
                // `OptionalWrap`'s own doc comment for why a wrapper node
                // is needed here but not for a later plain reassignment.
                MirExpr::OptionalWrap(Box::new(value), inner.clone())
            } else {
                value
            };
            // #380 (PR-20): when the annotation is a protocol type, bind
            // with the value's concrete type instead — the protocol type
            // is a compile-time-only interface, and the MIR needs the
            // concrete type for method/attribute resolution (static
            // dispatch). `pycc_types` already validated conformance and
            // binds the concrete type in its own environment; the MIR
            // must agree.
            let bind_ty = if matches!(annotation, Ty::Protocol(_)) {
                value.ty()
            } else {
                annotation.clone()
            };
            // Issue #769 (Part 2 of #747): same kill-on-assignment as the
            // plain `Assign` arm above -- a re-annotated declaration is
            // still a reassignment of `target`.
            super::kill_narrowing(scopes, target);
            bind_variable(scopes, target.clone(), bind_ty);
            MirStmt::Assign {
                target: target.clone(),
                value,
            }
        }
        HirStmt::AnnAssign {
            target,
            annotation: annotation @ Ty::Optional(_),
            value: None,
            is_final: _,
        } => {
            // D-197 follow-up (#763/#770 review): unlike every other
            // value-less `AnnAssign` (the catch-all arm immediately below,
            // which deliberately binds nothing at all -- see
            // `a_value_less_annotated_assignment_does_not_bind_the_name`),
            // an `Optional[inner]` declaration's type must be recorded in
            // `scopes` even with no initializer. `pycc_types::check_
            // assignment` already tracks this in its own separate
            // `Environment::declared` map and treats the *declared* type,
            // not a later plain reassignment's own bare value type, as the
            // sticky representation from that point on (its own comment
            // cites the same invariant). MIR lowering must agree: without
            // this, a later plain `x = 5` lowers via the `HirStmt::Assign`
            // arm above with no record that `x` was ever declared
            // `Optional[int]`, so that arm's own `Optional`-rewrap check
            // finds nothing in scope and leaves the value as bare `Ty::
            // Int` -- reproducing the exact defect this arm exists to fix:
            // `x: int | None; x = 5; print(x is None)` panicked in codegen
            // because `collect_stmt_bindings` had predeclared `x`'s slot as
            // plain `Ty::Int` from that first real `MirStmt::Assign`, and a
            // non-`Optional` operand then reached the `is`/`is not`
            // lowering that assumes one.
            //
            // This does not weaken definite-assignment checking: recording
            // a declared type here is not the same as putting `target` in
            // a "has a value" state anywhere upstream. `pycc_types::check`
            // runs its own separate, complete definite-assignment pass
            // *before* `pycc_mir::build` ever sees this HIR (see `src/
            // main.rs`), so a program that reads `x` before any real
            // assignment is already rejected there and never reaches this
            // lowering pass at all. This binding is consulted only by the
            // plain `HirStmt::Assign` arm's own later-reassignment check
            // above.
            super::kill_narrowing(scopes, target);
            bind_variable(scopes, target.clone(), annotation.clone());
            MirStmt::NoOp
        }
        HirStmt::AnnAssign { value: None, .. } => MirStmt::NoOp,
        HirStmt::If { test, body, orelse } => {
            // PEP 572 (#774): bind before lowering, mirroring `ExprStmt`'s
            // own ordering rationale above.
            pre_bind_named_expr_targets(test, scopes, classes, current_class);
            let lowered_test = lower_expr(test, scopes, classes, current_class);
            // PEP 572 (#774): an `if` test condition is one of the three
            // placements a walrus is permitted in. Bound *before* lowering
            // `body`/`orelse` (not after, unlike `ExprStmt`'s own
            // after-the-fact bind, since here there genuinely is code
            // within this same statement -- the branches -- that can
            // reference the bound name: `if (n := f()) > 0: print(n)`).
            let mut named_bindings = Vec::new();
            lowered_test.collect_named_expr_bindings(&mut named_bindings);
            for (name, ty) in named_bindings {
                bind_variable(scopes, name, ty);
            }
            // Issue #769 (Part 2 of #747): recognize the same top-level
            // `name is None` / `name is not None` shape the checker does
            // (`pycc_hir::optional_none_test`, the shared recognizer), and
            // -- only when `name` is *currently* scoped as `Ty::Optional`
            // -- push a `$narrowed:{name}` sentinel into the top `scopes`
            // frame before lowering the one branch that is reachable only
            // when the value is present, exactly mirroring
            // `pycc_types::narrow::apply_branch_narrowing`'s own polarity
            // split: `is not None` narrows only `body`; `is None` narrows
            // only `orelse`. The sentinel is popped again immediately after
            // that branch's statements are lowered, so it never leaks past
            // this `if` into a sibling statement (MIR's `scopes` frame is
            // shared, mutable, and per-function rather than per-branch, so
            // an unpopped sentinel would otherwise leak across the join the
            // same way the checker's own module doc comment describes for
            // its own, separate `narrowed` overlay). `test` here is the
            // original (pre-lowering) HIR node, matching the checker's own
            // recognizer, which is also HIR-level.
            //
            // #1476: `[not] isinstance(name, C)` on an `object` name narrows
            // the same way (`crate::object_narrow::narrowing_target`).
            let narrowing = super::object_narrow::narrowing_target(test, scopes, classes);
            let narrow_for = |side| match &narrowing {
                Some((name, inner, narrowed)) if *narrowed == side => {
                    Some((name.as_str(), inner.clone()))
                }
                _ => None,
            };
            let before = super::narrowing_snapshot(scopes);
            let body_narrow = narrow_for(super::object_narrow::NarrowSide::Body);
            let (body_hir, orelse_hir) = (body, orelse);
            let (body, mut body_end) =
                super::lower_scoped_body(body_hir, scopes, classes, current_class, body_narrow);

            let orelse_narrow = narrow_for(super::object_narrow::NarrowSide::Orelse);
            let (orelse, mut orelse_end) =
                super::lower_scoped_body(orelse_hir, scopes, classes, current_class, orelse_narrow);
            // #1476: the guard's own narrowing ends with its branch, so the
            // narrowed branch joins with the name's pre-`if` state -- a
            // nested `isinstance(o, bool)` under an `isinstance(o, int)`
            // guard keeps `o` an `int` after it. Mirrors
            // `pycc_types::narrow::end_branch_narrowing`.
            if let Some((name, _, side)) = &narrowing {
                let (end, branch) = match side {
                    super::object_narrow::NarrowSide::Body => (&mut body_end, body_hir),
                    super::object_narrow::NarrowSide::Orelse => (&mut orelse_end, orelse_hir),
                };
                super::end_branch_narrowing(&before, end, name, branch);
            }

            // Blocker fix (D-068 review of #780): an `if` always runs
            // exactly one of `body`/`orelse`, never neither, so the
            // narrowing state visible to whatever statement follows this
            // `if` in the enclosing sequence is the join of both branches'
            // own ending states -- not whatever `scopes` happened to hold
            // before this `if` ran (each `lower_scoped_body` call above
            // already reverted `scopes` back to exactly that pre-`if`
            // state on its own, so without this call the join would
            // silently never have happened at all, reverting any kill made
            // inside just one of the two branches -- e.g. `if flag: x =
            // None` nested inside an outer `if x is not None:` -- the
            // moment this `if` closed). Mirrors
            // `pycc_types::join_if_branches`'s identical fix exactly, one
            // layer down.
            super::restore_narrowing(scopes, super::join_narrowed(&body_end, &[&orelse_end]));

            MirStmt::If {
                test: lowered_test,
                body,
                orelse,
            }
        }
        HirStmt::Break => MirStmt::Break,
        HirStmt::Continue => MirStmt::Continue,
        // The loop, then its `else:` clause in exactly the shape the
        // checker gives it: the plain statement sequence when no `break`
        // can skip it, otherwise the body of `pycc_hir::loop_else_as_if`'s
        // opaque `if`, so both passes join bindings and narrowing alike.
        HirStmt::LoopElse { loop_stmt, orelse } => {
            let lowered_loop = lower_stmt(loop_stmt, scopes, classes, current_class);
            let orelse = if pycc_hir::loop_body(loop_stmt).is_some_and(pycc_hir::body_breaks_loop) {
                let as_if = pycc_hir::loop_else_as_if(orelse);
                match lower_stmt(&as_if, scopes, classes, current_class) {
                    MirStmt::If { body, .. } => body,
                    other => panic!(
                        "pycc_mir: internal error: an `if` lowered to {other:?}, not `MirStmt::If`"
                    ),
                }
            } else {
                super::lower_stmt_sequence(orelse, scopes, classes, current_class)
            };
            // A loop lowered with set-up statements before it (`for c in s`
            // first binds `list(s)`): the `else:` wraps the loop itself, the
            // one whose `break` must skip it, not the sequence.
            if let MirStmt::Seq(stmts) = &lowered_loop
                && stmts.last().is_some_and(|last| {
                    matches!(
                        last,
                        MirStmt::While { .. }
                            | MirStmt::ForRange { .. }
                            | MirStmt::ForList { .. }
                            | MirStmt::ForObject { .. }
                            | MirStmt::ForDict { .. }
                            | MirStmt::ForSet { .. }
                            | MirStmt::ForGen { .. }
                    )
                })
            {
                let MirStmt::Seq(mut stmts) = lowered_loop else {
                    unreachable!()
                };
                let inner = stmts.pop().expect("a last statement");
                stmts.push(MirStmt::LoopElse {
                    loop_stmt: Box::new(inner),
                    orelse,
                });
                return MirStmt::Seq(stmts);
            }
            MirStmt::LoopElse {
                loop_stmt: Box::new(lowered_loop),
                orelse,
            }
        }
        HirStmt::While { test, body } => {
            // Issue #769 follow-up (D-068 re-review round 3): `test`
            // re-executes on every iteration too, so it needs the same
            // prescan `lower_loop_body` applies to `body` below, applied
            // *before* `test` is lowered -- `lower_loop_body` reapplies it
            // to the same `scopes` before lowering `body` itself, which is
            // a harmless no-op the second time.
            super::apply_kill_prescan(scopes, body);
            // PEP 572 (#774): bind before lowering, mirroring `If`'s own
            // ordering just above.
            pre_bind_named_expr_targets(test, scopes, classes, current_class);
            let lowered_test = lower_expr(test, scopes, classes, current_class);
            // PEP 572 (#774): mirrors `If`'s own bind-before-body handling
            // just above -- a `while` test condition is the other
            // permitted placement, and a name it binds must be visible to
            // the loop body: `while (chunk := f()) is not None: use(chunk)`.
            let mut named_bindings = Vec::new();
            lowered_test.collect_named_expr_bindings(&mut named_bindings);
            for (name, ty) in named_bindings {
                bind_variable(scopes, name, ty);
            }
            // A `while x is not None:` (or `while x:`) test narrows an
            // `Optional` `x` in the body, as `pycc_types::narrow::
            // while_body_narrowing` does: the test runs before every trip,
            // and the kill prescan has already dropped a narrowing the
            // body could invalidate. The post-loop join drops it again
            // (the loop exits when the test is false).
            let narrowing = match super::object_narrow::narrowing_target(test, scopes, classes) {
                Some((name, inner, super::object_narrow::NarrowSide::Body))
                    if matches!(super::lookup(scopes, &name), Ty::Optional(_)) =>
                {
                    Some((name, inner))
                }
                _ => None,
            };
            let body = match narrowing {
                None => lower_loop_body(body, scopes, classes, current_class),
                Some((name, inner)) => {
                    super::apply_kill_prescan(scopes, body);
                    let pre_snapshot = super::narrowing_snapshot(scopes);
                    super::push_narrowing(scopes, &name, inner);
                    let (body, body_end) =
                        super::lower_scoped_body(body, scopes, classes, current_class, None);
                    super::restore_narrowing(scopes, super::join_narrowed(&pre_snapshot, &[&body_end]));
                    body
                }
            };
            MirStmt::While {
                test: lowered_test,
                body,
            }
        }
        HirStmt::ForRange {
            var,
            start,
            stop,
            step,
            body,
        } => {
            let range_operand = |e| crate::index_operand(e, crate::IndexUse::Range);
            let start = range_operand(crate::expr::int_enum::lower_as_int(start, scopes, classes, current_class));
            let stop = range_operand(crate::expr::int_enum::lower_as_int(stop, scopes, classes, current_class));
            let step = range_operand(crate::expr::int_enum::lower_as_int(step, scopes, classes, current_class));
            bind_variable(scopes, var.clone(), Ty::Int);
            // D-068 re-review of #780 (sixth round): the loop's own
            // induction variable is a rebinding exactly like `Assign`, but
            // `bind_variable` alone never clears a stale narrowing
            // sentinel -- mirrors the `Assign`/`AnnAssign` arms' own
            // `kill_narrowing`+`bind_variable` pairing above, and the
            // checker's `check_assignment(env, var, Ty::Int)` (`pycc_types`'
            // `ForRange` arm), which unconditionally clears `env.narrowed`.
            // Applied *before* `lower_loop_body`'s own snapshot so the
            // clear also survives past the loop's own close, matching the
            // checker's `env`/`body_env` split.
            super::kill_narrowing(scopes, var);
            let body = lower_loop_body(body, scopes, classes, current_class);
            MirStmt::ForRange {
                var: var.clone(),
                start,
                stop,
                step,
                body,
            }
        }
        HirStmt::ForList { var, list, body } => {
            for_list::lower_for_list(var, list, body, scopes, classes, current_class)
        }
        // PR 3c of #1082: `for x in o.attr:` / `for x in o.method(...):`.
        // `pycc_types` has already proved the iterable is `Ty::Object`, so
        // there is no type dispatch to make here -- unlike `ForList` above,
        // this node's shape is fixed by the check pass.
        HirStmt::ForObject { var, iter, body } => {
            let iter = super::lower_expr(iter, scopes, classes, current_class);
            // `bind`, not `bind_variable`: this is the first loop construct
            // whose target type can differ from a prior binding of the same
            // name (`ForList`/`ForRange` always bind `Ty::Int`, so
            // `bind_variable`'s `or_insert` was harmless for them).
            // `pycc_types`' own `ForObject` arm calls `env.bind`, which
            // overwrites, so `x = 5` followed by `for x in <object>:` leaves
            // `x` as `Ty::Object` in the checker; `or_insert` here would keep
            // the stale `Ty::Int` and make the body lower against the wrong
            // type (a `len(x)` in the body took the list path and panicked in
            // `pycc_codegen`). Scoped to this arm deliberately: making
            // `bind_variable` itself unconditional would change the recorded
            // type of every other construct that calls it, whereas
            // `HirStmt::ForObject` is new in this pull request, so no program
            // that compiles today can observe this change.
            bind(scopes, var.clone(), Ty::Object);
            // D-068 re-review of #780 (sixth round): see the `ForRange` arm's
            // identical comment above.
            super::kill_narrowing(scopes, var);
            let body = lower_loop_body(body, scopes, classes, current_class);
            MirStmt::ForObject {
                var: var.clone(),
                iter,
                body,
            }
        }
        HirStmt::ListCompAssign {
            target,
            var,
            iter,
            cond,
            elt,
        } => {
            if let Some(stmt) = object_comp_assign(
                target,
                var,
                iter,
                cond.as_deref(),
                pycc_hir::CompElt::List(elt.as_ref().clone()),
                scopes,
                classes,
                current_class,
            ) {
                return stmt;
            }
            let (source, var_ty) = resolve_comp_source(iter, var, scopes, classes, current_class);
            let cond = cond
                .as_deref()
                .map(|c| lower_expr(c, scopes, classes, current_class));
            let elt = lower_expr(elt, scopes, classes, current_class);
            bind_variable(scopes, target.clone(), Ty::List(Box::new(elt.ty())));
            // D-068 re-review of #780 (sixth round): the comprehension's
            // own result `target` is a rebinding exactly like `Assign`,
            // paralleling `resolve_comp_source`'s own `var` fix just above
            // -- must kill a stale narrowing sentinel too.
            super::kill_narrowing(scopes, target);
            MirStmt::ListCompAssign {
                target: target.clone(),
                var: var.clone(),
                var_ty,
                source,
                cond: cond.map(Box::new),
                elt: Box::new(elt),
            }
        }
        HirStmt::SetCompAssign {
            target,
            var,
            iter,
            cond,
            elt,
        } => {
            if let Some(stmt) = object_comp_assign(
                target,
                var,
                iter,
                cond.as_deref(),
                pycc_hir::CompElt::Set(elt.as_ref().clone()),
                scopes,
                classes,
                current_class,
            ) {
                return stmt;
            }
            let (source, var_ty) = resolve_comp_source(iter, var, scopes, classes, current_class);
            let cond = cond
                .as_deref()
                .map(|c| lower_expr(c, scopes, classes, current_class));
            let elt = lower_expr(elt, scopes, classes, current_class);
            // #1344: a `set[C]` element carries its class's hash and
            // equality, exactly as a set literal's does.
            let ops = crate::expr::set_ops::lower_set_element_ops(&elt.ty(), scopes, classes)
                .map(Box::new);
            bind_variable(scopes, target.clone(), Ty::Set(Box::new(elt.ty())));
            // D-068 re-review of #780 (sixth round): see `ListCompAssign`'s
            // identical comment above.
            super::kill_narrowing(scopes, target);
            MirStmt::SetCompAssign {
                target: target.clone(),
                var: var.clone(),
                var_ty,
                source,
                cond: cond.map(Box::new),
                elt: Box::new(elt),
                ops,
            }
        }
        HirStmt::DictCompAssign {
            target,
            var,
            iter,
            cond,
            key,
            value,
        } => {
            if let Some(stmt) = object_comp_assign(
                target,
                var,
                iter,
                cond.as_deref(),
                pycc_hir::CompElt::Dict {
                    key: key.as_ref().clone(),
                    value: value.as_ref().clone(),
                },
                scopes,
                classes,
                current_class,
            ) {
                return stmt;
            }
            let (source, var_ty) = resolve_comp_source(iter, var, scopes, classes, current_class);
            let cond = cond
                .as_deref()
                .map(|c| lower_expr(c, scopes, classes, current_class));
            let key = lower_expr(key, scopes, classes, current_class);
            let value = lower_expr(value, scopes, classes, current_class);
            bind_variable(
                scopes,
                target.clone(),
                Ty::Dict(Box::new((key.ty(), value.ty()))),
            );
            // D-068 re-review of #780 (sixth round): see `ListCompAssign`'s
            // identical comment above.
            super::kill_narrowing(scopes, target);
            MirStmt::DictCompAssign {
                target: target.clone(),
                var: var.clone(),
                var_ty,
                source,
                cond: cond.map(Box::new),
                key: Box::new(key),
                value: Box::new(value),
            }
        }
        // Part 2 of #1175 (#1179) splits this one HIR node on the sliced
        // base's recorded type, exactly as `HirStmt::DictSet` below splits
        // a store on `lookup`'s answer. `pycc_types` has already refused
        // every buffer slice this split must not see -- any `step`, an
        // artifact-owned base, a non-`int` bound, and the shape in every
        // position other than the whole `return` operand -- so reaching
        // `Ty::MemoryView` here means the admitted sub-range egress and
        // nothing else.
        HirStmt::Return(value) => {
            if let Some(pycc_hir::HirExpr::Slice {
                base,
                start,
                stop,
                step: None,
            }) = value.as_ref()
                && let pycc_hir::HirExpr::Name(name) = base.as_ref()
                && lookup(scopes, name) == Ty::MemoryView
            {
                MirStmt::ReturnBufferSlice {
                    name: name.clone(),
                    start: start
                        .as_ref()
                        .map(|bound| lower_expr(bound, scopes, classes, current_class)),
                    stop: stop
                        .as_ref()
                        .map(|bound| lower_expr(bound, scopes, classes, current_class)),
                }
            } else {
                let value = value
                    .as_ref()
                    .map(|v| lower_expr(v, scopes, classes, current_class));
                let declared = scopes.last().and_then(|frame| frame.get(RETURN_KEY)).cloned();
                MirStmt::Return(value.map(|value| coerce_returned_tuple(value, declared)))
            }
        }
        // `<name>[k] = v`. Part 1 of #1142 splits this one HIR node on the
        // base's recorded type, exactly as `lower_expr`'s own `Subscript`
        // arm splits the *load* on `base.ty()` -- `b[i]` there, `b[i] = v`
        // here. The mechanism differs only because `HirStmt::DictSet.dict`
        // is a bare `String` with no lowered expression to ask `ty()` of,
        // so the type comes from `lookup` instead. `pycc_types` has already
        // refused every store this split must not see.
        HirStmt::DictSet { dict, key, value } => {
            store::lower_item_store(dict, key, value, scopes, classes, current_class)
        }
        HirStmt::AttrSet { base, attr, value } => {
            store::lower_attr_store(base, attr, value, scopes, classes, current_class)
        }
        HirStmt::Match { subject, cases } => {
            lower_match(subject, cases, scopes, classes, current_class)
        }
        // #1476: one walk serves both statement forms; see `try_stmt`'s
        // module doc for the narrowing overlay across them.
        HirStmt::Try {
            body,
            handlers,
            orelse,
            finalbody,
        } => try_stmt::lower_try(
            try_stmt::TryParts {
                body,
                handlers,
                orelse,
                finalbody,
                star: false,
            },
            scopes,
            classes,
            current_class,
        ),
        // Part 3 of #382 (#542, PEP 654): `except*` lowers like `Try`,
        // except an `as` binding always resolves to `ExceptionGroup`.
        HirStmt::TryStar {
            body,
            handlers,
            orelse,
            finalbody,
        } => try_stmt::lower_try(
            try_stmt::TryParts {
                body,
                handlers,
                orelse,
                finalbody,
                star: true,
            },
            scopes,
            classes,
            current_class,
        ),
        HirStmt::Raise { exc, cause } => lower_raise(exc, cause, scopes, classes, current_class),
        // #1244: `del name` is static-only. The type checker has already
        // demoted the name's binding state, and under D-124's leak-only
        // model there is no reference to release, so nothing runs.
        HirStmt::Delete { name } => match scopes.iter().rev().find_map(|scope| scope.get(name)) {
            Some(Ty::Generator(_)) => MirStmt::ReleaseName(name.clone()),
            _ => MirStmt::NoOp,
        },
        // Part 2c of #1371: `pycc_types` admits a slice `del` only on a
        // CPython object base, so this is always the object deletion;
        // codegen's `expect_object_pointer` panics on any other base.
        HirStmt::DeleteSlice {
            base,
            start,
            stop,
            step,
            ..
        } => {
            let base = lower_expr(base, scopes, classes, current_class);
            let lower_bound = |bound: &Option<Box<pycc_hir::HirExpr>>| {
                bound
                    .as_deref()
                    .map(|bound| lower_expr(bound, scopes, classes, current_class))
            };
            MirStmt::ObjDelSlice {
                base,
                start: lower_bound(start),
                stop: lower_bound(stop),
                step: lower_bound(step),
            }
        }
        // Non-strict mode (#3): `pycc_types` admits a slice store only on a
        // CPython object base; the value is evaluated first.
        HirStmt::SetSlice {
            base,
            start,
            stop,
            step,
            value,
            ..
        } => {
            let value =
                crate::obj_arith::object_arg(lower_expr(value, scopes, classes, current_class));
            let base = lower_expr(base, scopes, classes, current_class);
            let lower_bound = |bound: &Option<Box<pycc_hir::HirExpr>>| {
                bound
                    .as_deref()
                    .map(|bound| lower_expr(bound, scopes, classes, current_class))
            };
            MirStmt::ObjSetSlice {
                base,
                start: lower_bound(start),
                stop: lower_bound(stop),
                step: lower_bound(step),
                value,
            }
        }
        // #1457: `pycc_types` admits an attribute `del` only on a CPython
        // object base, as it does a slice `del`.
        HirStmt::DeleteAttr { base, attr, .. } => MirStmt::ObjDelAttr {
            base: lower_expr(base, scopes, classes, current_class),
            attr: attr.clone(),
        },
        // A `dict`/`list` base deletes through `pycc_hir::builtins`'
        // `__pycc_delitem__`; anything else is a CPython object (#3).
        HirStmt::DeleteItem { base, key, .. } => {
            let base = lower_expr(base, scopes, classes, current_class);
            let key = lower_expr(key, scopes, classes, current_class);
            match crate::expr::builtin_call::lower_builtin_method(
                base,
                "__pycc_delitem__",
                vec![key],
            ) {
                Ok(call) => MirStmt::ExprStmt(call),
                Err((base, mut key)) => MirStmt::ObjDelItem {
                    base,
                    key: key.pop().expect("one key"),
                },
            }
        }
        HirStmt::ForeignImport { bindings, .. } => {
            // Each name holds the imported CPython object from here on: a
            // module global of a module-level block's import, already bound
            // so, or a function local (v0.8 accept 2).
            for (local_name, _, _) in bindings {
                bind(scopes, local_name.clone(), Ty::Object);
                super::kill_narrowing(scopes, local_name);
            }
            MirStmt::ForeignImport {
                bindings: bindings.clone(),
            }
        }
    }
}

/// The arity of `ty` when it is a fixed-arity tuple annotation whose every
/// element is `float` -- the shape a foreign CPython object may be assigned
/// to at a module-level annotated assignment (Part 4 of #1026, PR 4c of
/// #1083) -- and `None` otherwise. The arity is returned rather than a bare
/// `bool` so the caller never has to destructure `annotation` a second time
/// on a path it has already proved is a tuple.
///
/// The **canonical statement of this admission rule is
/// `pycc_types::foreign::is_object_float_tuple_annotation`**, which is what
/// actually decides whether the program compiles; this is its lowering-side
/// mirror and the two must not drift. It is restated rather than shared
/// because this crate does not depend on `pycc_types` -- the same split
/// `pycc_hir`'s shape checks and `pycc_types`' type checks already live on.
///
/// Arity zero is excluded here as well as there: `tuple[()]` never reaches
/// this crate at all (`pycc_hir` refuses it with `T0053`), so the guard is
/// defence in depth against a hand-built HIR module rather than a reachable
/// user program.
fn float_tuple_annotation_arity(ty: &Ty) -> Option<usize> {
    match ty {
        Ty::Tuple(elems)
            if !elems.is_empty() && elems.iter().all(|elem| matches!(elem, Ty::Float)) =>
        {
            Some(elems.len())
        }
        _ => None,
    }
}

/// The frame entry recording a function's declared native tuple return
/// type, for `return_tuple::coerce_returned_tuple`. Not an identifier, so no source
/// name can collide with it.
pub(crate) const RETURN_KEY: &str = "$return";
