//! `str` operations of a generic body whose `int` operands stay words.
//!
//! In a generic body a scanner's position is an `object` local, often a
//! tagged one (`tagged_locals`). `src[start:pos]` and `src.startswith('"""',
//! pos)` would otherwise box each position into an `int`, build a `slice`
//! object or a bound method call, and free them again, for an operation
//! whose answer depends only on the string and two machine integers.
//! Where the receiver is a local holding an exact `str` and every `int`
//! operand is a small `int` -- a literal, a local holding one (tagged or a
//! compact `int` object), or the sum or difference of two such -- these
//! take a word path. A region copy's position is a native `int` local
//! instead (`docs/design/selected-regions.md` R3), as is any `Ty::Int`
//! local of a compiled body (a `range` loop variable, for one): its D-061
//! word is read as it is, and a sum or difference of native locals (`MirExpr::BinOp`)
//! is computed from the leaves as an `object` one is.
//!
//! - `s[a:b]` (no step): `pycc_ext_str_slice` clamps the bounds as
//!   `PySlice_AdjustIndices` does and returns what CPython's own slice
//!   returns ([`emit_slice`]);
//! - the truth of `s.startswith(t)` or `s.startswith(t, a)`, `t` a literal
//!   of one to eight ASCII characters, of a compact ASCII `s`, in an `if`,
//!   a `while` or a `not` ([`emit_truth`]): the bytes compared inline.
//!
//! A `str` subclass, a non-`int` operand, a `bool`, a big `int` or any
//! other receiver takes CPython's own operation, as before, from the same
//! evaluated receiver. The operands are names and literals, so evaluating
//! them again on that path has no effect a program can see; an exact
//! `str`'s `startswith` and slice cannot be overridden, and no `slice` or
//! `int` object they would have received escapes. The sum `a + b` of two
//! exact `int`s is the value CPython's `int.__add__` gives. Reading a
//! native `int` name makes no reference, so a big `int` word found on the
//! word path is read again, not leaked, on CPython's.

use super::*;
use crate::fast_ops::Fast;
use crate::foreign_fail::{ForeignFailEdge, route_null};
use crate::tagged_locals::{INT_BOUND, expect_object, int_operand};
use inkwell::builder::Builder;
use pycc_mir::{BinOpKind, Ty};

/// The receiver names [`emit_slice`] and [`emit_truth`] take: an `object`
/// name, not the module namespace a global read subscripts. Reading one
/// has no effect a program can see, so the CPython path may read it
/// again. A pinned read (`operand_pin`) is a new reference, which the word
/// path holds until the operation is done.
fn receiver(base: &MirExpr) -> bool {
    matches!(base, MirExpr::Name { ty: Ty::Object, name } if name != crate::ext::FALLBACK_NS_NAME)
}

/// An `int` literal, an `object` local or a native `int` local.
fn word_leaf(expr: &MirExpr) -> bool {
    matches!(expr, MirExpr::IntLiteral(_)) || receiver(expr) || native_int_name(expr)
}

/// A read of a native `int` local (a region copy's `pos`,
/// `docs/design/selected-regions.md` section 4.2): one D-061 word, which
/// the word path reads as it is, a small one by its tag (`int_operand`).
pub(crate) fn native_int_name(expr: &MirExpr) -> bool {
    matches!(expr, MirExpr::Name { ty: Ty::Int, .. })
}

/// Whether `expr` is an `int` operand the word path computes: a leaf, or
/// `leaf + leaf` / `leaf - leaf` of `object`s (`ObjBinOp`) or of native
/// `int`s (`BinOp`).
fn word_operand(expr: &MirExpr) -> bool {
    match expr {
        MirExpr::ObjBinOp { op: BinOpKind::Add | BinOpKind::Sub, left, right, .. }
        | MirExpr::BinOp { op: BinOpKind::Add | BinOpKind::Sub, left, right, ty: Ty::Int } => {
            word_leaf(left) && word_leaf(right)
        }
        _ => word_leaf(expr),
    }
}

/// The `int` operands of `expr` that [`emit_slice`] or [`emit_truth`]
/// would compute as words, when `expr` has their shape.
pub(crate) fn word_operands(expr: &MirExpr) -> Vec<&MirExpr> {
    match expr {
        MirExpr::ObjSlice { base, start, stop, step: None } if receiver(base) => {
            let bounds: Vec<&MirExpr> = start.iter().chain(stop.iter()).map(|bound| bound.as_ref()).collect();
            if !bounds.is_empty() && bounds.iter().all(|bound| word_operand(bound)) {
                bounds
            } else {
                Vec::new()
            }
        }
        MirExpr::ObjMethodCall { base, method, args } if method == "startswith" && receiver(base) => match args.as_slice() {
            [MirExpr::StringLiteral(text), start] if word_operand(start) && startswith_text(text) => vec![start],
            _ => Vec::new(),
        },
        _ => Vec::new(),
    }
}

/// Whether [`emit_truth`] compares `text` inline.
fn startswith_text(text: &str) -> bool {
    !text.is_empty() && text.len() <= 8 && text.is_ascii()
}

/// `|value| < 2**62`, as an `i1`.
fn in_range<'ctx>(context: &'ctx Context, builder: &Builder<'ctx>, value: IntValue<'ctx>, name: &str) -> IntValue<'ctx> {
    let i64_type = context.i64_type();
    let biased = builder
        .build_int_add(value, i64_type.const_int(INT_BOUND, false), name)
        .expect("add");
    builder
        .build_int_compare(IntPredicate::ULT, biased, i64_type.const_int(2 * INT_BOUND, false), name)
        .expect("icmp")
}

/// The `i64` value of a [`word_operand`] on `fast`'s path, requiring every
/// leaf to be a small `int` and the result to stay below `2**62`.
#[allow(clippy::too_many_arguments)]
fn emit_word_operand<'ctx>(
    context: &'ctx Context,
    builder: &Builder<'ctx>,
    module: &inkwell::module::Module<'ctx>,
    rt: &RtFns<'ctx>,
    user_functions: &HashMap<&str, UserFunction<'ctx>>,
    locals: &HashMap<String, StorageSlot<'ctx>>,
    fast: &Fast<'_, 'ctx>,
    expr: &MirExpr,
    name: &str,
) -> IntValue<'ctx> {
    let leaf = |operand: &MirExpr, label: &str| {
        let value = int_operand(context, builder, module, rt, user_functions, locals, fast, operand, label);
        fast.require(in_range(context, builder, value, label), label);
        value
    };
    match expr {
        MirExpr::ObjBinOp { op, left, right, .. } | MirExpr::BinOp { op, left, right, .. } => {
            let a = leaf(left, &format!("{name}_left"));
            let b = leaf(right, &format!("{name}_right"));
            // Both below 2**62 in magnitude: the `i64` cannot wrap.
            let value = match op {
                BinOpKind::Add => builder.build_int_add(a, b, name),
                _ => builder.build_int_sub(a, b, name),
            }
            .expect("an operation of two i64");
            fast.require(in_range(context, builder, value, name), name);
            value
        }
        _ => leaf(expr, name),
    }
}

/// `base[start:stop]` (`MirExpr::ObjSlice`) on the word path (module
/// docs), or `None` when the expression is not of that shape.
pub(crate) fn emit_slice<'ctx>(
    context: &'ctx Context,
    builder: &Builder<'ctx>,
    module: &inkwell::module::Module<'ctx>,
    rt: &RtFns<'ctx>,
    user_functions: &HashMap<&str, UserFunction<'ctx>>,
    locals: &HashMap<String, StorageSlot<'ctx>>,
    expr: &MirExpr,
) -> Option<Scalar<'ctx>> {
    let MirExpr::ObjSlice { base, start, stop, step: None } = expr else {
        return None;
    };
    if !receiver(base)
        || (start.is_none() && stop.is_none())
        || !start.iter().chain(stop.iter()).all(|bound| word_operand(bound))
    {
        return None;
    }
    let ptr = context.ptr_type(inkwell::AddressSpace::default());
    let i64_type = context.i64_type();
    let base_scalar = emit_expr(context, builder, module, rt, user_functions, locals, base);
    let held = crate::object_release::hold(context, module, rt, base, &base_scalar);
    let base_object = expect_object(base_scalar);
    let value = crate::fast_ops::guarded(
        context,
        builder,
        module,
        "str_slice",
        |fast| {
            fast.exact_str(base_object, "str_slice_base");
            let bound = |bound: &Option<Box<MirExpr>>, absent: u64, label: &str| match bound {
                Some(bound) => {
                    emit_word_operand(context, builder, module, rt, user_functions, locals, fast, bound, label)
                }
                None => i64_type.const_int(absent, false),
            };
            let lo = bound(start, 0, "str_slice_start");
            let hi = bound(stop, i64::MAX as u64, "str_slice_stop");
            let slice = crate::foreign_pack::shim_fn(
                module,
                crate::ext::EXT_STR_SLICE_SYMBOL,
                ptr.fn_type(&[ptr.into(), i64_type.into(), i64_type.into()], false),
            );
            let edge = ForeignFailEdge::for_current(builder);
            let result = builder
                .build_call(slice, &[base_object.into(), lo.into(), hi.into()], "str_slice")
                .expect("build_call should not fail for pycc_ext_str_slice")
                .try_as_basic_value()
                .expect_basic("pycc_ext_str_slice returns PyObject *")
                .into_pointer_value();
            route_null(context, builder, module, rt, edge, result, "str_slice");
            result.into()
        },
        || {
            let MirExpr::ObjSlice { base, start, stop, step } = expr else {
                unreachable!("matched above")
            };
            expect_object(crate::foreign_slice::emit_with_operands(
                context,
                builder,
                module,
                rt,
                user_functions,
                locals,
                base,
                [start.as_deref(), stop.as_deref(), step.as_deref()],
                |base, bounds| crate::foreign_slice::emit_slice(context, builder, module, rt, base, bounds),
            ))
            .into()
        },
    );
    held.release(builder, rt);
    Some(Scalar::Object(value.into_pointer_value()))
}

/// The truth of `test` as an `i1` when it is `s.startswith(t)` or
/// `s.startswith(t, a)` on the word path (module docs); `None` otherwise.
pub(crate) fn emit_truth<'ctx>(
    context: &'ctx Context,
    builder: &Builder<'ctx>,
    module: &inkwell::module::Module<'ctx>,
    rt: &RtFns<'ctx>,
    user_functions: &HashMap<&str, UserFunction<'ctx>>,
    locals: &HashMap<String, StorageSlot<'ctx>>,
    test: &MirExpr,
) -> Option<IntValue<'ctx>> {
    let MirExpr::ObjMethodCall { base, method, args } = test else {
        return None;
    };
    if method != "startswith" || !receiver(base) {
        return None;
    }
    let (text, start) = match args.as_slice() {
        [MirExpr::StringLiteral(text)] => (text, None),
        [MirExpr::StringLiteral(text), start] if word_operand(start) => (text, Some(start)),
        _ => return None,
    };
    if !startswith_text(text) {
        return None;
    }
    let i64_type = context.i64_type();
    let bool_type = context.bool_type();
    let base_scalar = emit_expr(context, builder, module, rt, user_functions, locals, base);
    let held = crate::object_release::hold(context, module, rt, base, &base_scalar);
    let base_object = expect_object(base_scalar);
    let answer = crate::fast_ops::guarded(
        context,
        builder,
        module,
        "str_startswith",
        |fast| {
            let length = fast.ascii_str(base_object, "str_startswith_base");
            let at = match start {
                Some(start) => {
                    let at = emit_word_operand(
                        context,
                        builder,
                        module,
                        rt,
                        user_functions,
                        locals,
                        fast,
                        start,
                        "str_startswith_start",
                    );
                    // A negative start counts from the end, then is clamped
                    // to 0 (`ADJUST_INDICES`).
                    let negative = builder
                        .build_int_compare(IntPredicate::SLT, at, i64_type.const_zero(), "str_startswith_negative")
                        .expect("icmp");
                    let from_end = builder.build_int_add(at, length, "str_startswith_from_end").expect("add");
                    let from_end_negative = builder
                        .build_int_compare(IntPredicate::SLT, from_end, i64_type.const_zero(), "str_startswith_before")
                        .expect("icmp");
                    let from_end = builder
                        .build_select(from_end_negative, i64_type.const_zero(), from_end, "str_startswith_clamped")
                        .expect("select")
                        .into_int_value();
                    builder
                        .build_select(negative, from_end, at, "str_startswith_at")
                        .expect("select")
                        .into_int_value()
                }
                None => i64_type.const_zero(),
            };
            // `at + len(t) <= len(s)`, signed: `at` may lie past the end.
            let room = builder
                .build_int_sub(length, i64_type.const_int(text.len() as u64, false), "str_startswith_room")
                .expect("sub");
            let fits = builder
                .build_int_compare(IntPredicate::SLE, at, room, "str_startswith_fits")
                .expect("icmp");
            let function = builder
                .get_insert_block()
                .and_then(|block| block.get_parent())
                .expect("inside a function");
            let entry = builder.get_insert_block().expect("positioned");
            let compare = context.append_basic_block(function, "str_startswith_compare");
            let done = context.append_basic_block(function, "str_startswith_done");
            builder
                .build_conditional_branch(fits, compare, done)
                .expect("build_conditional_branch should not fail for an i1");
            builder.position_at_end(compare);
            let mut all = bool_type.const_int(1, false);
            for (offset, byte) in text.bytes().enumerate() {
                let index = builder
                    .build_int_add(at, i64_type.const_int(offset as u64, false), "str_startswith_index")
                    .expect("add");
                let read = fast.ascii_byte(base_object, index, "str_startswith");
                let same = builder
                    .build_int_compare(IntPredicate::EQ, read, i64_type.const_int(u64::from(byte), false), "str_startswith_same")
                    .expect("icmp");
                all = builder.build_and(all, same, "str_startswith_all").expect("and");
            }
            let compare_end = builder.get_insert_block().expect("positioned");
            builder.build_unconditional_branch(done).expect("branch");
            builder.position_at_end(done);
            let phi = builder.build_phi(bool_type, "str_startswith_answer").expect("phi");
            phi.add_incoming(&[(&bool_type.const_zero(), entry), (&all, compare_end)]);
            phi.as_basic_value()
        },
        || {
            let scalar = emit_expr(context, builder, module, rt, user_functions, locals, test);
            let held = crate::object_release::hold(context, module, rt, test, &scalar);
            let cond = truthy(context, builder, module, rt, scalar);
            held.release(builder, rt);
            release_scalar_if_temporary(context, builder, rt, test, &scalar);
            cond.into()
        },
    );
    held.release(builder, rt);
    Some(answer.into_int_value())
}
