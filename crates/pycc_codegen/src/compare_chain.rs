//! Chained comparisons `a < b < c` (#1212, Part 4 of #1018): the short
//! circuit, built from basic blocks and an `i8` phi join.
//!
//! Every operand is emitted at most once, left to right. Link `i` compares
//! operand `i` (already emitted) with operand `i + 1` (emitted by the link);
//! a false link branches straight to the join with `False`, so no later
//! operand is evaluated.
//!
//! Ownership (D-181: every heap-bigint birth reference is retired exactly
//! once):
//!
//! | operand | released |
//! |---|---|
//! | operand 0 | after link 0's comparison |
//! | middle operand `k` | after link `k`'s comparison when link `k - 1` was true; in link `k - 1`'s false exit otherwise |
//! | last operand | after the last link's comparison |
//! | operand `k` while operand `k + 1` is emitted | on `pending_int_releases`, so a raising operand `k + 1` still releases it |
//! | operand `k + 1` across link `k`'s own guard | on `pending_int_releases`, so the unwind edge releases it; operand `k` was released just before the guard |
//!
//! The link guard: `pycc_rt_int_cmp` and `pycc_rt_int_to_float` raise a
//! pending `OverflowError` for a heap bigint and return a sentinel (`0`,
//! Equal, for `int_cmp`). A single comparison is classified as unable to
//! raise, but inside a chain the sentinel would pick a branch and run the
//! next operand's side effects while the exception is pending. So a link
//! with an `int` operand, and every dataclass `__eq__` call, is followed by
//! [`guard_statement_effects`].
//!
//! Each phi incoming block is read with `get_insert_block()` immediately
//! before that edge's `br join`: releases, guards and the `__eq__` call's
//! function-pointer dispatch all append blocks of their own.

use super::bigint_rc::{pop_pending_int_release, push_pending_int_release_if_scalar_temporary};
use super::boolop::Emitter;
use super::compare::emit_compare_values;
use super::exception::guard_statement_effects;
use super::{
    Scalar, build_call_to_with_leading_args, expect_instance_pointer,
    release_scalar_if_int_temporary,
};
use inkwell::IntPredicate;
use inkwell::basic_block::BasicBlock;
use inkwell::values::IntValue;
use pycc_mir::{MirCompareKind, MirCompareLink, MirExpr, Ty};

/// Emits one `MirExpr::CompareChain` as an `i8` bool.
pub(super) fn emit_compare_chain<'ctx>(
    emitter: &Emitter<'_, 'ctx>,
    first: &MirExpr,
    links: &[MirCompareLink],
) -> Scalar<'ctx> {
    let join = emitter.new_block("cmpchain_join");
    let mut incoming: Vec<(IntValue<'ctx>, BasicBlock<'ctx>)> = Vec::with_capacity(links.len());
    let mut left = first;
    let mut l = emitter.emit(first);
    for (index, link) in links.iter().enumerate() {
        let pending_l = push_pending_int_release_if_scalar_temporary(emitter.rt, left, &l);
        let r = emitter.emit(&link.right);
        pop_pending_int_release(emitter.rt, pending_l);
        let bit = link_bit(emitter, &link.kind, left, l, &link.right, r);
        release(emitter, left, &l);
        if link_can_raise(&link.kind, left, &link.right) {
            let pending_r =
                push_pending_int_release_if_scalar_temporary(emitter.rt, &link.right, &r);
            guard_statement_effects(emitter.context, emitter.builder, emitter.rt);
            pop_pending_int_release(emitter.rt, pending_r);
        }
        if index + 1 == links.len() {
            release(emitter, &link.right, &r);
            incoming.push((bit, emitter.current_block()));
            emitter.branch_to(join);
        } else {
            let next = emitter.new_block("cmpchain_next");
            let false_exit = emitter.new_block("cmpchain_false");
            let holds = emitter
                .builder
                .build_int_compare(
                    IntPredicate::NE,
                    bit,
                    emitter.context.i8_type().const_zero(),
                    "cmpchain_holds",
                )
                .expect("build_int_compare should not fail for an i8 bool");
            emitter
                .builder
                .build_conditional_branch(holds, next, false_exit)
                .expect("build_conditional_branch should not fail for a well-formed i1");
            emitter.builder.position_at_end(false_exit);
            release(emitter, &link.right, &r);
            incoming.push((
                emitter.context.i8_type().const_zero(),
                emitter.current_block(),
            ));
            emitter.branch_to(join);
            emitter.builder.position_at_end(next);
            left = &link.right;
            l = r;
        }
    }
    emitter.builder.position_at_end(join);
    let phi = emitter
        .builder
        .build_phi(emitter.context.i8_type(), "cmpchain_result")
        .expect("build_phi should not fail for an i8");
    for (value, block) in &incoming {
        phi.add_incoming(&[(value, *block)]);
    }
    Scalar::Bool(phi.as_basic_value().into_int_value())
}

/// One link's `i8` bool over its two already-emitted operands.
fn link_bit<'ctx>(
    emitter: &Emitter<'_, 'ctx>,
    kind: &MirCompareKind,
    left: &MirExpr,
    l: Scalar<'ctx>,
    right: &MirExpr,
    r: Scalar<'ctx>,
) -> IntValue<'ctx> {
    match kind {
        MirCompareKind::Plain(op) => emit_compare_values(
            emitter.context,
            emitter.builder,
            emitter.rt,
            *op,
            left,
            l,
            right,
            r,
        ),
        MirCompareKind::DataclassEq { callee, negate } => {
            // The synthesized `__eq__` is registered as an ordinary user
            // function, and the checker admits an instance operand only in
            // a same-dataclass `==`/`!=` link, so both lookups below hold
            // for any MIR `pycc_mir` builds.
            let function = emitter
                .user_functions
                .get(callee.as_str())
                .expect("pycc_codegen: internal error: a dataclass `__eq__` is a user function");
            let operand =
                |scalar| expect_instance_pointer(scalar, "a dataclass `==` operand").into();
            let call_site = build_call_to_with_leading_args(
                emitter.context,
                emitter.builder,
                emitter.module,
                emitter.rt,
                emitter.user_functions,
                emitter.locals,
                function,
                callee,
                &[operand(l), operand(r)],
                &[],
            );
            let equal = call_site
                .try_as_basic_value()
                .expect_basic("a dataclass `__eq__` returns bool")
                .into_int_value();
            if *negate {
                emitter
                    .builder
                    .build_xor(
                        equal,
                        emitter.context.i8_type().const_int(1, false),
                        "cmpchain_ne",
                    )
                    .expect("build_xor should not fail for an i8 bool")
            } else {
                equal
            }
        }
    }
}

/// Whether a link can leave a Python exception pending: a primitive
/// comparison with an `int` operand (a heap bigint raises `OverflowError`
/// in `pycc_rt_int_cmp`/`pycc_rt_int_to_float`), or any dataclass
/// `__eq__` call, which is guarded like every `MirExpr::Call`.
fn link_can_raise(kind: &MirCompareKind, left: &MirExpr, right: &MirExpr) -> bool {
    match kind {
        MirCompareKind::Plain(_) => left.ty() == Ty::Int || right.ty() == Ty::Int,
        MirCompareKind::DataclassEq { .. } => true,
    }
}

fn release<'ctx>(emitter: &Emitter<'_, 'ctx>, source: &MirExpr, scalar: &Scalar<'ctx>) {
    release_scalar_if_int_temporary(emitter.context, emitter.builder, emitter.rt, source, scalar);
}
