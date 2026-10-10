//! `try`/`except*` (exception group) lowering.

use super::*;

// The fixed builtin runtime type tag for the `ExceptionGroup` class (Part 3
// of #382, #542, PEP 654, D-202) is `pycc_hir::exception::
// EXCEPTION_GROUP_TYPE_TAG`, re-exported here through `pycc_mir` and
// *derived* from `ExceptionGroup`'s position in `BUILTIN_EXCEPTION_CLASSES`
// rather than hand-maintained as a separate literal (D-194's derivation
// discipline). `emit_try_star` passes this tag (and the class name below) to
// `pycc_rt_exception_group_partition` whenever it needs to build a fresh
// reconstructed subgroup, regardless of the original raised object's own
// dynamic class -- a deliberate D-202 simplification: codegen does not track
// a raised group's polymorphic subclass identity through partitioning, so
// every subgroup an `except*` clause binds is reported as a plain
// `ExceptionGroup`, never the original (possibly user-defined)
// `BaseExceptionGroup` subclass name.

/// Emits `try`/`except*`/`else`/`finally` (Part 3 of #382, #542, PEP 654).
///
/// Structurally this mirrors [`emit_try`] almost exactly -- the same five
/// basic blocks, the same `FinallyTarget`/`finally`/`else`/return-routing
/// machinery, reused verbatim below. The two differ only in the handler
/// dispatch section: `Try`'s handlers are mutually exclusive (first
/// `pycc_rt_exception_type_matches` match wins, the rest are skipped
/// entirely), whereas PEP 654 requires every `except*` clause to run against
/// whatever the raised exception's *previous* clauses left unmatched, since
/// more than one clause may claim a member out of the same raised group. So
/// instead of a boolean `type_matches` test, each clause here calls
/// `pycc_rt_exception_group_partition` against a `current_group` value
/// threaded from one clause to the next (the runtime's own "remaining
/// members" output), runs its body on whatever it matched (if anything), and
/// after the last clause reraises whatever is still unmatched -- PEP 654
/// propagates a leftover remainder rather than silently discarding it.
#[allow(clippy::too_many_arguments)]
pub(crate) fn emit_try_star<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    module: &inkwell::module::Module<'ctx>,
    rt: &RtFns<'ctx>,
    user_functions: &HashMap<&str, UserFunction<'ctx>>,
    locals: &mut HashMap<String, StorageSlot<'ctx>>,
    body: &[MirStmt],
    handlers: &[pycc_mir::MirExceptHandler],
    orelse: &[MirStmt],
    finalbody: &[MirStmt],
    expected_return_ty: pycc_mir::Ty,
    finally_stack: &mut Vec<FinallyTarget<'ctx>>,
) -> Result<(), String> {
    let function = builder.get_insert_block().unwrap().get_parent().unwrap();
    let ptr_type = context.ptr_type(inkwell::AddressSpace::default());

    let try_body_bb = context.append_basic_block(function, "trystar_body");
    let handler_dispatch_bb = context.append_basic_block(function, "trystar_handler_dispatch");
    let else_bb = context.append_basic_block(function, "trystar_else");
    let finally_bb = context.append_basic_block(function, "trystar_finally");
    let after_bb = context.append_basic_block(function, "trystar_after");

    let has_finally = !finalbody.is_empty();
    let saved_is_returning: Option<PointerValue<'ctx>>;
    let saved_ret_slot: Option<PointerValue<'ctx>>;
    if has_finally {
        let is_returning = alloca_at_entry(builder, context.i8_type(), "trystar_is_returning");
        builder
            .build_store(is_returning, context.i8_type().const_zero())
            .expect("build_store should not fail for is_returning init");
        let ret_slot = if expected_return_ty == pycc_mir::Ty::None {
            None
        } else {
            let slot = alloca_at_entry(
                builder,
                ty_to_basic_type(context, expected_return_ty.clone()),
                "trystar_ret_slot",
            );
            Some(slot)
        };
        saved_is_returning = Some(is_returning);
        saved_ret_slot = ret_slot;
        finally_stack.push(FinallyTarget {
            finally_bb,
            ret_slot,
            is_returning,
        });
    } else {
        saved_is_returning = None;
        saved_ret_slot = None;
    }

    builder
        .build_unconditional_branch(try_body_bb)
        .expect("build_unconditional_branch should not fail entering try* body");
    builder.position_at_end(try_body_bb);

    rt.exceptions.targets.borrow_mut().push(handler_dispatch_bb);
    emit_body(
        context,
        builder,
        module,
        rt,
        user_functions,
        locals,
        body,
        expected_return_ty.clone(),
        finally_stack,
    )?;
    rt.exceptions.targets.borrow_mut().pop();
    erase_unreachable_if_present(builder);
    let body_falls_through = builder
        .get_insert_block()
        .unwrap()
        .get_terminator()
        .is_none();
    if body_falls_through {
        let active = crate::int_fast::emit_exception_pending(
            context,
            builder,
            rt,
            "trystar_body_exc_active",
        );
        let has_exc = builder
            .build_int_compare(
                inkwell::IntPredicate::NE,
                active,
                context.i8_type().const_zero(),
                "trystar_body_has_exc",
            )
            .expect("build_int_compare should not fail");
        builder
            .build_conditional_branch(has_exc, handler_dispatch_bb, else_bb)
            .expect("build_conditional_branch should not fail");
    }

    // Handler dispatch: `except*` partitions the raised group across every
    // clause in source order (PEP 654), unlike `Try`'s mutually exclusive
    // first-match-wins chain. `pycc_hir::stmt::lower_stmt`'s own `Stmt::Try`
    // arm never lowers a bare `try`/`finally` (no `except*` clauses at all)
    // to `HirStmt::TryStar` -- only a real `except*` clause sets `is_star` --
    // so unlike `Try`, which codegens a real `handlers.is_empty()` branch for
    // exactly that case, there is no reachable empty-`handlers` case to
    // branch on here.
    builder.position_at_end(handler_dispatch_bb);
    let current_group_slot = alloca_at_entry(builder, ptr_type, "trystar_current_group");
    {
        let exc_val = builder
            .build_call(rt.exception_value, &[], "trystar_exc_val")
            .expect("build_call should not fail for exception_value")
            .try_as_basic_value()
            .expect_basic("pycc_rt_exception_value returns a pointer")
            .into_pointer_value();
        // The raised value becomes this dispatch chain's own to manage --
        // clear the runtime's pending state immediately, exactly once,
        // before any clause's body can run. No clause below re-sets the
        // pending state itself; only a clause body that raises a brand-new
        // exception, or the final unmatched remainder reraised after the
        // last clause, does.
        builder
            .build_call(rt.exception_clear, &[], "")
            .expect("build_call should not fail for exception_clear");
        builder
            .build_store(current_group_slot, exc_val)
            .expect("build_store should not fail for the trystar current-group slot");
    }

    let mut dispatch_bbs: Vec<inkwell::basic_block::BasicBlock> = Vec::new();
    let mut handler_body_bbs: Vec<inkwell::basic_block::BasicBlock> = Vec::new();
    for i in 0..handlers.len() {
        dispatch_bbs.push(context.append_basic_block(function, &format!("trystar_dispatch_{i}")));
        handler_body_bbs
            .push(context.append_basic_block(function, &format!("trystar_handler_{i}")));
    }
    let reraise_remainder_bb = context.append_basic_block(function, "trystar_reraise_remainder");

    builder
        .build_unconditional_branch(dispatch_bbs[0])
        .expect("build_unconditional_branch should not fail");

    let (group_name_ptr, group_name_len) =
        emit_str_bytes_constant(context, module, "ExceptionGroup", "exc_class_name");
    let group_type_tag = context
        .i8_type()
        .const_int(u64::from(EXCEPTION_GROUP_TYPE_TAG), false);

    for (i, handler) in handlers.iter().enumerate() {
        let next_bb = if i + 1 < handlers.len() {
            dispatch_bbs[i + 1]
        } else {
            reraise_remainder_bb
        };

        builder.position_at_end(dispatch_bbs[i]);
        let group_ptr = builder
            .build_load(ptr_type, current_group_slot, "trystar_group")
            .expect("build_load should not fail for the trystar current-group slot")
            .into_pointer_value();

        // An earlier, broader clause (e.g. one matching the universal
        // `EXCEPTION_TYPE_EXCEPTION` tag) can already have consumed every
        // remaining member, in which case `rest_ptr` from that clause's own
        // partition call -- stored into `current_group_slot` below -- is
        // null (`build_group_or_null` returns null for an empty half).
        // `pycc_rt_exception_group_partition`'s safety contract requires a
        // non-null `group`, so this clause must not be dispatched at all
        // when nothing is left to test; skip straight to the next clause
        // (or `reraise_remainder_bb`) instead of calling the runtime
        // function with a null pointer.
        if i > 0 {
            let group_is_null = builder
                .build_is_null(group_ptr, "trystar_group_is_null")
                .expect("build_is_null should not fail for a pointer comparison");
            let dispatch_body_bb =
                context.append_basic_block(function, &format!("trystar_dispatch_body_{i}"));
            builder
                .build_conditional_branch(group_is_null, next_bb, dispatch_body_bb)
                .expect("build_conditional_branch should not fail");
            builder.position_at_end(dispatch_body_bb);
        }

        // `pycc_mir` always resolves an `except*` clause's tag set --
        // `pycc_hir::stmt::lower_stmt`'s own `Stmt::Try` arm rejects a
        // typeless `except*:` at parse time before lowering ever runs (see
        // that module's `lower_try_star_bare_except_star_is_rejected_at_
        // parse_time` test) -- so this is never `Try`'s own bare-`except:`
        // catch-all `None` case.
        let tags = handler
            .exc_type_tag
            .as_deref()
            .expect("an except* handler always carries a resolved tag set");
        let tags_array_ty = context.i8_type().array_type(tags.len() as u32);
        let tags_alloca = alloca_at_entry(builder, tags_array_ty, &format!("trystar_tags_{i}"));
        for (j, tag) in tags.iter().enumerate() {
            let slot = unsafe {
                builder
                    .build_gep(
                        context.i8_type(),
                        tags_alloca,
                        &[context.i64_type().const_int(j as u64, false)],
                        &format!("trystar_tag_{i}_{j}"),
                    )
                    .expect("build_gep should not fail for a trystar tag slot")
            };
            builder
                .build_store(slot, context.i8_type().const_int(u64::from(*tag), false))
                .expect("build_store should not fail for a trystar tag slot");
        }
        let tags_len = context.i64_type().const_int(tags.len() as u64, false);

        let matched_slot = alloca_at_entry(builder, ptr_type, &format!("trystar_matched_{i}"));
        let rest_slot = alloca_at_entry(builder, ptr_type, &format!("trystar_rest_{i}"));

        builder
            .build_call(
                rt.exception_group_partition,
                &[
                    group_ptr.into(),
                    tags_alloca.into(),
                    tags_len.into(),
                    group_type_tag.into(),
                    group_name_ptr.into(),
                    group_name_len.into(),
                    matched_slot.into(),
                    rest_slot.into(),
                ],
                "",
            )
            .expect("build_call should not fail for exception_group_partition");
        let matched_ptr = builder
            .build_load(ptr_type, matched_slot, "trystar_matched_ptr")
            .expect("build_load should not fail for a trystar matched-out slot")
            .into_pointer_value();
        let rest_ptr = builder
            .build_load(ptr_type, rest_slot, "trystar_rest_ptr")
            .expect("build_load should not fail for a trystar rest-out slot")
            .into_pointer_value();
        builder
            .build_store(current_group_slot, rest_ptr)
            .expect("build_store should not fail for the trystar current-group slot");

        let no_match = builder
            .build_is_null(matched_ptr, "trystar_no_match")
            .expect("build_is_null should not fail for a pointer comparison");
        builder
            .build_conditional_branch(no_match, next_bb, handler_body_bbs[i])
            .expect("build_conditional_branch should not fail");

        builder.position_at_end(handler_body_bbs[i]);
        if let Some(binding_name) = &handler.binding_name {
            let exc_slot = alloca_at_entry(builder, ptr_type, binding_name);
            builder
                .build_store(exc_slot, matched_ptr)
                .expect("build_store should not fail for an except* binding");
            locals.insert(
                binding_name.clone(),
                StorageSlot {
                    ptr: exc_slot,
                    ty: handler
                        .binding_ty
                        .clone()
                        .expect("an except* binding always carries its static type"),
                    initialized: None,
                },
            );
        }
        // A bare `raise` inside an `except*` body reraises the exact
        // matched subgroup this clause was handed, mirroring `Try`'s own
        // `reraise_values` mechanism.
        let saved_exc = alloca_at_entry(builder, ptr_type, &format!("trystar_saved_exc_{i}"));
        builder
            .build_store(saved_exc, matched_ptr)
            .expect("build_store should not fail for a trystar reraise slot");
        rt.exceptions.reraise_values.borrow_mut().push(saved_exc);
        // D-202 simplification: a *new* exception raised from inside an
        // `except*` body (as opposed to a bare `raise` of the matched
        // subgroup above) is not folded back into this statement's
        // still-unhandled remainder the way CPython's own PEP 654 "derived
        // exception group" chaining would -- it propagates directly to
        // `finally_bb`, exactly like `Try`'s own handler bodies,
        // abandoning any later `except*` clauses and any remaining
        // unmatched members. This keeps `except*` codegen's control flow a
        // straight-line extension of `Try`'s existing check-and-branch
        // model rather than requiring a second exception-group merge step
        // with no `Try` precedent.
        rt.exceptions.targets.borrow_mut().push(finally_bb);
        emit_body(
            context,
            builder,
            module,
            rt,
            user_functions,
            locals,
            &handler.body,
            expected_return_ty.clone(),
            finally_stack,
        )?;
        rt.exceptions.targets.borrow_mut().pop();
        rt.exceptions.reraise_values.borrow_mut().pop();
        erase_unreachable_if_present(builder);
        let handler_falls_through = builder
            .get_insert_block()
            .unwrap()
            .get_terminator()
            .is_none();
        if handler_falls_through {
            builder
                .build_unconditional_branch(next_bb)
                .expect("build_unconditional_branch should not fail");
        }
    }

    // Every clause has now run (or been skipped). Reraise whatever
    // remains unmatched, if anything -- PEP 654 propagates a leftover
    // remainder rather than silently discarding it.
    builder.position_at_end(reraise_remainder_bb);
    {
        let remainder = builder
            .build_load(ptr_type, current_group_slot, "trystar_remainder")
            .expect("build_load should not fail for the trystar current-group slot")
            .into_pointer_value();
        let no_remainder = builder
            .build_is_null(remainder, "trystar_remainder_is_null")
            .expect("build_is_null should not fail for a pointer comparison");
        let reraise_bb = context.append_basic_block(function, "trystar_reraise");
        builder
            .build_conditional_branch(no_remainder, finally_bb, reraise_bb)
            .expect("build_conditional_branch should not fail");
        builder.position_at_end(reraise_bb);
        builder
            .build_call(rt.exception_raise, &[remainder.into()], "")
            .expect("build_call should not fail for exception_raise");
        builder
            .build_unconditional_branch(finally_bb)
            .expect("build_unconditional_branch should not fail");
    }

    // Else body: runs only if no exception was raised (identical to
    // `Try`'s own `else_bb`).
    builder.position_at_end(else_bb);
    if orelse.is_empty() {
        builder
            .build_unconditional_branch(finally_bb)
            .expect("build_unconditional_branch should not fail");
    } else {
        rt.exceptions.targets.borrow_mut().push(finally_bb);
        emit_body(
            context,
            builder,
            module,
            rt,
            user_functions,
            locals,
            orelse,
            expected_return_ty.clone(),
            finally_stack,
        )?;
        rt.exceptions.targets.borrow_mut().pop();
        erase_unreachable_if_present(builder);
        let else_falls_through = builder
            .get_insert_block()
            .unwrap()
            .get_terminator()
            .is_none();
        if else_falls_through {
            builder
                .build_unconditional_branch(finally_bb)
                .expect("build_unconditional_branch should not fail");
        }
    }

    if has_finally {
        finally_stack.pop();
    }

    builder.position_at_end(finally_bb);
    let finally_exception_bb = context.append_basic_block(function, "trystar_finally_exception");
    let pending_exception = if finalbody.is_empty() {
        None
    } else {
        let active = crate::int_fast::emit_exception_pending(
            context,
            builder,
            rt,
            "trystar_finally_pending_active",
        );
        let value = builder
            .build_call(rt.exception_value, &[], "trystar_finally_pending_value")
            .expect("build_call should not fail for exception_value")
            .try_as_basic_value()
            .expect_basic("pycc_rt_exception_value returns a pointer")
            .into_pointer_value();
        builder
            .build_call(rt.exception_clear, &[], "")
            .expect("build_call should not fail for exception_clear");
        Some((active, value))
    };
    if finalbody.is_empty() {
        // No finally body — just check is_returning / branch.
    } else {
        rt.exceptions
            .targets
            .borrow_mut()
            .push(finally_exception_bb);
        emit_body(
            context,
            builder,
            module,
            rt,
            user_functions,
            locals,
            finalbody,
            expected_return_ty.clone(),
            finally_stack,
        )?;
        rt.exceptions.targets.borrow_mut().pop();
    }
    erase_unreachable_if_present(builder);
    let finally_falls_through = builder
        .get_insert_block()
        .unwrap()
        .get_terminator()
        .is_none();
    if finally_falls_through {
        if let Some((pending_active, pending_value)) = pending_exception {
            let had_pending = builder
                .build_int_compare(
                    inkwell::IntPredicate::NE,
                    pending_active,
                    context.i8_type().const_zero(),
                    "trystar_finally_had_pending",
                )
                .expect("build_int_compare should not fail");
            let restore_bb =
                context.append_basic_block(function, "trystar_finally_restore_exception");
            let restored_bb =
                context.append_basic_block(function, "trystar_finally_exception_restored");
            builder
                .build_conditional_branch(had_pending, restore_bb, restored_bb)
                .expect("build_conditional_branch should not fail");
            builder.position_at_end(restore_bb);
            builder
                .build_call(rt.exception_raise, &[pending_value.into()], "")
                .expect("build_call should not fail for exception_raise");
            builder
                .build_unconditional_branch(restored_bb)
                .expect("build_unconditional_branch should not fail");
            builder.position_at_end(restored_bb);
        }
        if has_finally {
            let is_returning = saved_is_returning.unwrap();
            let ret_slot = saved_ret_slot;
            let flag_val = builder
                .build_load(context.i8_type(), is_returning, "trystar_finally_ret_flag")
                .expect("build_load should not fail for is_returning flag")
                .into_int_value();
            let is_ret = builder
                .build_int_compare(
                    inkwell::IntPredicate::NE,
                    flag_val,
                    context.i8_type().const_zero(),
                    "trystar_finally_is_ret",
                )
                .expect("build_int_compare should not fail");
            let ret_bb = context.append_basic_block(function, "trystar_finally_ret");
            builder
                .build_conditional_branch(is_ret, ret_bb, after_bb)
                .expect("build_conditional_branch should not fail");
            builder.position_at_end(ret_bb);
            if let Some(slot) = ret_slot {
                let ret_val = builder
                    .build_load(
                        ty_to_basic_type(context, expected_return_ty.clone()),
                        slot,
                        "trystar_finally_ret_val",
                    )
                    .expect("build_load should not fail for ret_slot");
                if let Some(outer) = finally_stack.last_mut() {
                    let outer_slot = outer
                        .ret_slot
                        .expect("nested finally in the same non-None function has a return slot");
                    builder
                        .build_store(outer_slot, ret_val)
                        .expect("build_store should not fail for outer ret_slot");
                    builder
                        .build_store(outer.is_returning, context.i8_type().const_int(1, false))
                        .expect("build_store should not fail for outer is_returning");
                    builder
                        .build_unconditional_branch(outer.finally_bb)
                        .expect("build_unconditional_branch should not fail");
                } else {
                    builder
                        .build_return(Some(&ret_val))
                        .expect("build_return should not fail for a finally-routed return");
                }
            } else if let Some(outer) = finally_stack.last_mut() {
                builder
                    .build_store(outer.is_returning, context.i8_type().const_int(1, false))
                    .expect("build_store should not fail for outer is_returning");
                builder
                    .build_unconditional_branch(outer.finally_bb)
                    .expect("build_unconditional_branch should not fail");
            } else if function.get_type().get_return_type().is_none() {
                builder
                    .build_return(None)
                    .expect("build_return should not fail for a finally-routed void return");
            } else {
                builder
                    .build_unreachable()
                    .expect("build_unreachable should not fail for dead ret_bb");
            }
        } else {
            builder
                .build_unconditional_branch(after_bb)
                .expect("build_unconditional_branch should not fail");
        }
    }

    builder.position_at_end(finally_exception_bb);
    builder
        .build_unconditional_branch(after_bb)
        .expect("build_unconditional_branch should propagate a finally exception");

    builder.position_at_end(after_bb);
    guard_statement_effects(context, builder, rt);
    Ok(())
}
