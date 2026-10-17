//! `==`/`!=` with a `T | None` operand (`x == 3`, `parents[v] != guy`,
//! `a == b` of two optionals): equal when both are `None`, or both hold a
//! value and the values compare equal, as CPython's `None.__eq__` and the
//! value's own `__eq__` decide. The values are compared only when both are
//! present, so a `None` pointer is never read.

use super::{RtFns, Scalar};
use inkwell::context::Context;
use inkwell::values::IntValue;
use pycc_mir::{MirExpr, Ty};

/// One operand split into its presence (`i1`) and its value as the `Scalar`
/// of `inner` (meaningful only when present).
fn split<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    ty: &Ty,
    scalar: Scalar<'ctx>,
) -> (IntValue<'ctx>, Ty, Scalar<'ctx>) {
    if let Some(inner) = crate::scalar::pointer_optional_inner(ty) {
        let ptr = crate::scalar::scalar_pointer(&scalar).expect("a pointer optional is a pointer");
        let present = builder
            .build_is_not_null(ptr, "opt_eq_present")
            .expect("build_is_not_null should not fail for a pointer");
        return (present, inner.clone(), scalar);
    }
    match (ty, scalar) {
        (Ty::Optional(inner), Scalar::Optional(v)) => {
            let flag = builder
                .build_extract_value(v, 1, "opt_eq_flag")
                .expect("build_extract_value should not fail on an Optional's field 1")
                .into_int_value();
            let present = builder
                .build_int_compare(
                    inkwell::IntPredicate::NE,
                    flag,
                    context.i8_type().const_zero(),
                    "opt_eq_present",
                )
                .expect("build_int_compare should not fail for two i8 values");
            let payload = builder
                .build_extract_value(v, 0, "opt_eq_payload")
                .expect("build_extract_value should not fail on an Optional's field 0");
            let value = match inner.as_ref() {
                Ty::Float => Scalar::Float(payload.into_float_value()),
                Ty::Bool => Scalar::Bool(payload.into_int_value()),
                _ => Scalar::Int(payload.into_int_value()),
            };
            (present, (**inner).clone(), value)
        }
        (ty, scalar) => (context.bool_type().const_int(1, false), ty.clone(), scalar),
    }
}

/// `l == r` (or `!=` when `negate`) when either operand is a `T | None`
/// other than the `None` literal; `None` otherwise. The result is an `i8`
/// bool, as `compare::emit_compare_values`'.
#[allow(clippy::too_many_arguments)]
pub(super) fn emit_optional_eq<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    rt: &RtFns<'ctx>,
    negate: bool,
    left: &MirExpr,
    l: Scalar<'ctx>,
    right: &MirExpr,
    r: Scalar<'ctx>,
) -> Option<IntValue<'ctx>> {
    let (left_ty, right_ty) = (left.ty(), right.ty());
    let optional = |ty: &Ty| matches!(ty, Ty::Optional(_));
    if !(optional(&left_ty) || optional(&right_ty))
        || matches!(left, MirExpr::NoneLiteral)
        || matches!(right, MirExpr::NoneLiteral)
    {
        return None;
    }
    let (l_present, l_ty, l_value) = split(context, builder, &left_ty, l);
    let (r_present, r_ty, r_value) = split(context, builder, &right_ty, r);
    let function = builder
        .get_insert_block()
        .and_then(|block| block.get_parent())
        .expect("a comparison is emitted inside a function");
    let entry = builder.get_insert_block().expect("an insertion block");
    let both_bb = context.append_basic_block(function, "opt_eq_both");
    let join_bb = context.append_basic_block(function, "opt_eq_join");
    let both = builder
        .build_and(l_present, r_present, "opt_eq_both_present")
        .expect("build_and should not fail for two i1 values");
    // Unless both are present, they are equal exactly when both are `None`.
    let neither = builder
        .build_int_compare(inkwell::IntPredicate::EQ, l_present, r_present, "opt_eq_same")
        .expect("build_int_compare should not fail for two i1 values");
    builder
        .build_conditional_branch(both, both_bb, join_bb)
        .expect("build_conditional_branch should not fail");
    builder.position_at_end(both_bb);
    let as_expr = |ty: Ty| MirExpr::Name {
        name: String::new(),
        ty,
    };
    let equal = crate::compare::emit_compare_values(
        context,
        builder,
        rt,
        pycc_mir::CmpOpKind::Eq,
        &as_expr(l_ty),
        l_value,
        &as_expr(r_ty),
        r_value,
    );
    let equal = builder
        .build_int_compare(
            inkwell::IntPredicate::NE,
            equal,
            context.i8_type().const_zero(),
            "opt_eq_values",
        )
        .expect("build_int_compare should not fail for two i8 values");
    let both_end = builder.get_insert_block().expect("an insertion block");
    builder
        .build_unconditional_branch(join_bb)
        .expect("build_unconditional_branch should not fail");
    builder.position_at_end(join_bb);
    let phi = builder
        .build_phi(context.bool_type(), "opt_eq")
        .expect("build_phi should not fail in a fresh join block");
    phi.add_incoming(&[(&neither, entry), (&equal, both_end)]);
    let result = phi.as_basic_value().into_int_value();
    let result = if negate {
        builder
            .build_not(result, "opt_ne")
            .expect("build_not should not fail for an i1 value")
    } else {
        result
    };
    Some(
        builder
            .build_int_z_extend(result, context.i8_type(), "opt_eq_i8")
            .expect("build_int_z_extend should not fail widening i1 to i8"),
    )
}
