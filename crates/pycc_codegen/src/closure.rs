//! Function values (`Ty::Callable`): building one and calling one.
//!
//! A function value is a `pycc_rt::closure::PyClosureObj`: a module-level
//! function `F` with its leading parameters bound to captured values (a
//! nested function's enclosing variables, lifted by
//! `pycc_ast::desugar::closures`, or none for a module-level function used
//! as a value). Each captured value is stored as one word and owned by the
//! object. Two functions are emitted once per `F`:
//!
//! * `pycc_closure_call.F(obj, args...)` loads the captured values, takes a
//!   reference of each for `F`'s parameter slots (which own their
//!   arguments) and calls `F` with them and `args`. Every function value of
//!   one `Callable[[A...], R]` type is called through this one signature,
//!   whatever it captured.
//! * `pycc_closure_drop.F(obj)` releases the captured values; the runtime
//!   calls it when the last reference to the object goes.

use super::*;
use inkwell::values::{BasicMetadataValueEnum, BasicValueEnum, IntValue};
use pycc_mir::Ty;

/// `pycc_rt::closure::CALL_OFFSET`.
const CALL_OFFSET: u64 = 8;
/// `pycc_rt::closure::CAPTURES_OFFSET`.
const CAPTURES_OFFSET: u64 = 32;

/// The runtime functions of function values.
pub(crate) struct ClosureFns<'ctx> {
    pub(crate) new: FunctionValue<'ctx>,
    pub(crate) incref: FunctionValue<'ctx>,
    pub(crate) decref: FunctionValue<'ctx>,
}

impl<'ctx> ClosureFns<'ctx> {
    pub(crate) fn declare(context: &'ctx Context, module: &inkwell::module::Module<'ctx>) -> Self {
        let ptr_type = context.ptr_type(inkwell::AddressSpace::default());
        let void_type = context.void_type();
        let declare = |name: &str, fn_type: inkwell::types::FunctionType<'ctx>| {
            module.get_function(name).unwrap_or_else(|| {
                module.add_function(name, fn_type, Some(inkwell::module::Linkage::External))
            })
        };
        ClosureFns {
            new: declare(
                "pycc_rt_closure_new",
                ptr_type.fn_type(
                    &[ptr_type.into(), ptr_type.into(), context.i64_type().into()],
                    false,
                ),
            ),
            incref: declare(
                "pycc_rt_closure_incref",
                void_type.fn_type(&[ptr_type.into()], false),
            ),
            decref: declare(
                "pycc_rt_closure_decref",
                void_type.fn_type(&[ptr_type.into()], false),
            ),
        }
    }
}

/// The address of captured word `index` of `obj`.
fn capture_slot<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    obj: PointerValue<'ctx>,
    index: usize,
) -> PointerValue<'ctx> {
    // SAFETY: the offset stays inside the object `pycc_rt_closure_new`
    // allocated with this many captured words.
    unsafe {
        builder
            .build_gep(
                context.i8_type(),
                obj,
                &[context
                    .i64_type()
                    .const_int(CAPTURES_OFFSET + 8 * index as u64, false)],
                "closure_capture",
            )
            .expect("build_gep should not fail for a byte offset")
    }
}

/// `value` as one 64-bit word.
fn to_word<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    value: BasicValueEnum<'ctx>,
) -> IntValue<'ctx> {
    let i64_type = context.i64_type();
    match value {
        BasicValueEnum::IntValue(v) if v.get_type().get_bit_width() == 64 => v,
        BasicValueEnum::IntValue(v) => builder
            .build_int_z_extend(v, i64_type, "closure_word")
            .expect("build_int_z_extend should not fail widening to i64"),
        BasicValueEnum::FloatValue(v) => builder
            .build_bit_cast(v, i64_type, "closure_word")
            .expect("build_bit_cast should not fail for an f64")
            .into_int_value(),
        BasicValueEnum::PointerValue(v) => builder
            .build_ptr_to_int(v, i64_type, "closure_word")
            .expect("build_ptr_to_int should not fail for a pointer"),
        other => {
            panic!("pycc_codegen: internal error: a captured value is not one word: {other:?}")
        }
    }
}

/// The value of LLVM type `ty` a word made by [`to_word`] holds.
fn from_word<'ctx>(
    builder: &inkwell::builder::Builder<'ctx>,
    ty: inkwell::types::BasicTypeEnum<'ctx>,
    word: IntValue<'ctx>,
) -> BasicValueEnum<'ctx> {
    match ty {
        inkwell::types::BasicTypeEnum::IntType(t) if t.get_bit_width() == 64 => word.into(),
        inkwell::types::BasicTypeEnum::IntType(t) => builder
            .build_int_truncate(word, t, "closure_value")
            .expect("build_int_truncate should not fail narrowing an i64")
            .into(),
        inkwell::types::BasicTypeEnum::FloatType(t) => builder
            .build_bit_cast(word, t, "closure_value")
            .expect("build_bit_cast should not fail for an i64"),
        inkwell::types::BasicTypeEnum::PointerType(t) => builder
            .build_int_to_ptr(word, t, "closure_value")
            .expect("build_int_to_ptr should not fail for an i64")
            .into(),
        other => {
            panic!("pycc_codegen: internal error: a captured value is not one word: {other:?}")
        }
    }
}

/// Takes (`retain`) or drops a reference to a captured value of type `ty`.
fn refcount<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    rt: &RtFns<'ctx>,
    ty: &Ty,
    value: BasicValueEnum<'ctx>,
    retain: bool,
) {
    if *ty == Ty::Int {
        bigint_rc::emit_bigint_refcount_call(
            context,
            builder,
            rt,
            value.into_int_value(),
            if retain {
                bigint_rc::BigIntRefcount::Retain
            } else {
                bigint_rc::BigIntRefcount::Release
            },
        );
    } else if let Some(kind) = heap_rc::heap_kind(rt, ty) {
        let ptr = value.into_pointer_value();
        if retain {
            heap_rc::emit_incref(context, builder, rt, kind, ptr);
        } else {
            heap_rc::emit_decref(context, builder, rt, kind, ptr);
        }
    }
}

/// `pycc_closure_call.F` and `pycc_closure_drop.F` for `function` with
/// `captures` bound parameters.
fn closure_functions<'ctx>(
    context: &'ctx Context,
    module: &inkwell::module::Module<'ctx>,
    rt: &RtFns<'ctx>,
    user_function: &UserFunction<'ctx>,
    function: &str,
    captures: usize,
) -> (FunctionValue<'ctx>, FunctionValue<'ctx>) {
    let call_name = format!("pycc_closure_call.{function}");
    let drop_name = format!("pycc_closure_drop.{function}");
    if let (Some(call), Some(drop)) = (
        module.get_function(&call_name),
        module.get_function(&drop_name),
    ) {
        return (call, drop);
    }
    let ptr_type = context.ptr_type(inkwell::AddressSpace::default());
    let params = user_function.fn_type.get_param_types();
    let mut call_params: Vec<inkwell::types::BasicMetadataTypeEnum<'ctx>> = vec![ptr_type.into()];
    call_params.extend(params[captures..].iter().copied());
    let call_type = match user_function.fn_type.get_return_type() {
        Some(ret) => ret.fn_type(&call_params, false),
        None => context.void_type().fn_type(&call_params, false),
    };
    let call = module.add_function(
        &call_name,
        call_type,
        Some(inkwell::module::Linkage::Private),
    );
    let builder = context.create_builder();
    builder.position_at_end(context.append_basic_block(call, "entry"));
    let obj = call
        .get_nth_param(0)
        .expect("the call function takes its object")
        .into_pointer_value();
    let mut args: Vec<BasicMetadataValueEnum<'ctx>> = Vec::new();
    for (index, ty) in user_function.param_tys[..captures].iter().enumerate() {
        let word = builder
            .build_load(
                context.i64_type(),
                capture_slot(context, &builder, obj, index),
                "word",
            )
            .expect("build_load should not fail for a captured word")
            .into_int_value();
        let value = from_word(&builder, ty_to_basic_type(context, ty.clone()), word);
        // The callee's parameter slot owns its argument.
        refcount(context, &builder, rt, ty, value, true);
        args.push(value.into());
    }
    for index in 0..params.len() - captures {
        args.push(
            call.get_nth_param(index as u32 + 1)
                .expect("the call function takes every remaining parameter")
                .into(),
        );
    }
    let result = crate::user_call::dispatch(context, &builder, rt, user_function, &args);
    match result.try_as_basic_value().basic() {
        Some(value) => builder.build_return(Some(&value)),
        None => builder.build_return(None),
    }
    .expect("build_return should not fail");

    let drop = module.add_function(
        &drop_name,
        context.void_type().fn_type(&[ptr_type.into()], false),
        Some(inkwell::module::Linkage::Private),
    );
    builder.position_at_end(context.append_basic_block(drop, "entry"));
    let obj = drop
        .get_nth_param(0)
        .expect("the drop function takes its object")
        .into_pointer_value();
    for (index, ty) in user_function.param_tys[..captures].iter().enumerate() {
        let word = builder
            .build_load(
                context.i64_type(),
                capture_slot(context, &builder, obj, index),
                "word",
            )
            .expect("build_load should not fail for a captured word")
            .into_int_value();
        let value = from_word(&builder, ty_to_basic_type(context, ty.clone()), word);
        refcount(context, &builder, rt, ty, value, false);
    }
    builder
        .build_return(None)
        .expect("build_return should not fail");
    (call, drop)
}

/// `MirExpr::MakeClosure`: a new function value owning its captured values.
#[allow(clippy::too_many_arguments)]
pub(crate) fn emit_make_closure<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    module: &inkwell::module::Module<'ctx>,
    rt: &RtFns<'ctx>,
    user_functions: &HashMap<&str, UserFunction<'ctx>>,
    locals: &HashMap<String, StorageSlot<'ctx>>,
    function: &str,
    captures: &[MirExpr],
) -> Scalar<'ctx> {
    let user_function = &user_functions[function];
    // Each captured value as a reference the object owns.
    let values = crate::user_call::marshal_args(
        context,
        builder,
        module,
        rt,
        user_functions,
        locals,
        &user_function.param_tys[..captures.len()],
        function,
        captures,
    );
    let (call, drop) =
        closure_functions(context, module, rt, user_function, function, captures.len());
    let obj = builder
        .build_call(
            rt.closures.new,
            &[
                call.as_global_value().as_pointer_value().into(),
                drop.as_global_value().as_pointer_value().into(),
                context
                    .i64_type()
                    .const_int(captures.len() as u64, false)
                    .into(),
            ],
            "closure",
        )
        .expect("build_call should not fail for pycc_rt_closure_new")
        .try_as_basic_value()
        .expect_basic("pycc_rt_closure_new returns a pointer")
        .into_pointer_value();
    for (index, value) in values.into_iter().enumerate() {
        let value =
            BasicValueEnum::try_from(value).expect("a captured value is a first-class LLVM value");
        let word = to_word(context, builder, value);
        builder
            .build_store(capture_slot(context, builder, obj, index), word)
            .expect("build_store should not fail for a captured word");
    }
    Scalar::Closure(obj)
}

/// `MirExpr::CallClosure`: `callee(args)` through the value's call function.
#[allow(clippy::too_many_arguments)]
pub(crate) fn emit_call_closure<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    module: &inkwell::module::Module<'ctx>,
    rt: &RtFns<'ctx>,
    user_functions: &HashMap<&str, UserFunction<'ctx>>,
    locals: &HashMap<String, StorageSlot<'ctx>>,
    callee: &MirExpr,
    args: &[MirExpr],
    ty: &Ty,
) -> Scalar<'ctx> {
    let Ty::Callable(sig) = callee.ty() else {
        panic!("pycc_codegen: internal error: a called value is not a function value")
    };
    let Scalar::Closure(obj) =
        emit_expr(context, builder, module, rt, user_functions, locals, callee)
    else {
        panic!("pycc_codegen: internal error: a called value is not a function value")
    };
    // A fresh value (`make_adder(1)(2)`) is released after the call, or if
    // an argument raises.
    let mark = heap_rc::pending_mark(rt);
    let fresh = !heap_rc::is_borrowed_read(callee);
    if fresh {
        heap_rc::hold_owned(rt, heap_rc::HeapKind::Closure, obj);
    }
    let mut values: Vec<BasicMetadataValueEnum<'ctx>> = vec![obj.into()];
    values.extend(crate::user_call::marshal_args(
        context,
        builder,
        module,
        rt,
        user_functions,
        locals,
        &sig.0,
        "function",
        args,
    ));
    let ptr_type = context.ptr_type(inkwell::AddressSpace::default());
    let mut param_types: Vec<inkwell::types::BasicMetadataTypeEnum<'ctx>> = vec![ptr_type.into()];
    param_types.extend(
        sig.0
            .iter()
            .map(|t| -> inkwell::types::BasicMetadataTypeEnum<'ctx> {
                ty_to_basic_type(context, t.clone()).into()
            }),
    );
    let fn_type = match &sig.1 {
        Ty::None => context.void_type().fn_type(&param_types, false),
        other => ty_to_basic_type(context, other.clone()).fn_type(&param_types, false),
    };
    // SAFETY: `CALL_OFFSET` is inside the object's header.
    let call_slot = unsafe {
        builder
            .build_gep(
                context.i8_type(),
                obj,
                &[context.i64_type().const_int(CALL_OFFSET, false)],
                "closure_call_slot",
            )
            .expect("build_gep should not fail for a byte offset")
    };
    let call = builder
        .build_load(ptr_type, call_slot, "closure_call")
        .expect("build_load should not fail for the call function")
        .into_pointer_value();
    let call_site = builder
        .build_indirect_call(fn_type, call, &values, "call_closure")
        .expect("build_indirect_call should not fail for a function value");
    if fresh {
        heap_rc::truncate_pending(rt, mark);
        heap_rc::emit_decref(context, builder, rt, heap_rc::HeapKind::Closure, obj);
    }
    crate::call_result::call_result_scalar(context, call_site, ty)
}
