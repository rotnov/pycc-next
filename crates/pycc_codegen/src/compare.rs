//! The predicate of one comparison `left op right` over two
//! already-evaluated operands, shared by the single comparison
//! (`MirExpr::Compare`) and each link of a chained comparison
//! (`MirExpr::CompareChain`, #1212).
//!
//! [`emit_compare_values`] performs no releases: each caller owns its
//! operands' int temporaries and releases them after the comparison.

use super::{RtFns, Scalar, to_float, to_numeric_encoded_int};
use inkwell::context::Context;
use inkwell::values::IntValue;
use inkwell::{FloatPredicate, IntPredicate};
use pycc_mir::{MirExpr, Ty};

/// Emits `l op r`, where `l` and `r` are the values of `left` and `right`,
/// and returns the result as an `i8` bool. The `is`/`is not` branch keys on
/// whether `left` is syntactically the `None` literal, so it needs the
/// operand expressions, not only their values.
#[allow(clippy::too_many_arguments)]
pub(super) fn emit_compare_values<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    rt: &RtFns<'ctx>,
    op: pycc_mir::CmpOpKind,
    left: &MirExpr,
    l: Scalar<'ctx>,
    right: &MirExpr,
    r: Scalar<'ctx>,
) -> IntValue<'ctx> {
    let left_ty = left.ty();
    let right_ty = right.ty();
    // `is`/`is not` (D-197, #763, Part 1 of #747). HIR lowering
    // (`pycc_hir::compare_chain::lower_cmp_op`)
    // guarantees one operand is syntactically `Expr::NoneLiteral`
    // whenever `op` is `Is`/`IsNot`, and `pycc_types`' own
    // `Is`/`IsNot` typing arm (`pycc_types::compare_chain::compare_link_ty`)
    // guarantees the *other* operand's type is `Ty::Optional(_)` or
    // `Ty::None` -- never anything else. Handled as its own
    // early-computed branch, before the float/str/numeric branches
    // below (none of which know what to do with a struct-valued
    // `Scalar::Optional`), by testing the *other* operand's
    // present/absent flag directly rather than doing any real
    // comparison: `None`/`Ty::None` is always absent (`is` is
    // always `False`, `is not` always `True`, independent of the
    // operand's own emitted value, so neither `l` nor `r` needs
    // inspecting for that shape).
    // Narrows `op`'s 10 `CmpOpKind` variants down to the 6 ordinary
    // ordering comparators in exactly one place (D-197, #763, Part 1
    // of #747): `Is`/`IsNot` are handled and returned right here,
    // inline, so the three type-dispatched matches below (float/
    // str/int) only ever see `OrderedCmpOp`'s 6 variants and need no
    // `Is`/`IsNot` arm of their own at all. The project's own
    // established convention (see `emit_string_literal`'s doc
    // comment) is to eliminate a provably-dead branch structurally
    // rather than leave an `unreachable!()` arm as a permanently
    // uncovered region under this crate's 100%-region gate (D-014)
    // -- the three-way duplication this replaces (one `unreachable!`
    // per type-dispatched match) was exactly that anti-pattern.
    enum OrderedCmpOp {
        Eq,
        NotEq,
        Lt,
        LtE,
        Gt,
        GtE,
    }
    let op = match op {
        pycc_mir::CmpOpKind::Is | pycc_mir::CmpOpKind::IsNot => {
            let (other_scalar, other_ty) = if matches!(left, MirExpr::NoneLiteral) {
                (r, right_ty)
            } else {
                (l, left_ty)
            };
            // Dispatches on `other_scalar`'s own runtime variant,
            // not on `other_ty`, so there is no separate "statically
            // `Optional[_]` but did not evaluate to `Scalar::
            // Optional`" arm to keep alive: every `Ty::Optional`-
            // typed `MirExpr` this crate can emit -- `Name` (guarded
            // by its own `debug_assert_eq!` on `slot.ty`),
            // `OptionalWrap`, and `Call`'s `Ty::Optional` result
            // extraction -- always produces a matching `Scalar::
            // Optional`, so that combination is unreachable by
            // construction, not merely untested; matching on the
            // scalar directly removes the branch instead of leaving
            // it as a dead, permanently-uncoverable region.
            let present = match other_scalar {
                Scalar::Optional(v) => builder
                    .build_extract_value(v, 1, "opt_present")
                    .expect(
                        "build_extract_value should not fail reading field 1 of a 2-field struct",
                    )
                    .into_int_value(),
                _ if other_ty == Ty::None => context.i8_type().const_zero(),
                _ => panic!(
                    "pycc_codegen: internal error: an `is`/`is not` operand's non-`None` side must be `Optional[_]` -- pycc_types::check (T0021) should have rejected this before codegen"
                ),
            };
            let is_absent = builder
                .build_int_compare(
                    IntPredicate::EQ,
                    present,
                    context.i8_type().const_zero(),
                    "is_none",
                )
                .expect("build_int_compare should not fail comparing two i8 operands");
            let as_bool = if matches!(op, pycc_mir::CmpOpKind::Is) {
                is_absent
            } else {
                builder
                    .build_not(is_absent, "is_not_none")
                    .expect("build_not should not fail negating an i1 value")
            };
            return builder
                .build_int_z_extend(as_bool, context.i8_type(), "bool_from_is")
                .expect("build_int_z_extend should not fail widening i1 to i8");
        }
        pycc_mir::CmpOpKind::Eq => OrderedCmpOp::Eq,
        pycc_mir::CmpOpKind::NotEq => OrderedCmpOp::NotEq,
        pycc_mir::CmpOpKind::Lt => OrderedCmpOp::Lt,
        pycc_mir::CmpOpKind::LtE => OrderedCmpOp::LtE,
        pycc_mir::CmpOpKind::Gt => OrderedCmpOp::Gt,
        pycc_mir::CmpOpKind::GtE => OrderedCmpOp::GtE,
        // Part 2b of #1371: membership is admitted only with a CPython
        // object container, which `pycc_mir` lowers to `ObjContains`.
        pycc_mir::CmpOpKind::In | pycc_mir::CmpOpKind::NotIn => panic!(
            "pycc_codegen: internal error: a native `in`/`not in` reached codegen -- \
             pycc_types admits membership only in a CPython object"
        ),
    };
    if left_ty == Ty::Float || right_ty == Ty::Float {
        let l = to_float(context, builder, rt, l);
        let r = to_float(context, builder, rt, r);
        let predicate = match op {
            OrderedCmpOp::Eq => FloatPredicate::OEQ,
            // `UNE` ("unordered or not equal"), not `ONE` --
            // CPython's `float('nan') != float('nan')` is `True`,
            // and `NaN` involves an *unordered* comparison, not an
            // ordered not-equal one. The other five predicates
            // below correctly stay "ordered" (`O*`): Python's
            // `<`/`<=`/`>`/`>=`/`==` on `float` are all `False`
            // whenever `NaN` is involved, which is exactly what the
            // ordered forms give.
            OrderedCmpOp::NotEq => FloatPredicate::UNE,
            OrderedCmpOp::Lt => FloatPredicate::OLT,
            OrderedCmpOp::LtE => FloatPredicate::OLE,
            OrderedCmpOp::Gt => FloatPredicate::OGT,
            OrderedCmpOp::GtE => FloatPredicate::OGE,
        };
        let cond = builder
            .build_float_compare(predicate, l, r, "fcmp")
            .expect("build_float_compare should not fail for two f64 operands");
        builder
            .build_int_z_extend(cond, context.i8_type(), "bool_from_fcmp")
            .expect("build_int_z_extend should not fail widening i1 to i8")
    } else if left_ty == Ty::Str || right_ty == Ty::Str {
        let Scalar::Str(l) = l else {
            panic!("pycc_codegen: internal error: str Compare operand did not evaluate to str")
        };
        let Scalar::Str(r) = r else {
            panic!("pycc_codegen: internal error: str Compare operand did not evaluate to str")
        };
        let ordering = builder
            .build_call(rt.str_cmp, &[l.into(), r.into()], "str_cmp")
            .expect("build_call should not fail for a well-formed comparison")
            .try_as_basic_value()
            .expect_basic("pycc_rt_str_cmp returns a non-void `i32`")
            .into_int_value();
        let zero = context.i32_type().const_int(0, false);
        let predicate = match op {
            OrderedCmpOp::Eq => IntPredicate::EQ,
            OrderedCmpOp::NotEq => IntPredicate::NE,
            OrderedCmpOp::Lt => IntPredicate::SLT,
            OrderedCmpOp::LtE => IntPredicate::SLE,
            OrderedCmpOp::Gt => IntPredicate::SGT,
            OrderedCmpOp::GtE => IntPredicate::SGE,
        };
        let cond = builder
            .build_int_compare(predicate, ordering, zero, "str_cmp_pred")
            .expect("build_int_compare should not fail for two i32 operands");
        builder
            .build_int_z_extend(cond, context.i8_type(), "bool_from_str_cmp")
            .expect("build_int_z_extend should not fail widening i1 to i8")
    } else {
        let l = to_numeric_encoded_int(context, builder, l);
        let r = to_numeric_encoded_int(context, builder, r);
        let ordering = builder
            .build_call(rt.int_cmp, &[l.into(), r.into()], "int_cmp")
            .expect("build_call should not fail for a well-formed comparison")
            .try_as_basic_value()
            .expect_basic("pycc_rt_int_cmp returns a non-void `i32`")
            .into_int_value();
        let zero = context.i32_type().const_int(0, false);
        let predicate = match op {
            OrderedCmpOp::Eq => IntPredicate::EQ,
            OrderedCmpOp::NotEq => IntPredicate::NE,
            OrderedCmpOp::Lt => IntPredicate::SLT,
            OrderedCmpOp::LtE => IntPredicate::SLE,
            OrderedCmpOp::Gt => IntPredicate::SGT,
            OrderedCmpOp::GtE => IntPredicate::SGE,
        };
        let cond = builder
            .build_int_compare(predicate, ordering, zero, "cmp")
            .expect("build_int_compare should not fail for two i32 operands");
        builder
            .build_int_z_extend(cond, context.i8_type(), "bool_from_cmp")
            .expect("build_int_z_extend should not fail widening i1 to i8")
    }
}
