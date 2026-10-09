//! The conditional expression `body if test else orelse` (#1395): a branch
//! on the condition's truth and a phi join.
//!
//! 1. emit `test`, test its truth once, and release its `int` temporary (as
//!    the `MirExpr::Not` arm and a truth-only `and`/`or` operand do). A
//!    `str` or owned `Optional[int]` condition temporary is only tested, not
//!    released: the same accepted leak-only behaviour `boolop.rs` records,
//!    since `release_scalar_if_int_temporary` handles only a bare `int`;
//! 2. branch to `ifexp_body` or `ifexp_orelse`; exactly one runs;
//! 3. each arm emits its branch and converts it to an owned value of the
//!    node's type with [`owned_value`] (a borrowed `int`/`str` read is
//!    retained or incref'd before the conversion, and a bare `int`/`float`/
//!    `bool` or `None` is wrapped into an `Optional` result);
//! 4. both arms branch to `ifexp_join`, whose phi selects the arm's value.
//!
//! The result is therefore always owned, which is why
//! `int_value_is_a_duplicate_reference` and
//! `str_value_is_a_duplicate_reference` classify `MirExpr::IfExp` as
//! owning. Each phi incoming block is read with `current_block()`
//! immediately before that arm's `br join`: a branch's own exception guard
//! and a bigint retain each append blocks of their own.
//!
//! Neither arm holds a word across the other, and the condition's `int`
//! temporary is released before either branch runs, so the node pushes
//! nothing onto `pending_int_releases`.
//!
//! A produced CPython-object condition (`a if o.ready() else b`) is held
//! across its truth test, which can raise, and released before either
//! branch runs (Part 3 of #1092). An `object` arm passes through
//! [`owned_value`] unchanged, so the node's result is owned only when both
//! arms are produced objects; `object_release::is_produced` lists it as a
//! producer exactly then. A result that mixes a produced and a borrowed
//! arm is still leaked on the produced arm (#1499).

use super::boolop::{Emitter, basic_value, owned_value};
use super::{Scalar, release_scalar_if_int_temporary, ty_to_basic_type};
use inkwell::values::BasicValueEnum;
use pycc_mir::{MirExpr, Ty};

/// Emits one `MirExpr::IfExp`.
pub(super) fn emit_if_exp<'ctx>(
    emitter: &Emitter<'_, 'ctx>,
    test: &MirExpr,
    body: &MirExpr,
    orelse: &MirExpr,
    ty: &Ty,
) -> Scalar<'ctx> {
    let test_scalar = emitter.emit(test);
    let held = emitter.hold(test, &test_scalar);
    let truth = emitter.truth(test_scalar);
    held.release(emitter.builder, emitter.rt);
    release_scalar_if_int_temporary(
        emitter.context,
        emitter.builder,
        emitter.rt,
        test,
        &test_scalar,
    );
    let body_block = emitter.new_block("ifexp_body");
    let orelse_block = emitter.new_block("ifexp_orelse");
    let join = emitter.new_block("ifexp_join");
    emitter
        .builder
        .build_conditional_branch(truth, body_block, orelse_block)
        .expect("build_conditional_branch should not fail for a well-formed i1");

    let mut incoming = Vec::with_capacity(2);
    for (block, branch) in [(body_block, body), (orelse_block, orelse)] {
        emitter.builder.position_at_end(block);
        let scalar = emitter.emit(branch);
        let value = basic_value(owned_value(emitter, branch, scalar, ty));
        incoming.push((value, emitter.current_block()));
        emitter.branch_to(join);
    }

    emitter.builder.position_at_end(join);
    let phi = emitter
        .builder
        .build_phi(ty_to_basic_type(emitter.context, ty.clone()), "ifexp_value")
        .expect("build_phi should not fail for a well-formed value type");
    for (value, block) in &incoming {
        phi.add_incoming(&[(value, *block)]);
    }
    scalar_of(ty, phi.as_basic_value())
}

/// The joined value as the `Scalar` for the node's type, one of the shapes
/// `pycc_hir::if_exp_result_ty` yields.
fn scalar_of<'ctx>(ty: &Ty, value: BasicValueEnum<'ctx>) -> Scalar<'ctx> {
    match ty {
        Ty::Int => Scalar::Int(value.into_int_value()),
        Ty::Bool => Scalar::Bool(value.into_int_value()),
        Ty::Float => Scalar::Float(value.into_float_value()),
        Ty::Str => Scalar::Str(value.into_pointer_value()),
        Ty::Optional(_) => Scalar::Optional(value.into_struct_value()),
        Ty::Tuple(_) => Scalar::Tuple(value.into_struct_value()),
        Ty::List(_) => Scalar::List(value.into_pointer_value()),
        Ty::Dict(_) => Scalar::Dict(value.into_pointer_value()),
        Ty::Set(_) | Ty::FrozenSet(_) => Scalar::Set(value.into_pointer_value()),
        Ty::Instance(_) => Scalar::Instance(value.into_pointer_value()),
        Ty::Object => Scalar::Object(value.into_pointer_value()),
        other => panic!(
            "pycc_codegen: internal error: a conditional expression typed `{}`, which \
             pycc_hir::if_exp_result_ty never yields",
            other.name()
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use inkwell::AddressSpace;
    use inkwell::context::Context;

    /// `scalar_of` covers every type the join yields, including the
    /// `object` result only `--ext` programs reach.
    #[test]
    fn scalar_of_wraps_every_joinable_type() {
        let context = Context::create();
        let pointer = context.ptr_type(AddressSpace::default()).const_null();
        let pair = context.const_struct(
            &[
                context.i64_type().const_int(1, false).into(),
                context.i8_type().const_int(1, false).into(),
            ],
            false,
        );
        let int = Ty::Int;
        assert!(matches!(
            scalar_of(&int, context.i64_type().const_int(3, false).into()),
            Scalar::Int(_)
        ));
        assert!(matches!(
            scalar_of(&Ty::Bool, context.i8_type().const_int(1, false).into()),
            Scalar::Bool(_)
        ));
        assert!(matches!(
            scalar_of(&Ty::Float, context.f64_type().const_float(0.5).into()),
            Scalar::Float(_)
        ));
        assert!(matches!(
            scalar_of(&Ty::Optional(Box::new(Ty::Int)), pair.into()),
            Scalar::Optional(_)
        ));
        assert!(matches!(
            scalar_of(&Ty::Tuple(Box::new(vec![Ty::Int, Ty::Bool])), pair.into()),
            Scalar::Tuple(_)
        ));
        assert!(matches!(
            scalar_of(&Ty::Str, pointer.into()),
            Scalar::Str(_)
        ));
        assert!(matches!(
            scalar_of(&Ty::List(Box::new(Ty::Int)), pointer.into()),
            Scalar::List(_)
        ));
        assert!(matches!(
            scalar_of(&Ty::Dict(Box::new((Ty::Str, Ty::Int))), pointer.into()),
            Scalar::Dict(_)
        ));
        assert!(matches!(
            scalar_of(&Ty::Set(Box::new(Ty::Int)), pointer.into()),
            Scalar::Set(_)
        ));
        assert!(matches!(
            scalar_of(&Ty::FrozenSet(Box::new(Ty::Int)), pointer.into()),
            Scalar::Set(_)
        ));
        assert!(matches!(
            scalar_of(&Ty::Instance(Box::new("C".to_string())), pointer.into()),
            Scalar::Instance(_)
        ));
        assert!(matches!(
            scalar_of(&Ty::Object, pointer.into()),
            Scalar::Object(_)
        ));
    }

    #[test]
    #[should_panic(expected = "never yields")]
    fn scalar_of_a_type_the_join_never_yields_is_an_internal_error() {
        let context = Context::create();
        scalar_of(&Ty::None, context.i8_type().const_int(0, false).into());
    }
}
