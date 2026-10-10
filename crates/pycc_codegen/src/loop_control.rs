//! `break`, `continue` and a loop's `else:` clause.
//!
//! Every loop statement registers its two jump targets with
//! [`LoopControlState::enter`] before it emits its body and drops them with
//! [`LoopControlState::exit`] afterwards: the *break* target is the loop's
//! own normal-exit block (so whatever that block releases -- a `range`
//! loop's induction words, an object loop's iterator -- is released on a
//! `break` too), and the *continue* target is the block that advances to
//! the next iteration (the test block, or the latch that increments the
//! loop's private counter before it).
//!
//! A loop with an `else:` clause (`MirStmt::LoopElse`) whose body can
//! `break` gets a one-byte flag: it is cleared before the loop, set by every
//! `break` bound to that loop, and read after the loop's exit block to skip
//! the clause. The flag is handed to the wrapped loop through
//! [`LoopControlState::set_pending_else_flag`], keyed by that loop
//! statement: only [`LoopControlState::enter`] for the same statement
//! consumes it, and [`LoopControlState::take_unclaimed_else_flag`] proves it
//! was consumed once the loop is emitted.
//!
//! `pycc_hir` refuses a `break`/`continue` that would leave a `try` with a
//! `finally` clause, so a jump never has a pending `finally` between it and
//! its loop; [`emit_jump`] re-checks that against the `finally` stack depth
//! the loop recorded rather than emitting a jump that skips one.

use crate::{
    FinallyTarget, RtFns, StorageSlot, UserFunction, emit_body, emit_body_then_branch, emit_stmt,
};
use inkwell::IntPredicate;
use inkwell::basic_block::BasicBlock;
use inkwell::context::Context;
use inkwell::values::PointerValue;
use pycc_mir::MirStmt;
use std::cell::RefCell;
use std::collections::HashMap;

/// One enclosing loop's jump targets.
#[derive(Clone, Copy)]
struct LoopTarget<'ctx> {
    break_bb: BasicBlock<'ctx>,
    continue_bb: BasicBlock<'ctx>,
    /// The `else:` clause's "a `break` ran" flag, when the loop has one.
    broke_flag: Option<PointerValue<'ctx>>,
    /// `finally_stack.len()` when the loop was entered.
    finally_depth: usize,
}

/// The loop-target stack of the function being emitted.
#[derive(Default)]
pub(crate) struct LoopControlState<'ctx> {
    targets: RefCell<Vec<LoopTarget<'ctx>>>,
    /// The `else:` flag for the loop statement it is keyed by (compared by
    /// address), waiting for that loop's [`Self::enter`].
    pending_else_flag: RefCell<Option<(*const MirStmt, PointerValue<'ctx>)>>,
}

/// Which jump a `break`/`continue` statement makes.
#[derive(Clone, Copy)]
pub(crate) enum LoopJump {
    Break,
    Continue,
}

impl<'ctx> LoopControlState<'ctx> {
    /// Registers the loop statement `stmt`, whose `break` leaves through
    /// `break_bb` and whose `continue` resumes at `continue_bb`. It takes the
    /// pending `else:` flag only when that flag was set for `stmt` itself.
    pub(crate) fn enter(
        &self,
        stmt: &MirStmt,
        break_bb: BasicBlock<'ctx>,
        continue_bb: BasicBlock<'ctx>,
        finally_depth: usize,
    ) {
        let broke_flag = {
            let mut pending = self.pending_else_flag.borrow_mut();
            match *pending {
                Some((owner, flag)) if std::ptr::eq(owner, stmt) => {
                    *pending = None;
                    Some(flag)
                }
                _ => None,
            }
        };
        self.targets.borrow_mut().push(LoopTarget {
            break_bb,
            continue_bb,
            broke_flag,
            finally_depth,
        });
    }

    /// Drops the innermost loop registered by [`Self::enter`].
    pub(crate) fn exit(&self) {
        self.targets
            .borrow_mut()
            .pop()
            .expect("every loop exit is paired with its own enter");
    }

    /// Hands `flag` to the [`Self::enter`] of the loop statement `stmt`.
    pub(crate) fn set_pending_else_flag(&self, stmt: &MirStmt, flag: PointerValue<'ctx>) {
        let previous = self
            .pending_else_flag
            .borrow_mut()
            .replace((stmt as *const MirStmt, flag));
        assert!(
            previous.is_none(),
            "pycc_codegen: internal error: a loop `else:` flag was never claimed by its loop"
        );
    }

    /// Clears the pending `else:` flag and returns it when its loop never
    /// claimed it; `None` once the loop's [`Self::enter`] has taken it.
    pub(crate) fn take_unclaimed_else_flag(&self) -> Option<PointerValue<'ctx>> {
        self.pending_else_flag
            .borrow_mut()
            .take()
            .map(|(_, flag)| flag)
    }
}

/// Emits a `break` or `continue` bound to the innermost enclosing loop,
/// terminating the current block.
pub(crate) fn emit_jump<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    loops: &LoopControlState<'ctx>,
    finally_stack: &[FinallyTarget<'ctx>],
    jump: LoopJump,
) -> Result<(), String> {
    let target = *loops.targets.borrow().last().ok_or_else(|| {
        "pycc_codegen: internal error: a `break`/`continue` outside every loop reached codegen"
            .to_string()
    })?;
    if finally_stack.len() != target.finally_depth {
        return Err(
            "pycc_codegen: internal error: a `break`/`continue` across a `finally` clause \
             reached codegen -- pycc_hir refuses it with C0001"
                .to_string(),
        );
    }
    let dest = match jump {
        LoopJump::Break => {
            if let Some(flag) = target.broke_flag {
                builder
                    .build_store(flag, context.i8_type().const_int(1, false))
                    .expect("build_store should not fail for a loop-else flag");
            }
            target.break_bb
        }
        LoopJump::Continue => target.continue_bb,
    };
    builder
        .build_unconditional_branch(dest)
        .expect("build_unconditional_branch should not fail for a loop jump");
    Ok(())
}

/// Finishes a loop latch: when nothing branches to `latch` (the body never
/// falls through and holds no `continue`) the block is deleted and `false`
/// returned, so the caller emits no dead increment; otherwise the builder is
/// positioned at its end and `true` returned.
pub(crate) fn position_at_latch_if_used<'ctx>(
    builder: &inkwell::builder::Builder<'ctx>,
    latch: BasicBlock<'ctx>,
) -> bool {
    if latch.get_first_use().is_none() {
        // SAFETY: nothing references `latch` (no branch targets it, checked
        // just above, and it holds no instructions), so deleting it cannot
        // leave a dangling use.
        unsafe { latch.delete() }.expect("an unreferenced empty block can be deleted");
        return false;
    }
    builder.position_at_end(latch);
    true
}

/// Emits a loop with an `else:` clause. The wrapped loop runs first; its
/// normal exit (and, through its exit block, every `break`) continues here.
/// Without a `break` bound to the loop the clause simply follows; with one,
/// the flag every such `break` sets skips it.
#[allow(clippy::too_many_arguments)]
pub(crate) fn emit_loop_else<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    module: &inkwell::module::Module<'ctx>,
    rt: &RtFns<'ctx>,
    user_functions: &HashMap<&str, UserFunction<'ctx>>,
    locals: &mut HashMap<String, StorageSlot<'ctx>>,
    loop_stmt: &MirStmt,
    orelse: &[MirStmt],
    expected_return_ty: pycc_mir::Ty,
    finally_stack: &mut Vec<FinallyTarget<'ctx>>,
) -> Result<(), String> {
    let breaks =
        crate::fallthrough::loop_body(loop_stmt).is_some_and(crate::fallthrough::body_breaks_loop);
    let flag = breaks.then(|| {
        // In the entry block, so a loop `else:` inside another loop does
        // not grow the stack on every iteration.
        let flag = crate::alloca_at_entry(builder, context.i8_type(), "loop_broke");
        builder
            .build_store(flag, context.i8_type().const_zero())
            .expect("build_store should not fail for a loop-else flag");
        rt.loops.set_pending_else_flag(loop_stmt, flag);
        flag
    });
    let emitted = emit_stmt(
        context,
        builder,
        module,
        rt,
        user_functions,
        locals,
        loop_stmt,
        expected_return_ty.clone(),
        finally_stack,
    );
    let unclaimed = rt.loops.take_unclaimed_else_flag();
    emitted?;
    assert!(
        unclaimed.is_none(),
        "pycc_codegen: internal error: the loop wrapped by an `else:` clause never claimed its flag"
    );
    let Some(flag) = flag else {
        return emit_body(
            context,
            builder,
            module,
            rt,
            user_functions,
            locals,
            orelse,
            expected_return_ty,
            finally_stack,
        );
    };
    let function = builder.get_insert_block().unwrap().get_parent().unwrap();
    let else_bb = context.append_basic_block(function, "loop_else");
    let merge_bb = context.append_basic_block(function, "loop_else_after");
    let broke = builder
        .build_load(context.i8_type(), flag, "loop_broke_value")
        .expect("build_load should not fail for a loop-else flag")
        .into_int_value();
    let broke = builder
        .build_int_compare(
            IntPredicate::NE,
            broke,
            context.i8_type().const_zero(),
            "loop_broke_set",
        )
        .expect("build_int_compare should not fail comparing two i8 operands");
    builder
        .build_conditional_branch(broke, merge_bb, else_bb)
        .expect("build_conditional_branch should not fail for a well-formed i1 condition");
    builder.position_at_end(else_bb);
    emit_body_then_branch(
        context,
        builder,
        module,
        rt,
        user_functions,
        locals,
        orelse,
        merge_bb,
        expected_return_ty,
        finally_stack,
    )?;
    builder.position_at_end(merge_bb);
    Ok(())
}
