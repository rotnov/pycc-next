//! The smallint version of a `for v in range(start, stop, k)` loop with a
//! literal, non-zero step `k` whose body holds at most one level of nested
//! loops.
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
//! Emitting the body twice is limited to bodies without a `try` (no
//! statement whose emission registers state is emitted twice), to at most
//! one level of nested `for`/`while` loops whose own bodies are flat (an
//! innermost body is emitted at most four times), and to loops without an
//! `else:` clause (whose "broke" flag only one emission can claim).

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
pub(super) fn versionable_step(
    rt: &RtFns<'_>,
    stmt: &MirStmt,
    step: &MirExpr,
    body: &[MirStmt],
) -> Option<i64> {
    let MirExpr::IntLiteral(k) = step else {
        return None;
    };
    let k = i64::try_from(*k).ok()?;
    (k != 0 && k.abs() <= MAX_STEP && body.iter().all(|stmt| is_flat(stmt, 1)) && !rt.loops.has_pending_else_flag(stmt))
        .then_some(k)
}

/// Whether `stmt` contains no `try` and at most `loops` levels of nested
/// `for ... in range`/`while` loops without `else:` (see the module doc).
fn is_flat(stmt: &MirStmt, loops: usize) -> bool {
    match stmt {
        MirStmt::If { body, orelse, .. } => body.iter().chain(orelse).all(|stmt| is_flat(stmt, loops)),
        MirStmt::Seq(stmts) => stmts.iter().all(|stmt| is_flat(stmt, loops)),
        MirStmt::ForRange { body, .. } | MirStmt::While { body, .. } if loops > 0 => {
            body.iter().all(|stmt| is_flat(stmt, loops - 1))
        }
        MirStmt::ExprStmt(_)
        | MirStmt::Assign { .. }
        | MirStmt::NoOp
        | MirStmt::ReleaseName(_)
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
    let test_bb = context.append_basic_block(function, "small_range_test");
    let body_bb = context.append_basic_block(function, "small_range_body");
    let latch_bb = context.append_basic_block(function, "small_range_latch");
    let predicate = if step > 0 {
        IntPredicate::SLT
    } else {
        IntPredicate::SGT
    };
    // Every value this loop binds is a smallint, which owns nothing. When
    // nothing but the loop itself binds the variable, only the value it held
    // before the loop can need a release: release it once, on entry to a
    // first iteration (a loop that runs zero times leaves it bound), instead
    // of testing the previous value on every iteration.
    let hoisted = hoist_release(rt, locals, var, body);
    if hoisted {
        let enter_bb = context.append_basic_block(function, "small_range_enter");
        let first = builder
            .build_int_compare(predicate, start_raw, stop_raw, "small_range_first")
            .expect("icmp");
        builder
            .build_conditional_branch(first, enter_bb, exit_bb)
            .expect("branch");
        builder.position_at_end(enter_bb);
        crate::bigint_rc::release_int_slot_before_store(context, builder, rt, &locals[var]);
    }
    let preheader = builder.get_insert_block().expect("positioned");
    builder.build_unconditional_branch(test_bb).expect("branch");

    builder.position_at_end(test_bb);
    let induction = builder
        .build_phi(i64_type, "small_range_i")
        .expect("build_phi in a fresh block");
    induction.add_incoming(&[(&start_raw, preheader)]);
    let i = induction.as_basic_value().into_int_value();
    let more = builder
        .build_int_compare(predicate, i, stop_raw, "small_range_more")
        .expect("icmp");
    builder
        .build_conditional_branch(more, body_bb, exit_bb)
        .expect("branch");

    builder.position_at_end(body_bb);
    // `i` lies between the two smallint bounds, so the tagged word is a
    // smallint and owns nothing: no retain before the bind. The doubling
    // is `nsw` (|i| < 2^62), which lets LLVM fold the body's untagging
    // `(2i | 1) >> 1` back to `i` and carry the loop's range facts to it.
    let two = i64_type.const_int(2, false);
    let shifted = builder
        .build_int_nsw_mul(i, two, "small_range_shl")
        .expect("mul");
    let word = builder
        .build_or(shifted, one, "small_range_word")
        .expect("or");
    if hoisted {
        crate::entry_slots::store_unowned_int(context, builder, &locals[var], word);
    } else {
        crate::emit_assign(context, builder, rt, locals, var, Scalar::Int(word));
    }
    rt.loops.enter(
        stmt,
        exit_bb,
        latch_bb,
        finally_stack.len(),
        crate::exception::owned_slots::depth(rt),
    );
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

/// Whether the loop variable's release can move out of the loop: an `int`
/// function local (an `alloca`; a module global could be rebound by any call
/// in the body) that no statement of the body binds.
fn hoist_release(
    rt: &RtFns<'_>,
    locals: &HashMap<String, StorageSlot<'_>>,
    var: &str,
    body: &[MirStmt],
) -> bool {
    let Some(slot) = locals.get(var) else {
        return false;
    };
    let is_alloca = slot
        .ptr
        .as_instruction()
        .is_some_and(|inst| inst.get_opcode() == inkwell::values::InstructionOpcode::Alloca);
    // A borrowed local does not own its value, so releasing it would free
    // memory its owner still uses (same rule as emit_int_self_update).
    if slot.ty != pycc_mir::Ty::Int || !is_alloca || rt.borrowed_locals.borrow().contains(var) {
        return false;
    }
    let mut bound = std::collections::BTreeMap::new();
    for stmt in body {
        crate::collect_stmt_bindings(stmt, &mut bound);
    }
    !bound.contains_key(var)
}
