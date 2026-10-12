//! Index evaluation, decoding and bounds checking for typed array element
//! access.

use super::*;

/// Decodes an `int` index word to a machine integer: a smallint (odd
/// word) by a shift, a bool marker (`0b0010`/`0b0110`) to 0 or 1, and a
/// bigint -- which numpy refuses as an index -- by raising its `IndexError`
/// on the foreign failure edge.
///
/// Branch-free but for that exit: the two decodings are selected, and the
/// only branch leaves the loop around the access. So a loop-invariant index
/// word is decoded once, outside the loop, and nothing that rejoins makes a
/// call (a call inside a loop would keep LLVM from hoisting the array's
/// header).
pub(super) fn untag_index<'ctx>(
    context: &'ctx Context,
    builder: &Builder<'ctx>,
    module: &inkwell::module::Module<'ctx>,
    rt: &RtFns<'ctx>,
    word: IntValue<'ctx>,
) -> IntValue<'ctx> {
    let i64_type = context.i64_type();
    if let Some(constant) = word.get_sign_extended_constant()
        && constant & 1 == 1
    {
        return i64_type.const_int((constant >> 1) as u64, true);
    }
    let one = i64_type.const_int(1, false);
    let low = builder.build_and(word, one, "nd_low").expect("and");
    let is_small = builder
        .build_int_compare(IntPredicate::NE, low, i64_type.const_zero(), "nd_is_small")
        .expect("build_int_compare should not fail");
    // `0b0010 | 0b0100 == 0b0110 | 0b0100`: exactly the two markers.
    let folded = builder
        .build_or(word, i64_type.const_int(4, false), "nd_marker_fold")
        .expect("or");
    let is_marker = builder
        .build_int_compare(
            IntPredicate::EQ,
            folded,
            i64_type.const_int(6, false),
            "nd_is_marker",
        )
        .expect("build_int_compare should not fail");
    let decodable = builder
        .build_or(is_small, is_marker, "nd_decodable")
        .expect("or");
    let big = builder.build_not(decodable, "nd_big_index").expect("not");
    cold_raise(context, builder, module, rt, big, "nd_index", |b| {
        let ptr = context.ptr_type(inkwell::AddressSpace::default());
        let raise = shim_fn(
            module,
            ND_INDEX_WORD_SYMBOL,
            context
                .i32_type()
                .fn_type(&[i64_type.into(), ptr.into()], false),
        );
        b.build_call(
            raise,
            &[word.into(), ptr.const_null().into()],
            "nd_big_index",
        )
        .expect("build_call should not fail");
    });
    let small = builder
        .build_right_shift(word, one, true, "nd_untag")
        .expect("ashr");
    let marker = builder
        .build_right_shift(word, i64_type.const_int(2, false), false, "nd_marker")
        .expect("lshr");
    builder
        .build_select(is_small, small, marker, "nd_index")
        .expect("build_select should not fail")
        .into_int_value()
}

/// Whether `expr` is an index leaf [`raw_index`] can decode without
/// arithmetic on `int` words: an `int` name or a small literal.
pub(super) fn is_index_leaf(expr: &MirExpr) -> bool {
    match expr {
        MirExpr::Name { ty, .. } => *ty == pycc_mir::Ty::Int,
        MirExpr::IntLiteral(k) => k.unsigned_abs() < (1 << 61),
        _ => false,
    }
}

/// An index expression evaluated but not yet decoded ([`evaluate_index`]).
pub(super) enum PendingIndex<'ctx, 'm> {
    /// A leaf, or the sum or difference of two leaves.
    Leaves {
        left: PendingLeaf<'ctx>,
        right: Option<(pycc_mir::BinOpKind, PendingLeaf<'ctx>)>,
    },
    /// Any other expression's `int` word, which the caller releases.
    Word(&'m MirExpr, IntValue<'ctx>),
}

/// A leaf of an index: a machine integer, or an `int` word to untag.
#[derive(Clone, Copy)]
pub(super) enum PendingLeaf<'ctx> {
    Raw(IntValue<'ctx>),
    Word(IntValue<'ctx>),
}

/// Evaluates the index expression `expr`, leaving its decoding to
/// [`decode_index`].
///
/// `i`, `i - 1` and `i + j` -- an `int` name or small literal, or the sum
/// or difference of two -- are computed on untagged machine words: each
/// leaf is below 2**62 in magnitude, so the sum cannot overflow `i64`, and
/// no `int` temporary is created. Anything else is evaluated as an `int`.
#[allow(clippy::too_many_arguments)]
pub(super) fn evaluate_index<'ctx, 'm>(
    context: &'ctx Context,
    builder: &Builder<'ctx>,
    module: &inkwell::module::Module<'ctx>,
    rt: &RtFns<'ctx>,
    user_functions: &HashMap<&str, UserFunction<'ctx>>,
    locals: &HashMap<String, StorageSlot<'ctx>>,
    expr: &'m MirExpr,
) -> PendingIndex<'ctx, 'm> {
    let leaf = |e: &MirExpr| match e {
        MirExpr::IntLiteral(k) => PendingLeaf::Raw(context.i64_type().const_int(*k as u64, true)),
        _ => {
            let scalar = emit_expr(context, builder, module, rt, user_functions, locals, e);
            PendingLeaf::Word(to_numeric_encoded_int(context, builder, scalar))
        }
    };
    match expr {
        MirExpr::BinOp {
            op: op @ (pycc_mir::BinOpKind::Add | pycc_mir::BinOpKind::Sub),
            left,
            right,
            ty: pycc_mir::Ty::Int,
        } if is_index_leaf(left) && is_index_leaf(right) => PendingIndex::Leaves {
            left: leaf(left),
            right: Some((*op, leaf(right))),
        },
        e if is_index_leaf(e) => PendingIndex::Leaves {
            left: leaf(e),
            right: None,
        },
        other => {
            let scalar = emit_expr(context, builder, module, rt, user_functions, locals, other);
            PendingIndex::Word(other, to_numeric_encoded_int(context, builder, scalar))
        }
    }
}

/// The machine value of an evaluated index ([`untag_index`]); the second
/// element of the pair is an evaluated `int` word for the caller to
/// release.
pub(super) fn decode_index<'ctx, 'm>(
    context: &'ctx Context,
    builder: &Builder<'ctx>,
    module: &inkwell::module::Module<'ctx>,
    rt: &RtFns<'ctx>,
    index: PendingIndex<'ctx, 'm>,
) -> (IntValue<'ctx>, Option<(&'m MirExpr, IntValue<'ctx>)>) {
    let leaf = |l: PendingLeaf<'ctx>| match l {
        PendingLeaf::Raw(value) => value,
        PendingLeaf::Word(word) => untag_index(context, builder, module, rt, word),
    };
    match index {
        PendingIndex::Leaves { left, right: None } => (leaf(left), None),
        PendingIndex::Leaves {
            left,
            right: Some((op, right)),
        } => {
            let l = leaf(left);
            let r = leaf(right);
            let value = if op == pycc_mir::BinOpKind::Add {
                builder.build_int_add(l, r, "nd_index_sum")
            } else {
                builder.build_int_sub(l, r, "nd_index_diff")
            }
            .expect("integer arithmetic should not fail");
            (value, None)
        }
        PendingIndex::Word(expr, word) => (
            untag_index(context, builder, module, rt, word),
            Some((expr, word)),
        ),
    }
}

/// The address of the element at `indices`, after bounds-checking each one
/// against its axis (a negative index counts from the end).
///
/// `chained` is `a[i][j]`: the second index indexes a row, a 1-D array, so
/// numpy reports it against axis 0.
pub(super) fn element_address<'ctx>(
    context: &'ctx Context,
    builder: &Builder<'ctx>,
    module: &inkwell::module::Module<'ctx>,
    rt: &RtFns<'ctx>,
    header: &Header<'ctx>,
    indices: &[IntValue<'ctx>],
    chained: bool,
) -> PointerValue<'ctx> {
    let i64_type = context.i64_type();
    let mut offset = i64_type.const_zero();
    for (axis, index) in indices.iter().enumerate() {
        let axis = u8::try_from(axis).expect("at most two axes");
        let size = header.dim(context, builder, axis, false);
        let negative = builder
            .build_int_compare(IntPredicate::SLT, *index, i64_type.const_zero(), "nd_neg")
            .expect("build_int_compare should not fail");
        let wrapped = builder
            .build_int_add(*index, size, "nd_wrapped")
            .expect("build_int_add should not fail");
        let position = builder
            .build_select(negative, wrapped, *index, "nd_pos")
            .expect("build_select should not fail")
            .into_int_value();
        // One unsigned compare also catches a still-negative position.
        let out_of_range = builder
            .build_int_compare(IntPredicate::UGE, position, size, "nd_oob")
            .expect("build_int_compare should not fail");
        let reported_axis = if chained { 0 } else { u64::from(axis) };
        let index_value = *index;
        cold_raise(
            context,
            builder,
            module,
            rt,
            out_of_range,
            "nd_bounds",
            |b| {
                let raise = shim_fn(
                    module,
                    ND_INDEX_ERROR_SYMBOL,
                    context
                        .i32_type()
                        .fn_type(&[i64_type.into(), i64_type.into(), i64_type.into()], false),
                );
                b.build_call(
                    raise,
                    &[
                        index_value.into(),
                        i64_type.const_int(reported_axis, false).into(),
                        size.into(),
                    ],
                    "nd_index_error",
                )
                .expect("build_call should not fail");
            },
        );
        let stride = header.dim(context, builder, axis, true);
        let scaled = builder
            .build_int_mul(position, stride, "nd_scaled")
            .expect("build_int_mul should not fail");
        offset = builder
            .build_int_add(offset, scaled, "nd_offset")
            .expect("build_int_add should not fail");
    }
    let data = header.data(context, builder);
    // SAFETY (LLVM): within the array's data, by the bounds checks above.
    unsafe {
        builder
            .build_gep(context.i8_type(), data, &[offset], "nd_element")
            .expect("build_gep should not fail")
    }
}
