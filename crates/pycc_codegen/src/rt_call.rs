//! Emission of [`MirExpr::RtCall`]: a builtin lowered to a direct call into
//! `pycc_rt` (`pycc_hir::builtins` states the ABI).
//!
//! Arguments are borrowed by the runtime. An argument that is a freshly
//! built `str` (an owning temporary, see `str_rc`) is released after the
//! call, so `int(input())` does not leak the line it parsed.

use super::*;
use crate::attr_slot::scalar_to_slot_word;
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

/// The list element word for `scalar` as an element of type `elem`,
/// borrowed (no retain).
fn borrowed_word<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    elem: &Ty,
    scalar: Scalar<'ctx>,
) -> IntValue<'ctx> {
    match elem {
        Ty::Int => to_encoded_int(context, builder, scalar),
        _ => scalar_to_slot_word(context, builder, scalar),
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
    let mut owned_strs = Vec::new();
    for ((arg, param), is_word) in call.args.iter().zip(&sig.params).zip(&sig.word_args) {
        let scalar = emit_expr(context, builder, module, rt, user_functions, locals, arg);
        if let Scalar::Str(ptr) = scalar
            && !str_rc::str_value_is_a_duplicate_reference(arg)
        {
            owned_strs.push(ptr);
        }
        if *is_word {
            let scalar = coerce_scalar_to_type(context, builder, scalar, param.clone());
            values.push(borrowed_word(context, builder, param, scalar).into());
            param_types.push(i64_type.into());
        } else {
            let scalar = coerce_scalar_to_type(context, builder, scalar, param.clone());
            values.push(abi_value(scalar));
            param_types.push(ty_to_basic_type(context, param.clone()).into());
        }
    }
    let fn_type = if sig.ret_word {
        i64_type.fn_type(&param_types, false)
    } else if sig.ret == Ty::None {
        context.void_type().fn_type(&param_types, false)
    } else {
        ty_to_basic_type(context, sig.ret.clone()).fn_type(&param_types, false)
    };
    let function = module.get_function(sig.symbol).unwrap_or_else(|| {
        module.add_function(sig.symbol, fn_type, Some(inkwell::module::Linkage::External))
    });
    let call_site = builder
        .build_call(function, &values, "rt_call")
        .expect("build_call should not fail for a declared runtime builtin");
    for ptr in owned_strs {
        builder
            .build_call(rt.str_decref, &[ptr.into()], "")
            .expect("build_call should not fail for a well-formed decref");
    }
    if sig.ret_word {
        let word = call_site
            .try_as_basic_value()
            .expect_basic("a word-returning builtin returns i64")
            .into_int_value();
        return list::word_element(context, builder, &sig.ret, word);
    }
    crate::call_result::call_result_scalar(context, call_site, &sig.ret)
}
