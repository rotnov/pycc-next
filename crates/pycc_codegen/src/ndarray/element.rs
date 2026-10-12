//! Loading, converting and storing typed array elements.

use super::*;

pub(super) fn untag_word<'ctx>(
    context: &'ctx Context,
    builder: &Builder<'ctx>,
    module: &inkwell::module::Module<'ctx>,
    rt: &RtFns<'ctx>,
    word: IntValue<'ctx>,
    slow_symbol: &str,
    label: &str,
) -> IntValue<'ctx> {
    let i64_type = context.i64_type();
    if let Some(constant) = word.get_sign_extended_constant()
        && constant & 1 == 1
    {
        return i64_type.const_int((constant >> 1) as u64, true);
    }
    let edge = ForeignFailEdge::for_current(builder);
    let function = edge.function();
    let small_bb = context.append_basic_block(function, &format!("{label}_small"));
    let slow_bb = context.append_basic_block(function, &format!("{label}_slow"));
    let join_bb = context.append_basic_block(function, &format!("{label}_join"));
    let low = builder
        .build_and(word, i64_type.const_int(1, false), "nd_low")
        .expect("build_and should not fail");
    let is_small = builder
        .build_int_compare(IntPredicate::NE, low, i64_type.const_zero(), "nd_is_small")
        .expect("build_int_compare should not fail");
    crate::int_fast::likely_branch(context, builder, is_small, small_bb, slow_bb);
    builder.position_at_end(small_bb);
    let small = builder
        .build_right_shift(word, i64_type.const_int(1, false), true, "nd_untag")
        .expect("build_right_shift should not fail");
    builder
        .build_unconditional_branch(join_bb)
        .expect("br should not fail");
    builder.position_at_end(slow_bb);
    let out = crate::build_at_entry_block(builder, function, |b| {
        b.build_alloca(i64_type, "nd_word_out")
            .expect("build_alloca should not fail")
    });
    let ptr = context.ptr_type(inkwell::AddressSpace::default());
    let slow = shim_fn(
        module,
        slow_symbol,
        context
            .i32_type()
            .fn_type(&[i64_type.into(), ptr.into()], false),
    );
    let status = builder
        .build_call(slow, &[word.into(), out.into()], label)
        .expect("build_call should not fail")
        .try_as_basic_value()
        .expect_basic("an nd word helper returns int")
        .into_int_value();
    route_negative(context, builder, module, rt, edge, status, label);
    let decoded = builder
        .build_load(i64_type, out, "nd_word")
        .expect("build_load should not fail")
        .into_int_value();
    let slow_end = builder.get_insert_block().expect("positioned");
    builder
        .build_unconditional_branch(join_bb)
        .expect("br should not fail");
    builder.position_at_end(join_bb);
    let phi = builder
        .build_phi(i64_type, label)
        .expect("build_phi should not fail");
    phi.add_incoming(&[(&small, small_bb), (&decoded, slow_end)]);
    phi.as_basic_value().into_int_value()
}

pub(super) fn load_element<'ctx>(
    context: &'ctx Context,
    builder: &Builder<'ctx>,
    rt: &RtFns<'ctx>,
    address: PointerValue<'ctx>,
    dtype: NdDtype,
) -> Scalar<'ctx> {
    let element_ty: inkwell::types::BasicTypeEnum<'ctx> = match dtype {
        NdDtype::F64 => context.f64_type().into(),
        NdDtype::I64 => context.i64_type().into(),
        NdDtype::Bool => context.i8_type().into(),
    };
    let load = builder
        .build_load(element_ty, address, "nd_load")
        .expect("build_load should not fail");
    if let Some(instruction) = load.as_instruction_value() {
        instruction
            .set_alignment(1)
            .expect("a load accepts an alignment");
        crate::tbaa::tag(context, instruction, TAG_ELEMENT);
    }
    match dtype {
        NdDtype::F64 => Scalar::Float(load.into_float_value()),
        NdDtype::I64 => Scalar::Int(tag_i64(context, builder, rt, load.into_int_value())),
        NdDtype::Bool => {
            let nonzero = builder
                .build_int_compare(
                    IntPredicate::NE,
                    load.into_int_value(),
                    context.i8_type().const_zero(),
                    "nd_bool",
                )
                .expect("build_int_compare should not fail");
            Scalar::Bool(
                builder
                    .build_int_z_extend(nonzero, context.i8_type(), "nd_bool_byte")
                    .expect("build_int_z_extend should not fail"),
            )
        }
    }
}

/// Encodes a machine `int64` as an `int` word: a smallint inline when it
/// fits 63 bits, a bigint through `pycc_rt_int_from_i64` otherwise.
pub(super) fn tag_i64<'ctx>(
    context: &'ctx Context,
    builder: &Builder<'ctx>,
    rt: &RtFns<'ctx>,
    value: IntValue<'ctx>,
) -> IntValue<'ctx> {
    let i64_type = context.i64_type();
    let function = builder
        .get_insert_block()
        .and_then(|b| b.get_parent())
        .expect("inside a function");
    let one = i64_type.const_int(1, false);
    let shifted = builder.build_left_shift(value, one, "nd_shl").expect("shl");
    let back = builder
        .build_right_shift(shifted, one, true, "nd_sar")
        .expect("sar");
    let fits = builder
        .build_int_compare(IntPredicate::EQ, back, value, "nd_fits")
        .expect("build_int_compare should not fail");
    let small_bb = context.append_basic_block(function, "nd_tag_small");
    let big_bb = context.append_basic_block(function, "nd_tag_big");
    let join_bb = context.append_basic_block(function, "nd_tag_join");
    crate::int_fast::likely_branch(context, builder, fits, small_bb, big_bb);
    builder.position_at_end(small_bb);
    let small = builder.build_or(shifted, one, "nd_tagged").expect("or");
    builder.build_unconditional_branch(join_bb).expect("br");
    builder.position_at_end(big_bb);
    let big = builder
        .build_call(rt.int_from_i64, &[value.into()], "nd_bigint")
        .expect("build_call should not fail")
        .try_as_basic_value()
        .expect_basic("pycc_rt_int_from_i64 returns i64")
        .into_int_value();
    builder.build_unconditional_branch(join_bb).expect("br");
    builder.position_at_end(join_bb);
    let phi = builder.build_phi(i64_type, "nd_int").expect("phi");
    phi.add_incoming(&[(&small, small_bb), (&big, big_bb)]);
    phi.as_basic_value().into_int_value()
}

pub(super) fn emit_writeable_check<'ctx>(
    context: &'ctx Context,
    builder: &Builder<'ctx>,
    module: &inkwell::module::Module<'ctx>,
    rt: &RtFns<'ctx>,
    header: &Header<'ctx>,
) {
    let flags = header.flags(context, builder);
    let i32_type = context.i32_type();
    let bit = builder
        .build_and(flags, i32_type.const_int(WRITEABLE, false), "nd_writeable")
        .expect("build_and should not fail");
    let read_only = builder
        .build_int_compare(IntPredicate::EQ, bit, i32_type.const_zero(), "nd_read_only")
        .expect("build_int_compare should not fail");
    cold_raise(
        context,
        builder,
        module,
        rt,
        read_only,
        "nd_readonly",
        |b| {
            let raise = shim_fn(module, ND_READONLY_SYMBOL, i32_type.fn_type(&[], false));
            b.build_call(raise, &[], "nd_readonly")
                .expect("build_call should not fail");
        },
    );
}

#[allow(clippy::too_many_arguments)]
pub(super) fn store_element<'ctx>(
    context: &'ctx Context,
    builder: &Builder<'ctx>,
    module: &inkwell::module::Module<'ctx>,
    rt: &RtFns<'ctx>,
    address: PointerValue<'ctx>,
    dtype: NdDtype,
    value_expr: &MirExpr,
    value: Scalar<'ctx>,
) {
    let stored: inkwell::values::BasicValueEnum<'ctx> = match dtype {
        NdDtype::F64 => {
            let word = match value {
                Scalar::Int(word) => Some(word),
                _ => None,
            };
            let float = to_float(context, builder, rt, value);
            if let Some(word) = word {
                release_if_int_temporary(context, builder, rt, value_expr, word);
            }
            float.into()
        }
        NdDtype::I64 => match value {
            Scalar::Bool(byte) => builder
                .build_int_z_extend(byte, context.i64_type(), "nd_bool_word")
                .expect("build_int_z_extend should not fail")
                .into(),
            other => {
                let word = to_numeric_encoded_int(context, builder, other);
                let raw = untag_word(
                    context,
                    builder,
                    module,
                    rt,
                    word,
                    ND_INT_WORD_SYMBOL,
                    "nd_int_value",
                );
                release_if_int_temporary(context, builder, rt, value_expr, word);
                raw.into()
            }
        },
        NdDtype::Bool => match value {
            Scalar::Bool(byte) => byte.into(),
            _ => panic!("pycc_codegen: internal error: a bool array stores a non-bool value"),
        },
    };
    let store = builder
        .build_store(address, stored)
        .expect("build_store should not fail");
    store
        .set_alignment(1)
        .expect("a store accepts an alignment");
    crate::tbaa::tag(context, store, TAG_ELEMENT);
}
