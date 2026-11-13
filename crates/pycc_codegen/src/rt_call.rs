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
        Scalar::Generator(v) | Scalar::Closure(v) => v.into(),
        Scalar::Int(v) | Scalar::Num(v) | Scalar::Bool(v) => v.into(),
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

/// `__pycc_region_intact(k)`'s symbol (`pycc_hir::builtins`), which has no
/// definition: codegen reads the flag itself ([`emit_region_intact`]).
pub(crate) const REGION_INTACT_SYMBOL: &str = "pycc_ext_region_intact";

/// The flags of an `ext` module's regions of selected functions, one byte
/// each, in the module's unit (`src/ext/module/module_ctx.c`).
pub const EXT_REGION_FLAGS_SYMBOL: &str = "pycc_ext_region_flags";

/// The value of an intact region's flag (`PYCC_REGION_INTACT`,
/// `src/ext/module/regions.c`); a region still pending has another.
const REGION_INTACT: u64 = 1;

/// The most regions a module arms (`PYCC_EXT_REGION_MAX`).
const REGION_MAX: i64 = 64;

/// `__pycc_region_intact(k)`: `pycc_ext_region_flags[k] == 1`, a load and a
/// compare where the call stands. The flag is a plain byte: a call that may
/// run Python, and so a watcher, makes LLVM read it again.
fn emit_region_intact<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    module: &inkwell::module::Module<'ctx>,
    call: &RtCallExpr,
) -> Scalar<'ctx> {
    let k = match call.args.as_slice() {
        [MirExpr::IntLiteral(k)] if (0..REGION_MAX).contains(k) => *k as u64,
        _ => panic!("pycc_codegen: internal error: a region's index is a literal below {REGION_MAX}"),
    };
    let i8_type = context.i8_type();
    let flags_type = i8_type.array_type(REGION_MAX as u32);
    let flags = module.get_global(EXT_REGION_FLAGS_SYMBOL).unwrap_or_else(|| {
        let global = module.add_global(flags_type, None, EXT_REGION_FLAGS_SYMBOL);
        global.set_linkage(inkwell::module::Linkage::External);
        global
    });
    let i64_type = context.i64_type();
    // SAFETY: `k` is below the array's length, checked above.
    let slot = unsafe {
        builder.build_in_bounds_gep(
            flags_type,
            flags.as_pointer_value(),
            &[i64_type.const_zero(), i64_type.const_int(k, false)],
            "region_flag",
        )
    }
    .expect("build_in_bounds_gep should not fail for a region flag");
    let flag = builder
        .build_load(i8_type, slot, "region_flag_value")
        .expect("build_load should not fail for a region flag")
        .into_int_value();
    let intact = builder
        .build_int_compare(
            inkwell::IntPredicate::EQ,
            flag,
            i8_type.const_int(REGION_INTACT, false),
            "region_intact",
        )
        .expect("build_int_compare should not fail for a region flag");
    let bool_type = ty_to_basic_type(context, Ty::Bool).into_int_type();
    Scalar::Bool(
        builder
            .build_int_z_extend_or_bit_cast(intact, bool_type, "region_intact_bool")
            .expect("build_int_z_extend_or_bit_cast should not fail for a bool"),
    )
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
    if call.sig.symbol == REGION_INTACT_SYMBOL {
        return emit_region_intact(context, builder, module, call);
    }
    if let Some(result) = crate::extend_scratch::emit(context, builder, module, rt, user_functions, locals, call) {
        return result;
    }
    let sig = &call.sig;
    let i64_type = context.i64_type();
    let mut values = Vec::with_capacity(call.args.len());
    let mut param_types: Vec<BasicMetadataTypeEnum<'ctx>> = Vec::with_capacity(call.args.len());
    let mut temporaries = Vec::new();
    let mut boxes = Vec::new();
    let passes_through = sig.symbol == "pycc_rt_list_unpack";
    // A fresh argument is released if a later argument raises.
    let mark = heap_rc::pending_mark(rt);
    let mut pending_ints = Vec::new();
    let mut coerced_strs = Vec::new();
    let mut converted_nums = Vec::new();
    // A produced CPython object argument (a shim helper's, boxed by
    // `pycc_mir`) is held across the call and released after it.
    let mut held_objects = Vec::new();
    for ((arg, param), is_word) in call.args.iter().zip(&sig.params).zip(&sig.word_args) {
        // Non-strict mode: a CPython object for a scalar parameter
        // (`pycc_hir::builtins::object_param`) is converted with a run-time
        // type check; a converted `str` is a new reference released after
        // the call.
        if let Some(what) = pycc_hir::builtins::object_param_subject(sig.symbol)
            && crate::object_coerce::coerces_into(arg, param)
        {
            let scalar = crate::object_coerce::emit_coerced(
                context,
                builder,
                module,
                rt,
                user_functions,
                locals,
                arg,
                param,
                what,
            );
            if let Scalar::Str(ptr) = scalar {
                coerced_strs.push(ptr);
            }
            values.push(abi_value(scalar));
            param_types.push(ty_to_basic_type(context, param.clone()).into());
            continue;
        }
        let scalar = emit_expr(context, builder, module, rt, user_functions, locals, arg);
        if let Scalar::Object(_) = scalar {
            held_objects.push(crate::object_release::hold(
                context, module, rt, arg, &scalar,
            ));
        }
        // #625: a fresh `int` argument (`str(x * 2)`) is released after the
        // call, or if a later argument raises.
        if !passes_through {
            pending_ints.push(bigint_rc::push_pending_int_release_if_scalar_temporary(
                rt, arg, &scalar,
            ));
        }
        let scalar = if passes_through {
            // The result is this very list: it carries the reference.
            heap_rc::retain_if_heap_duplicate(context, builder, rt, arg, scalar)
        } else {
            heap_rc::hold_temporary(rt, arg, &scalar);
            temporaries.push((arg, scalar));
            scalar
        };
        if *is_word {
            // A `float` for an `int | float` word is a fresh word, released
            // after the call.
            let scalar = match (param, scalar) {
                (Ty::Num, scalar @ Scalar::Float(_)) => {
                    let (word, converted) = crate::num::operand_word(context, builder, rt, scalar);
                    converted_nums.push(converted);
                    Scalar::Num(word)
                }
                (_, scalar) => coerce_scalar_to_type(context, builder, rt, scalar, param.clone()),
            };
            // A runtime call borrows its word argument: one it stores takes
            // its own reference (`pycc_rt::value::retain`).
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
                // `float(x)` of an `int | float` (`OverflowError` for a huge
                // `int`).
                (Ty::Float, Scalar::Num(word)) => {
                    Scalar::Float(crate::num::emit_num_to_float(context, builder, rt, word, true))
                }
                // A `float` for an `int | float` parameter is a fresh word,
                // released after the call.
                (Ty::Num, scalar @ Scalar::Float(_)) => {
                    let (word, converted) = crate::num::operand_word(context, builder, rt, scalar);
                    converted_nums.push(converted);
                    Scalar::Num(word)
                }
                (_, scalar) => coerce_scalar_to_type(context, builder, rt, scalar, param.clone()),
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
    for desc in &sig.desc_args {
        values.push(
            i64_type
                .const_int(crate::containers::desc_word(rt, desc), false)
                .into(),
        );
        param_types.push(i64_type.into());
    }
    // A struct tuple comes back from the runtime boxed.
    let boxed_ret = !sig.ret_word
        && matches!(&sig.ret, Ty::Tuple(elements) if !pycc_hir::tuple_is_boxed(elements));
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
    // `tuple()` of a list only the call holds, `sorted()` of a generator
    // expression's list: their own helpers (`object_tuple::fresh_symbol`).
    let symbol = crate::object_tuple::fresh_symbol(call).unwrap_or(sig.symbol);
    let function = module.get_function(symbol).unwrap_or_else(|| {
        module.add_function(
            symbol,
            fn_type,
            Some(inkwell::module::Linkage::External),
        )
    });
    let call_site = builder
        .build_call(function, &values, "rt_call")
        .expect("build_call should not fail for a declared runtime builtin");
    heap_rc::truncate_pending(rt, mark);
    for pushed in pending_ints.into_iter().rev() {
        bigint_rc::pop_pending_int_release(rt, pushed);
    }
    // The runtime borrows its arguments, and an `int` result is always a
    // reference of its own (`pycc_rt_int_max2` retains the argument it
    // answers), so every fresh argument dies here.
    for (arg, scalar) in temporaries {
        match scalar {
            Scalar::Int(word) => {
                bigint_rc::release_if_int_temporary(context, builder, rt, arg, word);
            }
            _ => heap_rc::release_scalar_if_temporary(context, builder, rt, arg, &scalar),
        }
    }
    for boxed in boxes {
        crate::containers::release_boxed(context, builder, rt, boxed);
    }
    crate::num::release_converted(context, builder, rt, &converted_nums);
    for held in held_objects.into_iter().rev() {
        held.release(builder, rt);
    }
    for ptr in coerced_strs {
        heap_rc::emit_decref(context, builder, rt, heap_rc::HeapKind::Str, ptr);
    }
    if boxed_ret {
        let Ty::Tuple(elements) = &sig.ret else {
            unreachable!("boxed_ret is a tuple result")
        };
        let boxed = call_site
            .try_as_basic_value()
            .expect_basic("a tuple-returning builtin returns a pointer")
            .into_pointer_value();
        let tuple = crate::containers::unbox_struct_tuple(context, builder, rt, elements, boxed);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_region_flag_is_loaded_and_compared_where_it_is_read() {
        let context = Context::create();
        let module = context.create_module("m");
        let builder = context.create_builder();
        let f = module.add_function("f", context.void_type().fn_type(&[], false), None);
        builder.position_at_end(context.append_basic_block(f, "entry"));
        let call = RtCallExpr {
            sig: pycc_hir::builtins::resolve_call("__pycc_region_intact", &[Ty::Int])
                .expect("claimed")
                .expect("admitted"),
            args: vec![MirExpr::IntLiteral(3)],
        };
        assert_eq!(call.sig.symbol, REGION_INTACT_SYMBOL);
        let Scalar::Bool(_) = emit_region_intact(&context, &builder, &module, &call) else {
            panic!("a bool")
        };
        let ir = crate::compile::llvm_string_to_owned(module.print_to_string());
        assert!(ir.contains("@pycc_ext_region_flags = external global [64 x i8]"), "{ir}");
        assert!(ir.contains("load i8, ptr getelementptr inbounds ([64 x i8], ptr @pycc_ext_region_flags, i64 0, i64 3)"), "{ir}");
        assert!(ir.contains("icmp eq i8"), "{ir}");
        assert!(!ir.contains("call"), "{ir}");
    }

    #[test]
    #[should_panic(expected = "a region's index is a literal below 64")]
    fn a_region_index_out_of_range_is_an_internal_error() {
        let context = Context::create();
        let module = context.create_module("m");
        let builder = context.create_builder();
        let f = module.add_function("f", context.void_type().fn_type(&[], false), None);
        builder.position_at_end(context.append_basic_block(f, "entry"));
        let call = RtCallExpr {
            sig: pycc_hir::builtins::resolve_call("__pycc_region_intact", &[Ty::Int])
                .expect("claimed")
                .expect("admitted"),
            args: vec![MirExpr::IntLiteral(64)],
        };
        emit_region_intact(&context, &builder, &module, &call);
    }
}

