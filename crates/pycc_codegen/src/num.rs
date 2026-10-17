//! `int | float` values (`Ty::Num`, `pycc_rt::num`): the runtime
//! functions, the conversions to and from the word, and the operators.
//!
//! The word extends the `int` word: an `int` is its own `int` word, so an
//! `int` converts to `int | float` (and back, once it is known to be one)
//! with no instruction, and every `int` refcount path -- a slot's release
//! before a store, a temporary's release, the exception edges' pending
//! releases -- serves the word unchanged (`bigint_rc`). A `float` is
//! inline in the patterns ending in binary `10` for every magnitude a
//! program normally meets, and a counted heap box otherwise; the
//! conversions below inline the common case and call the runtime for the
//! rest.
//!
//! Each operator is one runtime call over two words: `pycc_rt` dispatches
//! on what the words hold, with CPython's semantics for each pairing.

use super::*;
use inkwell::IntPredicate;

/// `pycc_rt::num::SPECIAL_BASE`: the first word of the inline `float`s
/// the rotation cannot place, which only the runtime decodes.
const SPECIAL_BASE: u64 = 0x8000_0000_0000_0002;
/// The span of those words.
const SPECIAL_SPAN: u64 = 32;

/// Every `pycc_rt_num_*` function this crate calls.
pub(crate) struct NumFns<'ctx> {
    pub(crate) from_float: FunctionValue<'ctx>,
    pub(crate) to_float: FunctionValue<'ctx>,
    pub(crate) to_int: FunctionValue<'ctx>,
    pub(crate) binop: FunctionValue<'ctx>,
    pub(crate) truediv: FunctionValue<'ctx>,
    pub(crate) compare: FunctionValue<'ctx>,
    pub(crate) abs: FunctionValue<'ctx>,
    pub(crate) truthy: FunctionValue<'ctx>,
    pub(crate) to_str: FunctionValue<'ctx>,
    pub(crate) hash: FunctionValue<'ctx>,
    pub(crate) round: FunctionValue<'ctx>,
    pub(crate) round_digits: FunctionValue<'ctx>,
    pub(crate) int_round_digits: FunctionValue<'ctx>,
    pub(crate) check_element: FunctionValue<'ctx>,
    pub(crate) list_contains: FunctionValue<'ctx>,
    /// `llvm.fshl.i64`/`llvm.fshr.i64`: the rotations of the inline
    /// `float` encoding.
    fshl: FunctionValue<'ctx>,
    fshr: FunctionValue<'ctx>,
}

impl<'ctx> NumFns<'ctx> {
    pub(crate) fn declare(context: &'ctx Context, module: &inkwell::module::Module<'ctx>) -> Self {
        let i64_type = context.i64_type();
        let i32_type = context.i32_type();
        let i8_type = context.i8_type();
        let f64_type = context.f64_type();
        let ptr_type = context.ptr_type(inkwell::AddressSpace::default());
        let declare = |name: &str, fn_type: inkwell::types::FunctionType<'ctx>| {
            module.get_function(name).unwrap_or_else(|| {
                module.add_function(name, fn_type, Some(inkwell::module::Linkage::External))
            })
        };
        let word_to_word = i64_type.fn_type(&[i64_type.into()], false);
        let intrinsic = |name: &str| {
            inkwell::intrinsics::Intrinsic::find(name)
                .and_then(|intrinsic| intrinsic.get_declaration(module, &[i64_type.into()]))
                .expect("LLVM provides the funnel-shift intrinsics")
        };
        NumFns {
            from_float: declare("pycc_rt_num_from_float", i64_type.fn_type(&[f64_type.into()], false)),
            to_float: declare("pycc_rt_num_to_float", f64_type.fn_type(&[i64_type.into()], false)),
            to_int: declare("pycc_rt_num_to_int", word_to_word),
            binop: declare(
                "pycc_rt_num_binop",
                i64_type.fn_type(&[i32_type.into(), i64_type.into(), i64_type.into()], false),
            ),
            truediv: declare(
                "pycc_rt_num_truediv",
                f64_type.fn_type(&[i64_type.into(), i64_type.into()], false),
            ),
            compare: declare(
                "pycc_rt_num_compare",
                i8_type.fn_type(&[i32_type.into(), i64_type.into(), i64_type.into()], false),
            ),
            abs: declare("pycc_rt_num_abs", word_to_word),
            truthy: declare("pycc_rt_num_truthy", i8_type.fn_type(&[i64_type.into()], false)),
            to_str: declare("pycc_rt_num_to_str", ptr_type.fn_type(&[i64_type.into()], false)),
            hash: declare("pycc_rt_hash_num", word_to_word),
            round: declare("pycc_rt_num_round", word_to_word),
            round_digits: declare(
                "pycc_rt_num_round_digits",
                i64_type.fn_type(&[i64_type.into(), i64_type.into()], false),
            ),
            int_round_digits: declare(
                "pycc_rt_int_round_digits",
                i64_type.fn_type(&[i64_type.into(), i64_type.into()], false),
            ),
            check_element: declare(
                "pycc_rt_num_check_element",
                context.void_type().fn_type(&[i64_type.into()], false),
            ),
            list_contains: declare(
                "pycc_rt_list_contains_num",
                i8_type.fn_type(&[ptr_type.into(), i64_type.into()], false),
            ),
            fshl: intrinsic("llvm.fshl"),
            fshr: intrinsic("llvm.fshr"),
        }
    }
}

/// `pycc_rt::num`'s operator codes.
pub(crate) fn binop_code(op: pycc_mir::BinOpKind) -> u64 {
    match op {
        pycc_mir::BinOpKind::Add => 0,
        pycc_mir::BinOpKind::Sub => 1,
        pycc_mir::BinOpKind::Mul => 2,
        pycc_mir::BinOpKind::FloorDiv => 3,
        pycc_mir::BinOpKind::Mod => 4,
        pycc_mir::BinOpKind::Pow => 5,
        other => unreachable!("pycc_types admits no `int | float` `{other:?}` (T0021)"),
    }
}

fn current_function<'ctx>(builder: &inkwell::builder::Builder<'ctx>) -> FunctionValue<'ctx> {
    builder
        .get_insert_block()
        .and_then(|block| block.get_parent())
        .expect("an `int | float` value is emitted inside a function body")
}

fn call_value<'ctx>(
    builder: &inkwell::builder::Builder<'ctx>,
    function: FunctionValue<'ctx>,
    args: &[inkwell::values::BasicMetadataValueEnum<'ctx>],
    name: &str,
) -> inkwell::values::BasicValueEnum<'ctx> {
    builder
        .build_call(function, args, name)
        .expect("build_call should not fail for a declared runtime function")
        .try_as_basic_value()
        .expect_basic("the `pycc_rt_num_*` functions return a value")
}

/// The word of a numeric scalar. An `int` or `int | float` operand is its
/// own word (with its own ownership); a `bool` is its identity marker
/// (owning nothing); a `float` is a fresh word ([`emit_float_to_num`]),
/// which the caller releases when it does not keep it.
pub(crate) fn to_num_word<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    rt: &RtFns<'ctx>,
    scalar: Scalar<'ctx>,
) -> IntValue<'ctx> {
    match scalar {
        Scalar::Num(word) | Scalar::Int(word) => word,
        Scalar::Bool(_) => to_encoded_int(context, builder, scalar),
        Scalar::Float(value) => emit_float_to_num(context, builder, rt, value),
        _ => panic!("pycc_codegen: internal error: expected a numeric operand for `int | float`"),
    }
}

/// `to_num_word` of the operand `expr` evaluated to `scalar`, and the word
/// to release once the operation is done when the conversion made one (a
/// `float` operand).
pub(crate) fn operand_word<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    rt: &RtFns<'ctx>,
    scalar: Scalar<'ctx>,
) -> (IntValue<'ctx>, Option<IntValue<'ctx>>) {
    let converted = matches!(scalar, Scalar::Float(_));
    let word = to_num_word(context, builder, rt, scalar);
    (word, converted.then_some(word))
}

/// Releases the words [`operand_word`] made.
pub(crate) fn release_converted<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    rt: &RtFns<'ctx>,
    words: &[Option<IntValue<'ctx>>],
) {
    for word in words.iter().flatten() {
        crate::bigint_rc::emit_bigint_refcount_call(
            context,
            builder,
            rt,
            *word,
            crate::bigint_rc::BigIntRefcount::Release,
        );
    }
}

/// A `float` as a fresh `int | float` word: inline for the common
/// magnitudes (the rotation of `pycc_rt::num`), a runtime call otherwise.
pub(crate) fn emit_float_to_num<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    rt: &RtFns<'ctx>,
    value: FloatValue<'ctx>,
) -> IntValue<'ctx> {
    let i64_type = context.i64_type();
    let c = |v: u64| i64_type.const_int(v, false);
    let bits = builder
        .build_bit_cast(value, i64_type, "num_float_bits")
        .expect("bitcast f64 to i64")
        .into_int_value();
    let top = builder.build_right_shift(bits, c(60), false, "num_exp_top").expect("lshr");
    let top = builder.build_and(top, c(0b111), "num_exp_top3").expect("and");
    let offset = builder.build_int_sub(top, c(3), "num_exp_offset").expect("sub");
    let in_range = builder
        .build_int_compare(IntPredicate::ULE, offset, c(1), "num_in_range")
        .expect("icmp");
    let rotated = call_value(
        builder,
        rt.num.fshl,
        &[bits.into(), bits.into(), c(3).into()],
        "num_rotl",
    )
    .into_int_value();
    let cleared = builder.build_and(rotated, c(!0b11), "num_clear_tag").expect("and");
    let word = builder.build_or(cleared, c(0b10), "num_inline").expect("or");
    let marker = builder.build_and(word, c(!0b100), "num_marker_test").expect("and");
    let not_marker = builder
        .build_int_compare(IntPredicate::NE, marker, c(0b10), "num_not_marker")
        .expect("icmp");
    let special = builder.build_int_sub(word, c(SPECIAL_BASE), "num_special").expect("sub");
    let not_special = builder
        .build_int_compare(IntPredicate::UGE, special, c(SPECIAL_SPAN), "num_not_special")
        .expect("icmp");
    let fast = builder.build_and(in_range, not_marker, "num_fast0").expect("and");
    let fast = builder.build_and(fast, not_special, "num_fast").expect("and");
    let function = current_function(builder);
    let fast_bb = builder.get_insert_block().expect("positioned");
    let slow_bb = context.append_basic_block(function, "num_box_float");
    let join_bb = context.append_basic_block(function, "num_from_float");
    crate::int_fast::likely_branch(context, builder, fast, join_bb, slow_bb);
    builder.position_at_end(slow_bb);
    let slow = call_value(builder, rt.num.from_float, &[value.into()], "num_from_float_call")
        .into_int_value();
    let slow_end = builder.get_insert_block().expect("positioned");
    builder.build_unconditional_branch(join_bb).expect("branch");
    builder.position_at_end(join_bb);
    let phi = builder.build_phi(i64_type, "num_word").expect("phi");
    phi.add_incoming(&[(&word, fast_bb), (&slow, slow_end)]);
    phi.as_basic_value().into_int_value()
}

/// `float(x)` of an `int | float` word: decoded inline for an inline
/// `float`, a runtime call otherwise (which raises `OverflowError` for an
/// `int` beyond the `float` range; `guard` emits the pending-exception
/// test after it, for an operation that guards itself).
pub(crate) fn emit_num_to_float<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    rt: &RtFns<'ctx>,
    word: IntValue<'ctx>,
    guard: bool,
) -> FloatValue<'ctx> {
    let i64_type = context.i64_type();
    let c = |v: u64| i64_type.const_int(v, false);
    let tag = builder.build_and(word, c(0b11), "num_tag").expect("and");
    let is_inline = builder
        .build_int_compare(IntPredicate::EQ, tag, c(0b10), "num_is_inline")
        .expect("icmp");
    let marker = builder.build_and(word, c(!0b100), "num_marker_test").expect("and");
    let not_marker = builder
        .build_int_compare(IntPredicate::NE, marker, c(0b10), "num_not_marker")
        .expect("icmp");
    let special = builder.build_int_sub(word, c(SPECIAL_BASE), "num_special").expect("sub");
    let not_special = builder
        .build_int_compare(IntPredicate::UGE, special, c(SPECIAL_SPAN), "num_not_special")
        .expect("icmp");
    let fast = builder.build_and(is_inline, not_marker, "num_fast0").expect("and");
    let fast = builder.build_and(fast, not_special, "num_fast").expect("and");
    let function = current_function(builder);
    let fast_bb = context.append_basic_block(function, "num_unbox_inline");
    let slow_bb = context.append_basic_block(function, "num_unbox_call");
    let join_bb = context.append_basic_block(function, "num_to_float");
    crate::int_fast::likely_branch(context, builder, fast, fast_bb, slow_bb);
    builder.position_at_end(fast_bb);
    let negative = builder
        .build_int_compare(IntPredicate::SLT, word, i64_type.const_zero(), "num_bit60")
        .expect("icmp");
    let top = builder
        .build_select(negative, c(0b01), c(0b10), "num_top_bits")
        .expect("select")
        .into_int_value();
    let cleared = builder.build_and(word, c(!0b11), "num_clear_tag").expect("and");
    let restored = builder.build_or(cleared, top, "num_restored").expect("or");
    let bits = call_value(
        builder,
        rt.num.fshr,
        &[restored.into(), restored.into(), c(3).into()],
        "num_rotr",
    )
    .into_int_value();
    let fast_value = builder
        .build_bit_cast(bits, context.f64_type(), "num_float")
        .expect("bitcast i64 to f64")
        .into_float_value();
    builder.build_unconditional_branch(join_bb).expect("branch");
    builder.position_at_end(slow_bb);
    let slow_value =
        call_value(builder, rt.num.to_float, &[word.into()], "num_to_float_call").into_float_value();
    if guard {
        crate::exception::guard_statement_effects(context, builder, rt);
    }
    let slow_end = builder.get_insert_block().expect("positioned");
    builder.build_unconditional_branch(join_bb).expect("branch");
    builder.position_at_end(join_bb);
    let phi = builder.build_phi(context.f64_type(), "num_as_float").expect("phi");
    phi.add_incoming(&[(&fast_value, fast_bb), (&slow_value, slow_end)]);
    phi.as_basic_value().into_float_value()
}

/// `left op right` with an `int | float` result: both operands as words,
/// one runtime call, then the operands released as their consumer.
#[allow(clippy::too_many_arguments)]
pub(crate) fn emit_binop<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    rt: &RtFns<'ctx>,
    op: pycc_mir::BinOpKind,
    left: &MirExpr,
    l: Scalar<'ctx>,
    right: &MirExpr,
    r: Scalar<'ctx>,
) -> Scalar<'ctx> {
    let (lw, lc) = operand_word(context, builder, rt, l);
    let (rw, rc) = operand_word(context, builder, rt, r);
    let code = context.i32_type().const_int(binop_code(op), false);
    let result = call_value(builder, rt.num.binop, &[code.into(), lw.into(), rw.into()], "num_binop")
        .into_int_value();
    release_converted(context, builder, rt, &[lc, rc]);
    release_scalar_if_temporary(context, builder, rt, left, &l);
    release_scalar_if_temporary(context, builder, rt, right, &r);
    Scalar::Num(result)
}

/// `left / right` when an operand is `int | float`: two `int`s divide
/// exactly (`pycc_rt_num_truediv`), anything else as `float`s.
pub(crate) fn emit_truediv<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    rt: &RtFns<'ctx>,
    left: &MirExpr,
    l: Scalar<'ctx>,
    right: &MirExpr,
    r: Scalar<'ctx>,
) -> Scalar<'ctx> {
    let (lw, lc) = operand_word(context, builder, rt, l);
    let (rw, rc) = operand_word(context, builder, rt, r);
    let result = call_value(builder, rt.num.truediv, &[lw.into(), rw.into()], "num_truediv")
        .into_float_value();
    release_converted(context, builder, rt, &[lc, rc]);
    release_scalar_if_temporary(context, builder, rt, left, &l);
    release_scalar_if_temporary(context, builder, rt, right, &r);
    Scalar::Float(result)
}

/// The `i8` truth of `l op r` (a `pycc_rt::num` comparison code) for a
/// comparison with an `int | float` operand. Releases only the words a
/// conversion made; the caller owns the operands (`compare`).
pub(crate) fn emit_compare<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    rt: &RtFns<'ctx>,
    code: u64,
    l: Scalar<'ctx>,
    r: Scalar<'ctx>,
) -> IntValue<'ctx> {
    let (lw, lc) = operand_word(context, builder, rt, l);
    let (rw, rc) = operand_word(context, builder, rt, r);
    let code = context.i32_type().const_int(code, false);
    let result = call_value(builder, rt.num.compare, &[code.into(), lw.into(), rw.into()], "num_cmp")
        .into_int_value();
    release_converted(context, builder, rt, &[lc, rc]);
    result
}

/// One `pycc_rt_num_*` call over the word `word` returning a value of
/// `kind`.
pub(crate) fn emit_unary<'ctx>(
    builder: &inkwell::builder::Builder<'ctx>,
    function: FunctionValue<'ctx>,
    word: IntValue<'ctx>,
) -> inkwell::values::BasicValueEnum<'ctx> {
    call_value(builder, function, &[word.into()], "num_unary")
}

/// The container-ingress check of an `int | float` element: a word with a
/// heap payload (low bits `00`, non-zero) raises `OverflowError` on a cold
/// call, followed by the pending-exception guard. `release_on_refuse` is
/// set when `word` is a fresh conversion of a `float` no one else owns: the
/// refusal then releases it before raising.
pub(crate) fn emit_check_element<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    rt: &RtFns<'ctx>,
    word: IntValue<'ctx>,
    release_on_refuse: bool,
) {
    let i64_type = context.i64_type();
    let tag = builder.build_and(word, i64_type.const_int(0b11, false), "num_elem_tag").expect("and");
    let aligned = builder
        .build_int_compare(IntPredicate::EQ, tag, i64_type.const_zero(), "num_elem_aligned")
        .expect("icmp");
    let non_zero = builder
        .build_int_compare(IntPredicate::NE, word, i64_type.const_zero(), "num_elem_nonzero")
        .expect("icmp");
    let heap = builder.build_and(aligned, non_zero, "num_elem_heap").expect("and");
    let function = current_function(builder);
    let refuse_bb = context.append_basic_block(function, "num_elem_refuse");
    let join_bb = context.append_basic_block(function, "num_elem_ok");
    let branch = builder
        .build_conditional_branch(heap, refuse_bb, join_bb)
        .expect("branch");
    crate::int_fast::weight_branch(context, branch, false);
    builder.position_at_end(refuse_bb);
    builder
        .build_call(rt.num.check_element, &[word.into()], "")
        .expect("build_call should not fail for a declared runtime function");
    if release_on_refuse {
        builder
            .build_call(rt.bigint_release, &[word.into()], "")
            .expect("build_call should not fail for a declared runtime function");
    }
    crate::exception::guard_statement_effects(context, builder, rt);
    builder.build_unconditional_branch(join_bb).expect("branch");
    builder.position_at_end(join_bb);
}

