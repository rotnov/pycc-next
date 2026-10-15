//! Statement lowering: `emit_stmt`, its per-node dispatcher, and statement bodies.

use super::*;

// Large dispatcher arms live in per-topic modules (mechanical extraction:
// each helper re-destructures its own node and runs the arm body verbatim).
mod comp_assign;
mod iter_loops;
mod raise_stmt;
mod range_loop;
mod return_stmt;
mod store_stmt;

/// Emits every statement in `body` in order, stopping early the moment the
/// current block already ends in a terminator.
///
/// **History (Tasks 3/4 removed this exact check as unreachable, Task 5
/// re-adds it as their own doc comments predicted it eventually would
/// need to):** Task 3 (only `ExprStmt`/`Assign` existed) and Task 4 (which
/// added `If`/`While`/`ForRange`, every arm of which always finishes by
/// repositioning the builder at a fresh, never-yet-terminated continuation
/// block before returning) both proved this check was unreachable in their
/// own scope, confirmed empirically by `cargo llvm-cov`, and removed it
/// rather than carry dead code -- while flagging that a future `Return`
/// arm would be the first `MirStmt` shape whose codegen terminates the
/// *current* block without repositioning the builder afterward (nothing is
/// left to emit into). Task 5 adds exactly that `Return` arm, so a body
/// with a `Return` followed by further (legal, if dead, Python) statements
/// now really can leave this loop's *next* iteration trying to emit into
/// an already-terminated block -- without this check, that would build a
/// second terminator onto the same block, invalid LLVM IR that
/// `module.verify()` (correctly) rejects. `emit_body_then_branch` below
/// and `ForRange`'s own inline copy in `emit_stmt` need the exact same
/// reasoning applied to their own trailing branch, and both re-add their
/// own guard for the same reason (see each one's own doc comment).
#[allow(clippy::too_many_arguments)]
pub(crate) fn emit_body<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    module: &inkwell::module::Module<'ctx>,
    rt: &RtFns<'ctx>,
    user_functions: &HashMap<&str, UserFunction<'ctx>>,
    locals: &mut HashMap<String, StorageSlot<'ctx>>,
    body: &[MirStmt],
    expected_return_ty: pycc_mir::Ty,
    finally_stack: &mut Vec<FinallyTarget<'ctx>>,
) -> Result<(), String> {
    for stmt in body {
        emit_stmt(
            context,
            builder,
            module,
            rt,
            user_functions,
            locals,
            stmt,
            expected_return_ty.clone(),
            finally_stack,
        )?;
        if erase_unreachable_if_present(builder) {
            let exception_target = rt
                .exceptions
                .targets
                .borrow()
                .last()
                .copied()
                .expect("emit_body is always called inside an exception target");
            builder
                .build_unconditional_branch(exception_target)
                .expect("build_unconditional_branch should route an explicit raise");
            break;
        }
        if builder
            .get_insert_block()
            .unwrap()
            .get_terminator()
            .is_some()
        {
            // A return already terminated this suite. Raising expressions
            // and complete try statements route themselves; explicit raise
            // uses the `unreachable` path handled directly above.
            break;
        }
    }
    Ok(())
}

/// Emits `body` (via `emit_body`, which may now leave the current block
/// already terminated -- see its own doc comment), then an unconditional
/// branch to `dest` *unless* the block is already terminated (a `Return`
/// reached inside `body`, in which case there is nothing left to branch
/// from and doing so anyway would build an invalid second terminator).
/// Used by `If`'s `then`/`else` arms and `While`'s body; `ForRange`'s body
/// needs its own variant inline (it has extra post-body work --
/// incrementing the loop variable -- to do before branching back to the
/// loop test), so does not reuse this helper, but re-adds the identical
/// guard around its own trailing branch for the same reason.
#[allow(clippy::too_many_arguments)]
pub(crate) fn emit_body_then_branch<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    module: &inkwell::module::Module<'ctx>,
    rt: &RtFns<'ctx>,
    user_functions: &HashMap<&str, UserFunction<'ctx>>,
    locals: &mut HashMap<String, StorageSlot<'ctx>>,
    body: &[MirStmt],
    dest: inkwell::basic_block::BasicBlock<'ctx>,
    expected_return_ty: pycc_mir::Ty,
    finally_stack: &mut Vec<FinallyTarget<'ctx>>,
) -> Result<bool, String> {
    emit_body(
        context,
        builder,
        module,
        rt,
        user_functions,
        locals,
        body,
        expected_return_ty,
        finally_stack,
    )?;
    let falls_through = builder
        .get_insert_block()
        .unwrap()
        .get_terminator()
        .is_none();
    if falls_through {
        builder
            .build_unconditional_branch(dest)
            .expect("build_unconditional_branch should not fail on a block with no terminator yet");
    }
    Ok(falls_through)
}

/// Handles every `MirStmt` shape (this match is exhaustive over
/// `MirStmt`, no catch-all arm): a `print()` call of any number of
/// `int`/`float`/`bool`/`str` arguments plus any supported materializable
/// non-`print()` `None` expression, including direct user-function,
/// `ListAppend`, and `SetAdd` results, a D-075 parameter, or ordinary
/// assignment storage (space-separated, one trailing newline, matching
/// CPython's `print(*args)` -- all arguments are now evaluated
/// left-to-right before any output is emitted, matching CPython's own
/// argument-evaluation semantics (fixes #145; see
/// `emit_eval_print_arg`/`emit_write_print_arg`); D-072 still excludes
/// using `print()` itself as a nested expression), any
/// other bare expression statement (a user-function call with any number of
/// arguments included -- see `emit_expr`'s `Call` arm, which this now
/// delegates to uniformly instead of special-casing zero-arg calls here), a
/// local-variable assignment, `If`/`While`/
/// `ForRange` control flow (Task 4) -- real basic blocks, conditional
/// branches, and loop back-edges, using `truthy` for the shared `if`/
/// `while` truthiness check and `emit_body_then_branch`/an inline
/// equivalent for the terminator-safety this introduces (see both
/// helpers' own doc comments) -- `Return` (Task 5), terminating
/// the current block with the evaluated value (or none, for a bare
/// `return`) -- and `ForList` (v0.2, D-105/Task 11b), reusing `ForRange`'s
/// own loop/branch-building infrastructure parametrized over a runtime
/// `pycc_rt_list_len` call instead of a static bound. `ForList` is a
/// v0.2 addition, not part of v0.1's own original shape set.
#[allow(clippy::too_many_arguments)]
pub(crate) fn emit_stmt<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    module: &inkwell::module::Module<'ctx>,
    rt: &RtFns<'ctx>,
    user_functions: &HashMap<&str, UserFunction<'ctx>>,
    locals: &mut HashMap<String, StorageSlot<'ctx>>,
    stmt: &MirStmt,
    expected_return_ty: pycc_mir::Ty,
    finally_stack: &mut Vec<FinallyTarget<'ctx>>,
) -> Result<(), String> {
    match stmt {
        MirStmt::Yield(value) => {
            crate::generator::emit_yield(
                context,
                builder,
                module,
                rt,
                user_functions,
                locals,
                value,
                false,
            );
            Ok(())
        }
        MirStmt::GenReturn(value) => {
            crate::generator::emit_gen_return(
                context,
                builder,
                module,
                rt,
                user_functions,
                locals,
                value,
            );
            Ok(())
        }
        MirStmt::ForGen { .. } => crate::generator::emit_for_gen_stmt(
            context,
            builder,
            module,
            rt,
            user_functions,
            locals,
            stmt,
            expected_return_ty,
            finally_stack,
        ),
        MirStmt::ExprStmt(MirExpr::Call { callee, args, .. }) if callee == "print" => {
            // Two-phase evaluate-then-output (fixes #145): evaluate *all*
            // arguments left-to-right before emitting any output, so a
            // later argument's side effects (e.g. a user function that
            // itself calls `print`) complete before this `print`'s own
            // output begins -- matching CPython's left-to-right
            // argument-evaluation semantics. See `emit_eval_print_arg`/
            // `emit_write_print_arg` for the phase split and the
            // `cargo llvm-cov` region-attribution artifact that keeps
            // them as extracted helpers.
            string_render::emit_print_call(
                context,
                builder,
                module,
                rt,
                user_functions,
                locals,
                args,
            );
            Ok(())
        }
        MirStmt::ExprStmt(MirExpr::Call {
            callee: _,
            args: _,
            ty: pycc_mir::Ty::None,
        }) => store_stmt::emit_expr_stmt_stmt(
            context,
            builder,
            module,
            rt,
            user_functions,
            locals,
            stmt,
        ),
        MirStmt::ExprStmt(expr) => {
            let scalar = emit_expr(context, builder, module, rt, user_functions, locals, expr);
            // #146 Part 2 (D-181): a statement-position expression's value
            // is discarded outright, so nothing else will ever retire the
            // reference it was born with.
            release_scalar_if_temporary(context, builder, rt, expr, &scalar);
            // Part 1 of #1092: the same for a produced CPython object
            // (`o.update()`), whose reference nothing else will retire.
            object_release::release_if_produced(context, builder, module, expr, &scalar);
            Ok(())
        }
        MirStmt::Assign { .. } => {
            store_stmt::emit_assign_stmt(context, builder, module, rt, user_functions, locals, stmt)
        }
        MirStmt::NoOp => Ok(()),
        MirStmt::ReleaseName(name) => {
            let slot = locals
                .get(name)
                .cloned()
                .expect("a released name has a storage slot");
            heap_rc::release_slot(context, builder, rt, &slot.ty, slot.ptr);
            builder
                .build_store(
                    slot.ptr,
                    context
                        .ptr_type(inkwell::AddressSpace::default())
                        .const_null(),
                )
                .expect("build_store should not fail for a released slot");
            Ok(())
        }
        MirStmt::ForeignImport { bindings } => {
            foreign_import::emit_stmt(context, builder, module, rt, locals, bindings);
            Ok(())
        }
        MirStmt::Unreachable => {
            builder
                .build_unreachable()
                .expect("build_unreachable should terminate a statically impossible match path");
            Ok(())
        }
        MirStmt::If { .. } => range_loop::emit_if_stmt(
            context,
            builder,
            module,
            rt,
            user_functions,
            locals,
            stmt,
            expected_return_ty,
            finally_stack,
        ),
        MirStmt::While { .. } => range_loop::emit_while_stmt(
            context,
            builder,
            module,
            rt,
            user_functions,
            locals,
            stmt,
            expected_return_ty,
            finally_stack,
        ),
        MirStmt::Break => loop_control::emit_jump(
            context,
            builder,
            rt,
            finally_stack,
            loop_control::LoopJump::Break,
        ),
        MirStmt::Continue => loop_control::emit_jump(
            context,
            builder,
            rt,
            finally_stack,
            loop_control::LoopJump::Continue,
        ),
        MirStmt::LoopElse { .. } => range_loop::emit_loop_else_stmt(
            context,
            builder,
            module,
            rt,
            user_functions,
            locals,
            stmt,
            expected_return_ty,
            finally_stack,
        ),
        MirStmt::ForRange { .. } => range_loop::emit_for_range_stmt(
            context,
            builder,
            module,
            rt,
            user_functions,
            locals,
            stmt,
            expected_return_ty,
            finally_stack,
        ),
        MirStmt::ForList { .. } => iter_loops::emit_for_list_stmt(
            context,
            builder,
            module,
            rt,
            user_functions,
            locals,
            stmt,
            expected_return_ty,
            finally_stack,
        ),
        MirStmt::ForObject { .. } => iter_loops::emit_for_object_stmt(
            context,
            builder,
            module,
            rt,
            user_functions,
            locals,
            stmt,
            expected_return_ty,
            finally_stack,
        ),
        MirStmt::AssignMulti(_) => crate::multi_out::emit_assign_multi_stmt(
            context,
            builder,
            module,
            rt,
            user_functions,
            locals,
            stmt,
            expected_return_ty,
            finally_stack,
        ),
        MirStmt::SetOut { .. } => crate::multi_out::emit_set_out_stmt(
            context,
            builder,
            module,
            rt,
            user_functions,
            locals,
            stmt,
        ),
        MirStmt::ReturnBufferSlice { .. } => return_stmt::emit_return_buffer_slice_stmt(
            context,
            builder,
            module,
            rt,
            user_functions,
            locals,
            stmt,
            expected_return_ty,
            finally_stack,
        ),
        MirStmt::Return(..) => return_stmt::emit_return_stmt(
            context,
            builder,
            module,
            rt,
            user_functions,
            locals,
            stmt,
            expected_return_ty,
            finally_stack,
        ),
        MirStmt::BufferSet { .. } => store_stmt::emit_buffer_set_stmt(
            context,
            builder,
            module,
            rt,
            user_functions,
            locals,
            stmt,
        ),
        MirStmt::ListSet { .. } => store_stmt::emit_list_set_stmt(
            context,
            builder,
            module,
            rt,
            user_functions,
            locals,
            stmt,
        ),
        MirStmt::DictSet { .. } => store_stmt::emit_dict_set_stmt(
            context,
            builder,
            module,
            rt,
            user_functions,
            locals,
            stmt,
        ),
        MirStmt::ObjDelSlice { .. } => store_stmt::emit_obj_del_slice_stmt(
            context,
            builder,
            module,
            rt,
            user_functions,
            locals,
            stmt,
        ),
        MirStmt::ObjSetSlice { .. } => store_stmt::emit_obj_set_slice_stmt(
            context,
            builder,
            module,
            rt,
            user_functions,
            locals,
            stmt,
        ),
        MirStmt::ObjAttrSet { .. } => store_stmt::emit_obj_attr_set_stmt(
            context,
            builder,
            module,
            rt,
            user_functions,
            locals,
            stmt,
        ),
        MirStmt::ObjSetItem { .. } => store_stmt::emit_obj_set_item_stmt(
            context,
            builder,
            module,
            rt,
            user_functions,
            locals,
            stmt,
        ),
        MirStmt::ObjDelItem { .. } => store_stmt::emit_obj_del_item_stmt(
            context,
            builder,
            module,
            rt,
            user_functions,
            locals,
            stmt,
        ),
        MirStmt::ObjDelAttr { base, attr } => {
            let base_scalar = emit_expr(context, builder, module, rt, user_functions, locals, base);
            let held = object_release::hold(context, module, rt, base, &base_scalar);
            foreign_store::emit_del_attr(context, builder, module, rt, base_scalar, attr);
            held.release(builder, rt);
            Ok(())
        }
        MirStmt::AttrSet { .. } => store_stmt::emit_attr_set_stmt(
            context,
            builder,
            module,
            rt,
            user_functions,
            locals,
            stmt,
        ),
        MirStmt::ForDict { .. } => iter_loops::emit_for_dict_stmt(
            context,
            builder,
            module,
            rt,
            user_functions,
            locals,
            stmt,
            expected_return_ty,
            finally_stack,
        ),
        MirStmt::ForSet { .. } => iter_loops::emit_for_set_stmt(
            context,
            builder,
            module,
            rt,
            user_functions,
            locals,
            stmt,
            expected_return_ty,
            finally_stack,
        ),
        MirStmt::ListCompAssign { .. } => comp_assign::emit_list_comp_assign_stmt(
            context,
            builder,
            module,
            rt,
            user_functions,
            locals,
            stmt,
        ),
        MirStmt::SetCompAssign { .. } => comp_assign::emit_set_comp_assign_stmt(
            context,
            builder,
            module,
            rt,
            user_functions,
            locals,
            stmt,
        ),
        MirStmt::DictCompAssign { .. } => comp_assign::emit_dict_comp_assign_stmt(
            context,
            builder,
            module,
            rt,
            user_functions,
            locals,
            stmt,
        ),
        MirStmt::Seq(stmts) => emit_body(
            context,
            builder,
            module,
            rt,
            user_functions,
            locals,
            stmts,
            expected_return_ty,
            finally_stack,
        ),
        MirStmt::Raise { .. } => {
            raise_stmt::emit_raise_stmt(context, builder, module, rt, user_functions, locals, stmt)
        }
        MirStmt::ObjRaise { .. } => raise_stmt::emit_obj_raise_stmt(
            context,
            builder,
            module,
            rt,
            user_functions,
            locals,
            stmt,
        ),
        MirStmt::RaiseFrom { .. } => raise_stmt::emit_raise_from_stmt(
            context,
            builder,
            module,
            rt,
            user_functions,
            locals,
            stmt,
        ),
        MirStmt::Reraise => raise_stmt::emit_reraise_stmt(context, builder, rt, stmt),
        MirStmt::Try { .. } => raise_stmt::emit_try_stmt(
            context,
            builder,
            module,
            rt,
            user_functions,
            locals,
            stmt,
            expected_return_ty,
            finally_stack,
        ),
        MirStmt::TryStar { .. } => raise_stmt::emit_try_star_stmt(
            context,
            builder,
            module,
            rt,
            user_functions,
            locals,
            stmt,
            expected_return_ty,
            finally_stack,
        ),
    }
}
