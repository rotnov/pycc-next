//! Codegen for the exception *value* a `raise` produces: a constructed
//! exception or exception group (with its message), an existing exception
//! instance, and the per-raise frame record (#707).
//!
//! Narrow carve-out of `exception.rs` under AGENTS.md's decomposability
//! rule: exactly the cluster #1298 touches.

use super::*;

/// Emits a private constant holding a fixed byte string, and returns a
/// pointer to its bytes plus its length. Used for every string value a
/// caller needs to hand the runtime as a `(ptr, len)` pair baked in at
/// compile time -- an exception class's name, stored on the exception
/// object so an uncaught exception can be printed with its real class name
/// (Part 2 of #541, D-189), and a `raise`'s enclosing-frame function name,
/// stored via `pycc_rt_exception_set_frame` (#707). The bytes are not
/// NUL-terminated; the runtime reads exactly `len` of them. `global_name`
/// names the emitted LLVM global so each call site's IR reflects which
/// value it actually holds (e.g. `"exc_class_name"` or
/// `"exc_frame_function"`), rather than a name fixed for every caller.
pub(super) fn emit_str_bytes_constant<'ctx>(
    context: &'ctx Context,
    module: &inkwell::module::Module<'ctx>,
    value: &str,
    global_name: &str,
) -> (PointerValue<'ctx>, inkwell::values::IntValue<'ctx>) {
    let bytes = value.as_bytes();
    let global = module.add_global(
        context.i8_type().array_type(bytes.len() as u32),
        None,
        global_name,
    );
    global.set_initializer(&context.const_string(bytes, false));
    global.set_constant(true);
    global.set_linkage(inkwell::module::Linkage::Private);
    (
        global.as_pointer_value(),
        context.i64_type().const_int(bytes.len() as u64, false),
    )
}

/// Records `frame_function` (the enclosing function's source name, or
/// `"<module>"` at top level) on `exc_obj` via `pycc_rt_exception_set_frame`
/// (#707): the frame identifying where this `raise` executed, for
/// `exception_print_and_exit`'s traceback rendering. Called once per
/// `Raise`/`RaiseFrom` on the raised exception only -- never on a `from`
/// clause's cause -- immediately after `emit_exception_value` produces
/// `exc_obj`, before the runtime's pending-exception state is set.
pub(super) fn emit_exception_set_frame<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    module: &inkwell::module::Module<'ctx>,
    rt: &RtFns<'ctx>,
    exc_obj: PointerValue<'ctx>,
    frame_function: &str,
) {
    let (frame_ptr, frame_len) =
        emit_str_bytes_constant(context, module, frame_function, "exc_frame_function");
    builder
        .build_call(
            rt.exception_set_frame,
            &[exc_obj.into(), frame_ptr.into(), frame_len.into()],
            "",
        )
        .expect("build_call should not fail for exception_set_frame");
}

#[allow(clippy::too_many_arguments)]
pub(super) fn emit_exception_value<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    module: &inkwell::module::Module<'ctx>,
    rt: &RtFns<'ctx>,
    user_functions: &HashMap<&str, UserFunction<'ctx>>,
    locals: &HashMap<String, StorageSlot<'ctx>>,
    value: &MirExceptionValue,
    role: &str,
) -> Result<PointerValue<'ctx>, String> {
    match value {
        MirExceptionValue::Constructed {
            type_tag,
            class_name,
            message,
        } => {
            let message_scalar = emit_expr(
                context,
                builder,
                module,
                rt,
                user_functions,
                locals,
                message,
            );
            // #1298: the exception owns its own reference to its message,
            // whatever the message's shape. A borrowed read (`msg`, `obj.s`)
            // is retained here, so the exception's pointer survives its
            // owner releasing it; a fresh message already arrives as +1.
            // The reference is never released because exception objects
            // are leak-only -- the same leak a literal message has.
            let message = incref_if_str_duplicate(builder, rt, message, message_scalar);
            let Scalar::Str(message) = message else {
                let prefix = if role == "cause" {
                    "raise cause"
                } else {
                    "raise"
                };
                return Err(format!("{prefix} message must be a string"));
            };
            let type_tag = context.i8_type().const_int(*type_tag as u64, false);
            let (class_name_ptr, class_name_len) =
                emit_str_bytes_constant(context, module, class_name, "exc_class_name");
            Ok(builder
                .build_call(
                    rt.exception_alloc,
                    &[
                        type_tag.into(),
                        class_name_ptr.into(),
                        class_name_len.into(),
                        message.into(),
                    ],
                    &format!("{role}_alloc"),
                )
                .expect("build_call should not fail for exception_alloc")
                .try_as_basic_value()
                .expect_basic("pycc_rt_exception_alloc returns a pointer")
                .into_pointer_value())
        }
        // Part 3 of #382 (#542, PEP 654, D-202): `ExceptionGroup(msg,
        // [e1, e2, ...])` construction. Each member evaluates to a
        // `Scalar::Instance` pointer -- an already-allocated exception, from
        // a caught binding or an earlier `raise` -- exactly like `Existing`
        // below; this arm additionally collects them into a stack array and
        // hands the pointer/length pair to `pycc_rt_exception_group_alloc`.
        MirExceptionValue::ConstructedGroup {
            type_tag,
            class_name,
            message,
            members,
        } => {
            let message_scalar = emit_expr(
                context,
                builder,
                module,
                rt,
                user_functions,
                locals,
                message,
            );
            // #1298: as in the `Constructed` arm, the group owns its own
            // reference to its message. Leak-only exception objects are
            // also what makes it sound for `pycc_rt`'s group partition to
            // share this pointer with each derived group without an incref:
            // a change that frees exception objects must first give each
            // derived group its own reference and release the message in
            // the destructor.
            let message_scalar = incref_if_str_duplicate(builder, rt, message, message_scalar);
            let Scalar::Str(message_scalar) = message_scalar else {
                return Err(format!(
                    "{} message must be a string",
                    if role == "cause" {
                        "raise cause"
                    } else {
                        "raise"
                    }
                ));
            };
            let ptr_type = context.ptr_type(inkwell::AddressSpace::default());
            let members_array = builder
                .build_alloca(
                    ptr_type.array_type(members.len() as u32),
                    &format!("{role}_group_members"),
                )
                .expect("build_alloca should not fail for a group's member array");
            for (index, member) in members.iter().enumerate() {
                let member_scalar =
                    emit_expr(context, builder, module, rt, user_functions, locals, member);
                let Scalar::Instance(member_ptr) = member_scalar else {
                    return Err(format!("{role} group member must be an exception instance"));
                };
                let slot = unsafe {
                    builder
                        .build_gep(
                            ptr_type,
                            members_array,
                            &[context.i64_type().const_int(index as u64, false)],
                            &format!("{role}_group_member_{index}"),
                        )
                        .expect("build_gep should not fail for a group's member slot")
                };
                builder
                    .build_store(slot, member_ptr)
                    .expect("build_store should not fail for a group's member slot");
            }
            let type_tag = context.i8_type().const_int(*type_tag as u64, false);
            let (class_name_ptr, class_name_len) =
                emit_str_bytes_constant(context, module, class_name, "exc_class_name");
            let members_len = context.i64_type().const_int(members.len() as u64, false);
            Ok(builder
                .build_call(
                    rt.exception_group_alloc,
                    &[
                        type_tag.into(),
                        class_name_ptr.into(),
                        class_name_len.into(),
                        message_scalar.into(),
                        members_array.into(),
                        members_len.into(),
                    ],
                    &format!("{role}_group_alloc"),
                )
                .expect("build_call should not fail for exception_group_alloc")
                .try_as_basic_value()
                .expect_basic("pycc_rt_exception_group_alloc returns a pointer")
                .into_pointer_value())
        }
        MirExceptionValue::Existing(expr) => {
            let value = emit_expr(context, builder, module, rt, user_functions, locals, expr);
            let Scalar::Instance(value) = value else {
                return Err(format!("raise {role} must be an exception instance"));
            };
            Ok(value)
        }
    }
}
