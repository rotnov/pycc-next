//! A list display bound to a CPython object slot (Part 2d of #1371, D-258
//! rule 4 and its 2026-10-05 amendment).
//!
//! `x: object = []` and, after `s = value_stack[-size:]` bound `s` to an
//! object, `s = []` are both CPython lists in the source program. pycc has
//! no native `list[T]` that could stand in an object slot, so the display is
//! rewritten into [`HirExpr::ObjectList`], which builds a fresh CPython
//! `list` with every element boxed. Running here, inside the same pre-pass
//! that types `[]` from its slot, keeps the empty-container pass's key
//! property: `pycc check` and `pycc build` consume the identical node.
//!
//! **Sources.** The slot is an object when
//!
//! 1. the `AnnAssign` annotation is [`Ty::Object`] (`object`, `Any`, and in
//!    an `--ext` build every annotation that lowers to it), or
//! 2. there is no annotation and the target's flat whole-function binding
//!    (source 2 of the parent module) is [`Ty::Object`].
//!
//! An annotation of any other type wins over the binding, exactly as it does
//! for a native resolution, and is left to the parent's own sources.
//!
//! **Which displays.** Source 1 rewrites any list display, empty or not:
//! the annotation is written next to it and says what the display is.
//! Source 2 rewrites only the *empty* `[]`. A non-empty `[1, 2]` already has
//! a native type of its own, and the flat binding is order-insensitive (the
//! parent module's "Source 2 is order-insensitive" paragraph): rewriting
//! `xs = [1]` into an object because some *other* assignment binds `xs` to
//! an object would change the type of a display that compiles today, on the
//! strength of a binding that may follow it. Leaving it native keeps the
//! check phase's D-040 sticky-representation rule in charge of that mix. An
//! empty `[]` has no native type at all, so typing it from the binding can
//! only turn a `T0003` into an accepted program, never change one. A dict
//! display `{}` bound to an object stays `T0003`: D-258 rule 4's dict half
//! is not implemented yet.
//!
//! **Value positions (#1421).** A slot a declaration states -- source 1's
//! annotation, or a declared attribute slot (`attr_slot`'s object branch) --
//! receives more than a bare display: the #1207 subject's lines 43-44 store
//! `state_stack or [self.parse_conf.start_state]` and `value_stack or []`
//! into object-declared attributes. [`rewrite_value_displays`] therefore
//! rewrites every display in a *value position* of the stored expression:
//! the expression itself, both operands of a value-context `and`/`or` (either
//! may be the result), and both branches of a conditional expression. The
//! test of a conditional expression and the operands of a truth-only
//! `and`/`or` are consumed only for their truth and are never rewritten.
//! Source 2 keeps its bare-`[]` rule, for the order-insensitivity reason
//! above: its evidence is a binding, not a declaration written beside the
//! value.
//!
//! **Safety.** As for the parent's own source 2, this pass resolves but
//! never accepts: the check phase re-validates every assignment in program
//! order, so a rewritten `s = []` is accepted only where an object is
//! assignable to `s`, and `crate::foreign::list_display` refuses an element
//! with no boxing helper (`I0404`). A rewritten operand is typed by the
//! ordinary `and`/`or` and conditional-expression joins, which give an
//! object only where the other operand joins with one.

use super::*;

/// Rewrites `value` in place when its slot is an object (see the module
/// documentation for which displays qualify), returning whether anything
/// was rewritten.
pub(super) fn rewrite_object_slot(
    value: &mut HirExpr,
    target: &str,
    annotation: Option<&Ty>,
    env: &Environment,
) -> bool {
    match annotation {
        Some(Ty::Object) => rewrite_value_displays(value),
        Some(_) => false,
        None => {
            let rewrite = matches!(value, HirExpr::ListLiteral(elements) if elements.is_empty())
                && env
                    .binding_state(target)
                    .is_some_and(|state| *state.ty() == Ty::Object);
            if rewrite {
                *value = HirExpr::ObjectList(Vec::new());
            }
            rewrite
        }
    }
}

/// Rewrites every list display in a value position of `value` -- `value`
/// itself, both operands of a value-context `and`/`or`, both branches of a
/// conditional expression, recursively -- into [`HirExpr::ObjectList`],
/// returning whether any was rewritten. A display's own elements are not
/// value positions of the slot and are left alone.
pub(super) fn rewrite_value_displays(value: &mut HirExpr) -> bool {
    match value {
        HirExpr::ListLiteral(elements) => {
            *value = HirExpr::ObjectList(std::mem::take(elements));
            true
        }
        HirExpr::BoolOp {
            left,
            right,
            truth_only: false,
            ..
        } => rewrite_value_displays(left) | rewrite_value_displays(right),
        HirExpr::IfExp { body, orelse, .. } => {
            rewrite_value_displays(body) | rewrite_value_displays(orelse)
        }
        _ => false,
    }
}

/// Whether [`rewrite_value_displays`] would rewrite anything in `value`.
/// The fast-path triggers use it to decide whether the pass has work to do.
pub(super) fn has_value_display(value: &HirExpr) -> bool {
    match value {
        HirExpr::ListLiteral(_) => true,
        HirExpr::BoolOp {
            left,
            right,
            truth_only: false,
            ..
        } => has_value_display(left) || has_value_display(right),
        HirExpr::IfExp { body, orelse, .. } => has_value_display(body) || has_value_display(orelse),
        _ => false,
    }
}

/// Whether `stmt` is an annotated assignment of a value holding a list
/// display to an object slot -- the one shape [`rewrite_object_slot`]
/// rewrites that the parent's empty-literal fast path does not already see,
/// because the display may be non-empty or an operand.
pub(super) fn is_annotated_object_display(stmt: &HirStmt) -> bool {
    matches!(
        stmt,
        HirStmt::AnnAssign {
            annotation: Ty::Object,
            value: Some(value),
            ..
        } if has_value_display(value)
    )
}

#[cfg(test)]
#[path = "object_slot_tests.rs"]
mod tests;
