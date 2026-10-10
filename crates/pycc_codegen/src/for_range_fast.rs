//! The smallint version of an innermost `for v in range(start, stop, k)`
//! loop with a literal, non-zero step `k`.
//!
//! The generic `ForRange` loop keeps its induction value as an encoded `int`
//! word, so every iteration tests tags, checks the increment for overflow and
//! retains/releases the loop variable in case it became a bigint. When both
//! bounds are smallints at run time -- tested once, before the loop -- none of
//! that can happen: every value the loop variable takes lies between the two
//! bounds. This version counts with a raw `i64` and hands the body
//! `(i << 1) | 1`, from which LLVM also learns that the variable is a
//! smallint, folding the tag tests of the arithmetic the body does on it.
//! Bigint bounds still take the generic loop, emitted alongside.
//!
//! Emitting the body twice is limited to bodies without a nested loop or a
//! `try` (so code growth is linear, and no statement whose emission registers
//! state is emitted twice), and to loops without an `else:` clause (whose
//! "broke" flag only one emission can claim).

use crate::rt_fns::RtFns;
use crate::{FinallyTarget, Scalar, StorageSlot, UserFunction};
use inkwell::IntPredicate;
use inkwell::basic_block::BasicBlock;
use inkwell::builder::Builder;
use inkwell::context::Context;
use inkwell::values::IntValue;
use pycc_mir::{MirExpr, MirStmt};
use std::collections::HashMap;

/// The largest literal step this version accepts: `i + k` then cannot leave
/// `i64` for any `i` within the smallint range.
const MAX_STEP: i64 = 1 << 40;

/// The step `k` when `stmt` (a `ForRange` with step `step` and body `body`)
/// may take the smallint version.
pub(super) fn versionable_step(rt: &RtFns<'_>, stmt: &MirStmt, step: &MirExpr, body: &[MirStmt]) -> Option<i64> {
    let MirExpr::IntLiteral(k) = step else {
        return None;
    };
    let k = i64::try_from(*k).ok()?;
    (k != 0 && k.abs() <= MAX_STEP && body.iter().all(is_flat) && !rt.loops.has_pending_else_flag(stmt))
        .then_some(k)
}

/// Whether `stmt` contains no loop and no `try` (see the module doc).
fn is_flat(stmt: &MirStmt) -> bool {
    match stmt {
        MirStmt::If { body, orelse, .. } => body.iter().chain(orelse).all(is_flat),
        MirStmt::Seq(stmts) => stmts.iter().all(is_flat),
        MirStmt::ExprStmt(_)
        | MirStmt::Assign { .. }
        | MirStmt::NoOp
        | MirStmt::DictSet { .. }
        | MirStmt::BufferSet { .. }
        | MirStmt::ListSet { .. }
        | MirStmt::AttrSet { .. }
        | MirStmt::Return(_)
        | MirStmt::Break
        | MirStmt::Continue => true,
        _ => false,
    }
}

/// `i1`: both encoded bounds are smallints, so the smallint version runs.
pub(super) fn bounds_are_small<'ctx>(
    context: &'ctx Context,
    builder: &Builder<'ctx>,
    start: IntValue<'ctx>,
    stop: IntValue<'ctx>,
) -> IntValue<'ctx> {
    let start_small = crate::int_fast::is_small(context, builder, start);
    let stop_small = crate::int_fast::is_small(context, builder, stop);
    builder
        .build_and(start_small, stop_small, "range_bounds_small")
        .expect("and of two i1")
}

/// Emits the smallint loop starting at the current (empty) block and leaving
/// through `exit_bb`, where `break` also goes. The bounds are the normalized
/// encoded words the generic loop uses; the caller has checked them with
/// [`bounds_are_small`].
#[allow(clippy::too_many_arguments)]
pub(super) fn emit<'ctx>(
    context: &'ctx Context,
    builder: &Builder<'ctx>,
    module: &inkwell::module::Module<'ctx>,
    rt: &RtFns<'ctx>,
    user_functions: &HashMap<&str, UserFunction<'ctx>>,
    locals: &mut HashMap<String, StorageSlot<'ctx>>,
    stmt: &MirStmt,
    var: &str,
    start: IntValue<'ctx>,
    stop: IntValue<'ctx>,
    step: i64,
    body: &[MirStmt],
    exit_bb: BasicBlock<'ctx>,
    expected_return_ty: pycc_mir::Ty,
    finally_stack: &mut Vec<FinallyTarget<'ctx>>,
) -> Result<(), String> {
    let i64_type = context.i64_type();
    let function = builder
        .get_insert_block()
        .and_then(|block| block.get_parent())
        .expect("a loop is emitted inside a function body");
    let one = i64_type.const_int(1, false);
    let start_raw = builder
        .build_right_shift(start, one, true, "range_start_raw")
        .expect("ashr");
    let stop_raw = builder
        .build_right_shift(stop, one, true, "range_stop_raw")
        .expect("ashr");
    let preheader = builder.get_insert_block().expect("positioned");
    let test_bb = context.append_basic_block(function, "small_range_test");
    let body_bb = context.append_basic_block(function, "small_range_body");
    let latch_bb = context.append_basic_block(function, "small_range_latch");
    builder.build_unconditional_branch(test_bb).expect("branch");

    builder.position_at_end(test_bb);
    let induction = builder
        .build_phi(i64_type, "small_range_i")
        .expect("build_phi in a fresh block");
    induction.add_incoming(&[(&start_raw, preheader)]);
    let i = induction.as_basic_value().into_int_value();
    let predicate = if step > 0 {
        IntPredicate::SLT
    } else {
        IntPredicate::SGT
    };
    let more = builder
        .build_int_compare(predicate, i, stop_raw, "small_range_more")
        .expect("icmp");
    builder
        .build_conditional_branch(more, body_bb, exit_bb)
        .expect("branch");

    builder.position_at_end(body_bb);
    // `i` lies between the two smallint bounds, so the tagged word is a
    // smallint and owns nothing: no retain before the bind.
    let shifted = builder
        .build_left_shift(i, one, "small_range_shl")
        .expect("shl");
    let word = builder
        .build_or(shifted, one, "small_range_word")
        .expect("or");
    crate::emit_assign(context, builder, rt, locals, var, Scalar::Int(word));
    rt.loops.enter(stmt, exit_bb, latch_bb, finally_stack.len());
    crate::emit_body_then_branch(
        context,
        builder,
        module,
        rt,
        user_functions,
        locals,
        body,
        latch_bb,
        expected_return_ty,
        finally_stack,
    )?;
    rt.loops.exit();
    if crate::loop_control::position_at_latch_if_used(builder, latch_bb) {
        // No overflow: `|i| < 2^62` and `|step| <= 2^40`.
        let next = builder
            .build_int_nsw_add(i, i64_type.const_int(step as u64, true), "small_range_next")
            .expect("add");
        let latch_end = builder.get_insert_block().expect("positioned");
        induction.add_incoming(&[(&next, latch_end)]);
        builder.build_unconditional_branch(test_bb).expect("branch");
    }
    Ok(())
}
