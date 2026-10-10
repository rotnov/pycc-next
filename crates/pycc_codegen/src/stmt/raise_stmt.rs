//! `raise`, `raise ... from`, re-raise and `try`/`try*` statements.

use super::*;

// #382 (PR-22 Part 1): `raise ExceptionType("msg")` — allocate an
// exception object, set the pending exception state, then terminate
// this block. `emit_body` routes that explicit raise directly to the
// nearest installed exception target (a `try` handler, the caller's
// exceptional exit, or the top-level exit).
pub(crate) fn emit_raise_stmt<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    module: &inkwell::module::Module<'ctx>,
    rt: &RtFns<'ctx>,
    user_functions: &HashMap<&str, UserFunction<'ctx>>,
    locals: &mut HashMap<String, StorageSlot<'ctx>>,
    stmt: &MirStmt,
) -> Result<(), String> {
    let MirStmt::Raise {
        exception,
        frame_function,
    } = stmt
    else {
        unreachable!()
    };
    let exc_obj = emit_exception_value(
        context,
        builder,
        module,
        rt,
        user_functions,
        locals,
        exception,
        "exception",
    )?;
    emit_exception_set_frame(context, builder, module, rt, exc_obj, frame_function);
    builder
        .build_call(rt.exception_raise, &[exc_obj.into()], "")
        .expect("build_call should not fail for exception_raise");
    // Terminate the block with `unreachable` — the exception is
    // already set in the pending state. Do NOT position at a new
    // block after the unreachable: `emit_body` recognizes this
    // marker and replaces it with a branch to the current exception
    // target, while an unreachable block correctly does not fall
    // through in other structural checks.
    builder
        .build_unreachable()
        .expect("build_unreachable should not fail after raise");
    Ok(())
}

// Part 9 of #1371: `raise o` with a CPython object `o`.
pub(crate) fn emit_obj_raise_stmt<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    module: &inkwell::module::Module<'ctx>,
    rt: &RtFns<'ctx>,
    user_functions: &HashMap<&str, UserFunction<'ctx>>,
    locals: &mut HashMap<String, StorageSlot<'ctx>>,
    stmt: &MirStmt,
) -> Result<(), String> {
    let MirStmt::ObjRaise { value } = stmt else {
        unreachable!()
    };
    foreign_raise::emit_obj_raise(context, builder, module, rt, user_functions, locals, value);
    Ok(())
}

// #382: `raise ExceptionType("msg") from CauseType("cause")` —
// allocate both exception and cause objects, then raise with cause.
// Same `unreachable` approach as `Raise`.
pub(crate) fn emit_raise_from_stmt<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    module: &inkwell::module::Module<'ctx>,
    rt: &RtFns<'ctx>,
    user_functions: &HashMap<&str, UserFunction<'ctx>>,
    locals: &mut HashMap<String, StorageSlot<'ctx>>,
    stmt: &MirStmt,
) -> Result<(), String> {
    let MirStmt::RaiseFrom {
        exception,
        cause,
        frame_function,
    } = stmt
    else {
        unreachable!()
    };
    let exc_obj = emit_exception_value(
        context,
        builder,
        module,
        rt,
        user_functions,
        locals,
        exception,
        "exception",
    )?;
    let cause_obj = emit_exception_value(
        context,
        builder,
        module,
        rt,
        user_functions,
        locals,
        cause,
        "cause",
    )?;
    emit_exception_set_frame(context, builder, module, rt, exc_obj, frame_function);
    builder
        .build_call(
            rt.exception_raise_with_cause,
            &[exc_obj.into(), cause_obj.into()],
            "",
        )
        .expect("build_call should not fail for exception_raise_with_cause");
    // Terminate the block with `unreachable` — the exception is
    // already set in the pending state. See `Raise`'s comment for
    // the rationale (same approach).
    builder
        .build_unreachable()
        .expect("build_unreachable should not fail after raise_from");
    Ok(())
}

// #382: Bare `raise` (re-raise) — load the lexically enclosing
// handler's exception value, re-raise it, then terminate the block with
// `unreachable`. The enclosing check point will detect the active
// exception and handle it.
pub(crate) fn emit_reraise_stmt<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    rt: &RtFns<'ctx>,
    stmt: &MirStmt,
) -> Result<(), String> {
    let MirStmt::Reraise = stmt else {
        unreachable!()
    };
    let saved_exc = *rt
        .exceptions
        .reraise_values
        .borrow()
        .last()
        .expect("pycc_types rejects bare raise outside an except handler");
    let exc_val = builder
        .build_load(
            context.ptr_type(inkwell::AddressSpace::default()),
            saved_exc,
            "saved_exc_val",
        )
        .expect("build_load should not fail for a handler exception slot")
        .into_pointer_value();
    builder
        .build_call(rt.exception_raise, &[exc_val.into()], "")
        .expect("build_call should not fail for exception_raise");
    // Terminate the block with `unreachable` — the exception is
    // already set in the pending state. See `Raise`'s comment for
    // the rationale (same approach).
    builder
        .build_unreachable()
        .expect("build_unreachable should not fail after reraise");
    Ok(())
}

// #382 (PR-22 Part 1): `try`/`except`/`else`/`finally` codegen.
// The D-173 model uses explicit check-and-branch: after the try
// body completes, generated code checks `pycc_rt_exception_active`
// and, if an exception is pending, dispatches to the handler chain.
// Each handler checks `pycc_rt_exception_type_matches` against its
// declared type (or catches all for a bare `except:`). The handler
// clears the exception state before running its body. The `else`
// body runs only if no exception was raised. The `finally` body
// always runs (implemented as a shared merge block that both the
// normal-completion and exception paths branch to before exiting).
#[allow(clippy::too_many_arguments)]
pub(crate) fn emit_try_stmt<'ctx>(
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
    let MirStmt::Try {
        body,
        handlers,
        orelse,
        finalbody,
    } = stmt
    else {
        unreachable!()
    };
    exception::emit_try(
        context,
        builder,
        module,
        rt,
        user_functions,
        locals,
        body,
        handlers,
        orelse,
        finalbody,
        expected_return_ty,
        finally_stack,
    )
}

// Part 3 of #382 (#542, PEP 654, D-202): `try`/`except*`/`else`/
// `finally` codegen -- see `exception::emit_try_star`'s own doc
// comment for how its handler dispatch differs from `Try`'s above.
#[allow(clippy::too_many_arguments)]
pub(crate) fn emit_try_star_stmt<'ctx>(
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
    let MirStmt::TryStar {
        body,
        handlers,
        orelse,
        finalbody,
    } = stmt
    else {
        unreachable!()
    };
    exception::emit_try_star(
        context,
        builder,
        module,
        rt,
        user_functions,
        locals,
        body,
        handlers,
        orelse,
        finalbody,
        expected_return_ty,
        finally_stack,
    )
}
