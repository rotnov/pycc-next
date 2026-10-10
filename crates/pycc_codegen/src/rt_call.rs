//! Emission of [`MirExpr::RtCall`]: a builtin lowered to a direct call into
//! `pycc_rt` (`pycc_hir::builtins` states the ABI).
//!
//! Arguments are borrowed by the runtime. An argument that is a fresh
//! counted value (an owning temporary, see `heap_rc`) is released after the
//! call, so `int(input())` does not leak the line it parsed. A returned
//! counted value is a new reference the caller owns -- except
//! `pycc_rt_list_unpack`'s, which is its argument handed back, so that
//! argument's reference becomes the result's.

use super::*;
use inkwell::types::BasicMetadataTypeEnum;
use inkwell::values::BasicMetadataValueEnum;
use pycc_mir::{RtCallExpr, Ty};

/// The value `scalar` (already coerced to `ty`) passes as.
fn abi_value<'ctx>(scalar: Scalar<'ctx>) -> BasicMetadataValueEnum<'ctx> {
    match scalar {
        Scalar::Int(v) | Scalar::Bool(v) => v.into(),
        Scalar::Float(v) => v.into(),
        Scalar::Str(v)
        | Scalar::List(v)
        | Scalar::Dict(v)
        | Scalar::Set(v)
        | Scalar::Instance(v)
        | Scalar::Object(v) => v.into(),
        Scalar::Tuple(v) | Scalar::Optional(v) => v.into(),
        Scalar::MemoryView(_) => {
            panic!("pycc_codegen: internal error: a memoryview is never a builtin argument")
        }
    }
}

pub(super) fn emit_rt_call<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    module: &inkwell::module::Module<'ctx>,
    rt: &RtFns<'ctx>,
    user_functions: &HashMap<&str, UserFunction<'ctx>>,
    locals: &HashMap<String, StorageSlot<'ctx>>,
    call: &RtCallExpr,
) -> Scalar<'ctx> {
    let sig = &call.sig;
    let i64_type = context.i64_type();
    let mut values = Vec::with_capacity(call.args.len());
    let mut param_types: Vec<BasicMetadataTypeEnum<'ctx>> = Vec::with_capacity(call.args.len());
    let mut temporaries = Vec::new();
    let mut boxes = Vec::new();
    let passes_through = sig.symbol == "pycc_rt_list_unpack";
    // A fresh argument is released if a later argument raises.
    let mark = heap_rc::pending_mark(rt);
    for ((arg, param), is_word) in call.args.iter().zip(&sig.params).zip(&sig.word_args) {
        let scalar = emit_expr(context, builder, module, rt, user_functions, locals, arg);
        let scalar = if passes_through {
            // The result is this very list: it carries the reference.
            heap_rc::retain_if_heap_duplicate(context, builder, rt, arg, scalar)
        } else {
            heap_rc::hold_temporary(rt, arg, &scalar);
            temporaries.push((arg, scalar));
            scalar
        };
        if *is_word {
            let scalar = coerce_scalar_to_type(context, builder, scalar, param.clone());
            let (word, boxed) =
                crate::containers::borrowed_word(context, builder, rt, param, scalar);
            boxes.push(boxed);
            values.push(word.into());
            param_types.push(i64_type.into());
        } else {
            // An `int` passed to a `float` parameter (`math.sqrt(4)`) is
            // converted as `float(n)` would (`OverflowError` beyond range).
            let scalar = match (param, scalar) {
                (Ty::Float, scalar @ (Scalar::Int(_) | Scalar::Bool(_))) => {
                    Scalar::Float(crate::coerce::to_float(context, builder, rt, scalar))
                }
                (_, scalar) => coerce_scalar_to_type(context, builder, scalar, param.clone()),
            };
            values.push(abi_value(scalar));
            param_types.push(ty_to_basic_type(context, param.clone()).into());
        }
    }
    for kind in &sig.kind_args {
        values.push(
            i64_type
                .const_int(crate::containers::word_kind(rt, kind), false)
                .into(),
        );
        param_types.push(i64_type.into());
    }
    // A struct tuple comes back from the runtime boxed.
    let boxed_ret = matches!(&sig.ret, Ty::Tuple(elements) if !pycc_hir::tuple_is_boxed(elements));
    let fn_type = if boxed_ret {
        context
            .ptr_type(inkwell::AddressSpace::default())
            .fn_type(&param_types, false)
    } else if sig.ret_word {
        i64_type.fn_type(&param_types, false)
    } else if sig.ret == Ty::None {
        context.void_type().fn_type(&param_types, false)
    } else {
        ty_to_basic_type(context, sig.ret.clone()).fn_type(&param_types, false)
    };
    let function = module.get_function(sig.symbol).unwrap_or_else(|| {
        module.add_function(
            sig.symbol,
            fn_type,
            Some(inkwell::module::Linkage::External),
        )
    });
    let call_site = builder
        .build_call(function, &values, "rt_call")
        .expect("build_call should not fail for a declared runtime builtin");
    heap_rc::truncate_pending(rt, mark);
    for (arg, scalar) in temporaries {
        if !matches!(scalar, Scalar::Int(_)) {
            heap_rc::release_scalar_if_temporary(context, builder, rt, arg, &scalar);
        }
    }
    for boxed in boxes {
        crate::containers::release_boxed(context, builder, rt, boxed);
    }
    if boxed_ret {
        let Ty::Tuple(elements) = &sig.ret else {
            unreachable!("boxed_ret is a tuple result")
        };
        let boxed = call_site
            .try_as_basic_value()
            .expect_basic("a tuple-returning builtin returns a pointer")
            .into_pointer_value();
        let tuple = crate::containers::unbox_struct_tuple(context, builder, elements, boxed);
        heap_rc::emit_decref(context, builder, rt, heap_rc::HeapKind::List, boxed);
        return Scalar::Tuple(tuple);
    }
    if sig.ret_word {
        let word = call_site
            .try_as_basic_value()
            .expect_basic("a word-returning builtin returns i64")
            .into_int_value();
        return crate::containers::owned_word_element(context, builder, rt, &sig.ret, word);
    }
    crate::call_result::call_result_scalar(context, call_site, &sig.ret)
}
