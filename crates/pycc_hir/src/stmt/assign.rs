//! Lowering of a plain assignment statement (`Stmt::Assign`), extracted
//! verbatim from `crates/pycc_hir/src/stmt.rs`'s `lower_stmt` per AGENTS.md's
//! file-decomposition rule when augmented assignment (#1209) began routing
//! its desugared statement through it. The only edits are the ones the
//! function boundary forces: the `use` lines and the `Ok(..)` around the
//! arm's value.

use crate::expr::keyword_bind::SignatureTable;
use crate::expr::{
    comp_assign_stmt, is_zero_arg_super_call, lower_dict_comp, lower_expr, lower_list_comp,
    lower_set_comp,
};
use crate::int_boundary::check_boundary_literal;
use crate::{HirStmt, ImportBinding, unsupported};
use pycc_ast::{Expr, ExprAttribute, StmtAssign};
use pycc_diag::Diagnostic;

/// Lowers `assign` (`Stmt::Assign`) to `HirStmt::Assign`, a comprehension
/// assignment, `HirStmt::DictSet` or `HirStmt::AttrSet`, by target shape.
/// The walrus-placement check `lower_stmt` runs on every lowered statement
/// still applies to the result. `assign` has exactly one target: a chained
/// assignment (#1213) reaches here only as its single-target pieces.
pub(super) fn lower_assign(
    assign: &StmtAssign,
    in_function: bool,
    class_name: Option<&str>,
    imports: &[ImportBinding],
    signatures: &SignatureTable,
) -> Result<HirStmt, Diagnostic> {
    let [target] = <&[Expr; 1]>::try_from(assign.targets.as_slice())
        .expect("lower_stmt_expanded desugars a multi-target (chained) assignment before lowering");
    Ok(match target {
        Expr::Name(name) => match assign.value.as_ref() {
            // `name = <comp>` keeps its own statement variants (PR-12,
            // D-117): about twenty statement passes dispatch on them. A
            // comprehension in any other position lowers through
            // `lower_expr` to `HirExpr::Comprehension` (#1254, D-250);
            // both forms are built from the same lowered node.
            Expr::ListComp(comp) => comp_assign_stmt(
                name.id.as_str(),
                lower_list_comp(comp, class_name, imports, signatures)?,
            ),
            Expr::SetComp(comp) => comp_assign_stmt(
                name.id.as_str(),
                lower_set_comp(comp, class_name, imports, signatures)?,
            ),
            Expr::DictComp(comp) => comp_assign_stmt(
                name.id.as_str(),
                lower_dict_comp(comp, class_name, imports, signatures)?,
            ),
            _ => HirStmt::Assign {
                target: name.id.as_str().to_string(),
                value: lower_expr(&assign.value, in_function, class_name, imports, signatures)?,
            },
        },
        // `<bare name>[key] = value`, PR-11 Task 3 (D-123): unlike
        // `list[int]`'s own read-only-indexing consequence (D-105),
        // `dict[str, int]` ships `d[k] = v`. This lowering step has
        // no type information (mirroring `ForList`'s own bare-name
        // iterable, which is resolved to `Ty::List`, `Ty::Dict`, or
        // rejected downstream), so a `list[int]` subscript-assignment target
        // also reaches `HirStmt::DictSet` here -- `pycc_types`
        // rejects it with `T0033` once the base's real type is
        // known, relocating (not removing) the invariant
        // `subscript_assignment_to_a_non_bare_name_base_is_unsupported`
        // (`crates/pycc_hir/src/tests.rs`) used to enforce at the
        // lowering level.
        Expr::Subscript(sub) => {
            let Expr::Name(base_name) = sub.value.as_ref() else {
                return Err(unsupported(
                    "only assigning to a bare-name subscript target (`name[key] = value`) is supported so far",
                    pycc_ast::expr_range(target),
                ));
            };
            // Issue #618 (T0051): only the assigned `value` is a
            // checked boundary position here; the `key` slot is
            // deliberately left unchecked for the same reason a
            // dict-literal key is unchecked elsewhere in this module
            // -- this compiler never gives a dict key an `int`-typed,
            // boundary-sensitive representation, so a key literal has
            // no runtime `int`-untagging boundary to protect.
            let key = lower_expr(&sub.slice, in_function, class_name, imports, signatures)?;
            let value = lower_expr(&assign.value, in_function, class_name, imports, signatures)?;
            // This lowering step is type-blind (see the comment
            // above): `base_name` may turn out to be a `list[int]` at
            // `pycc_types` time, not a `dict`, in which case T0033
            // rejects the whole assignment downstream and this label
            // is never surfaced. The label is deliberately
            // base-neutral ("subscript-assign", not "dict
            // subscript-assign") so it does not imply a base type
            // this lowering step hasn't actually confirmed.
            check_boundary_literal(
                &value,
                pycc_ast::expr_range(&assign.value),
                "subscript-assign value",
            )?;
            HirStmt::DictSet {
                dict: base_name.id.as_str().to_string(),
                key,
                value,
            }
        }
        // `base.attr = value` (D-154, Part 1 of #375): structurally
        // recognized for any base expression, exactly like
        // `HirExpr::AttrGet`'s own `base` (no type information is
        // available at this lowering step to narrow it to only
        // `self` or only an instance-typed receiver -- `pycc_types`
        // rejects a non-instance base or an undeclared attribute
        // name). This supersedes the older, narrower invariant that
        // used to reject any non-bare-name `Stmt::Assign` target
        // outright ("only assigning to a bare name is supported so
        // far"). A tuple or list target (`a, b = t`, alone or as one
        // piece of a chain) never reaches this function:
        // `lower_stmt_expanded` routes it to `stmt/unpack.rs` (Part 1 of
        // #891), and no other caller lowers a `Stmt::Assign` here. The
        // `other => ..` catch-all just below stays as a defensive refusal.
        Expr::Attribute(attr) => {
            // #448: `super().attr = value` — super() attribute
            // assignment is not implemented in this version. Without
            // this special case, `super().attr = value` would lower
            // the `super()` base through the generic `lower_expr`
            // path, which rejects a bare `super()` with a confusing
            // "a bare `super()` expression is not supported" message
            // that doesn't name the actual unsupported operation
            // (attribute assignment through super()). Emit a dedicated
            // C0001 diagnostic instead.
            reject_super_attr_base(attr)?;
            HirStmt::AttrSet {
                base: lower_expr(&attr.value, in_function, class_name, imports, signatures)?,
                attr: attr.attr.to_string(),
                value: lower_expr(&assign.value, in_function, class_name, imports, signatures)?,
            }
        }
        other => {
            return Err(unsupported(
                format!(
                    "only assigning to a bare name is supported so far, got {}",
                    pycc_ast::expr_kind_name(other)
                ),
                pycc_ast::expr_range(other),
            ));
        }
    })
}

/// #448: `super().attr = value` -- super() attribute assignment is not
/// implemented. Shared by the plain (`lower_assign`) and the annotated
/// (`ann_assign`, #1264) attribute-target arms so both refuse it with the
/// same dedicated `C0001` rather than `lower_expr`'s bare-`super()` message.
pub(super) fn reject_super_attr_base(attr: &ExprAttribute) -> Result<(), Diagnostic> {
    if is_zero_arg_super_call(&attr.value) {
        return Err(unsupported(
            "super().attr = value is not supported yet — super() attribute \
             assignment is not implemented in this version",
            pycc_ast::expr_range(&attr.value),
        ));
    }
    Ok(())
}
