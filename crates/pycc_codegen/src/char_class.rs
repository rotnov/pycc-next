//! `s[i] in chars` without a character object: a character-class test of
//! a generic body or a native copy.
//!
//! A scanner tests one character at a time against a set of characters
//! (`while src[pos] in chars: pos += 1`). Lowered as two operations, the
//! subscript reads CPython's cached one-character `str` and counts a
//! reference to it, and the membership test checks that object's type,
//! state and length again before it reads the byte the subscript had in
//! hand (`foreign_call::emit_subscript`, then `Fast::ascii_member`).
//!
//! [`emit`] fuses the two when the subscript's operands are reads that may
//! be evaluated again (as `tagged_locals::emit_char_load` requires): an
//! exact compact ASCII `s` indexed in range by a small `int` -- a literal,
//! a tagged local's word, a compact `int` object or a native `int` --
//! yields the character's code, and no object is made or counted. Any
//! other `s` or index (a `str` subclass, a non-ASCII `str`, an index out of
//! range, a `bool`, a big `int`) evaluates the subscript as before, so its
//! `__getitem__` and its `IndexError` are CPython's, and the object it
//! returns takes the membership test as before.
//!
//! The container is evaluated after the subscript, CPython's order. A
//! character's code against an exact `frozenset` is a bit of the set's
//! cached ASCII mask (`Fast::ascii_byte_member`): a `frozenset` cannot
//! change, the mask is computed only when every member is an exact `str`
//! (so no member's `__eq__` or `__hash__` can run, and an ASCII character
//! equals only the member with the same code), and a set too large or
//! with another member is never usable. Anything else -- a `set`, which
//! may change between two tests, a `frozenset` subclass, any other
//! container, an unusable mask -- receives CPython's own character object
//! (`PyUnicode_FromOrdinal` of an ASCII code is the interpreter's cached
//! `str`, the object the subscript returns) and the generic membership
//! test (`foreign_compare::emit_contains`).
//!
//! A `while` loop whose test is such a membership test, of a `str` and a
//! set that are parameters the loop never rebinds, checks both once
//! before the loop ([`prepare_loop`]): an exact compact ASCII `s` and an
//! exact usable `frozenset`, whose mask's two words it copies. Each test
//! then reads the byte and its bit, with no type, state or cache check;
//! an index that is not a small `int` in range takes the test above. The
//! copied words stay the set's answer whatever later happens to the
//! cache, since the set cannot change while the loop holds it in a name.

use super::*;
use crate::foreign_fail::{ForeignFailEdge, route_null};
use crate::tagged_locals::expect_object;
use inkwell::builder::Builder;
use inkwell::values::BasicValueEnum;
use pycc_mir::Ty;
use std::cell::RefCell;

/// The enclosing `while` loops' prepared tests ([`prepare_loop`]).
#[derive(Default)]
pub(crate) struct LoopClasses<'ctx> {
    prepared: RefCell<Vec<Prepared<'ctx>>>,
}

/// What a loop's preheader read once for its test's item.
#[derive(Clone, Copy)]
struct Prepared<'ctx> {
    /// The test's item, compared by address.
    item: *const MirExpr,
    /// The `str`'s characters and length, when it is an exact compact
    /// ASCII one and the set an exact `frozenset` with a usable mask; else
    /// a length of 0, which no index is below, so every test takes
    /// [`emit_each`] and nothing else here is read.
    data: PointerValue<'ctx>,
    length: IntValue<'ctx>,
    /// The set's mask: codes 0-63, then 64-127.
    low: IntValue<'ctx>,
    high: IntValue<'ctx>,
}

impl<'ctx> LoopClasses<'ctx> {
    fn find(&self, item: &MirExpr) -> Option<Prepared<'ctx>> {
        let address: *const MirExpr = item;
        self.prepared.borrow().iter().rev().find(|p| p.item == address).copied()
    }
}

/// The base and index of `item` when [`emit`] takes `item in container`:
/// a subscript of an `object` local by an `int` literal or a name, both
/// read without a reference of their own, and a container that is not a
/// boxed native value (a `frozenset[str]` name, which `emit_in_text_set`
/// tests natively).
fn operands<'e>(item: &'e MirExpr, container: &MirExpr) -> Option<(&'e MirExpr, &'e MirExpr)> {
    let MirExpr::ObjSubscript { base, index } = item else {
        return None;
    };
    let base_name = matches!(base.as_ref(), MirExpr::Name { ty: Ty::Object, name } if name != crate::ext::FALLBACK_NS_NAME)
        && crate::heap_rc::is_borrowed_read(base);
    let index_simple = matches!(index.as_ref(), MirExpr::IntLiteral(_))
        || (matches!(index.as_ref(), MirExpr::Name { ty: Ty::Object | Ty::Int, .. })
            && crate::heap_rc::is_borrowed_read(index));
    // A boxed native container (a `frozenset[str]` name, a display) keeps
    // its own lowering, unless it is a constant the module caches.
    let boxed = matches!(container, MirExpr::ObjectBox(_)) && !crate::const_container::is_constant_sequence(container);
    (base_name && index_simple && !boxed).then_some((base.as_ref(), index.as_ref()))
}

/// Whether [`emit`] takes `item in container`.
pub(crate) fn applies(item: &MirExpr, container: &MirExpr) -> bool {
    operands(item, container).is_some()
}

/// The name `expr` reads when it is a parameter's `object` slot: always
/// bound, holding a reference (not a tagged word), read with no effect.
fn parameter<'e, 'ctx>(
    rt: &RtFns<'ctx>,
    locals: &HashMap<String, StorageSlot<'ctx>>,
    expr: &'e MirExpr,
) -> Option<&'e str> {
    let MirExpr::Name { ty: Ty::Object, name } = expr else {
        return None;
    };
    let slot = locals.get(name)?;
    (name != crate::ext::FALLBACK_NS_NAME
        && slot.initialized.is_none()
        && crate::heap_rc::is_borrowed_read(expr)
        && crate::tagged_locals::tagged_read(rt, locals, expr).is_none())
    .then_some(name.as_str())
}

/// Before the `while` loop `test`/`body`, emitted at the current position
/// (the loop's preheader): when `test` is `s[i] in c` (or `not in`) that
/// [`emit`] takes, with `s` and `c` parameters that `body` only reads, and
/// the loop cannot suspend (a generator's `yield`), reads `s`'s and `c`'s
/// facts once ([`Prepared`]) for the test to use. Returns whether it did;
/// the caller then calls [`finish_loop`] after the loop's body.
#[allow(clippy::too_many_arguments)]
pub(crate) fn prepare_loop<'ctx>(
    context: &'ctx Context,
    builder: &Builder<'ctx>,
    module: &inkwell::module::Module<'ctx>,
    rt: &RtFns<'ctx>,
    user_functions: &HashMap<&str, UserFunction<'ctx>>,
    locals: &HashMap<String, StorageSlot<'ctx>>,
    test: &MirExpr,
    body: &[MirStmt],
) -> bool {
    let MirExpr::ObjContains { item, container, .. } = test else {
        return false;
    };
    let Some((base, _)) = operands(item, container) else {
        return false;
    };
    let (Some(text), Some(set)) = (parameter(rt, locals, base), parameter(rt, locals, container)) else {
        return false;
    };
    if text == set
        || !crate::tagged_locals::read_only(body, text)
        || !crate::tagged_locals::read_only(body, set)
        || pycc_mir::body_yields(body)
    {
        return false;
    }
    let i64_type = context.i64_type();
    let ptr = context.ptr_type(inkwell::AddressSpace::default());
    let facts_type = context.struct_type(
        &[ptr.into(), i64_type.into(), i64_type.into(), i64_type.into()],
        false,
    );
    let facts = |values: [BasicValueEnum<'ctx>; 4]| {
        let mut value = facts_type.get_undef();
        for (at, field) in values.into_iter().enumerate() {
            value = builder
                .build_insert_value(value, field, at as u32, "char_class_fact")
                .expect("insertvalue")
                .into_struct_value();
        }
        value.into()
    };
    let read = |expr: &MirExpr| expect_object(emit_expr(context, builder, module, rt, user_functions, locals, expr));
    let base_object = read(base);
    let set_object = read(container);
    let value = crate::fast_ops::guarded(
        context,
        builder,
        module,
        "char_class_loop",
        |fast| {
            let length = fast.ascii_str(base_object, "char_class_loop_text");
            let data = fast.ascii_data(base_object, "char_class_loop_text");
            let (low, high) = fast.ascii_mask_words(set_object, "char_class_loop_mask");
            facts([
                data.into(),
                length.into(),
                low.into(),
                high.into(),
            ])
        },
        || {
            facts([
                ptr.const_null().into(),
                i64_type.const_zero().into(),
                i64_type.const_zero().into(),
                i64_type.const_zero().into(),
            ])
        },
    )
    .into_struct_value();
    let field = |at: u32, name: &str| builder.build_extract_value(value, at, name).expect("extractvalue");
    let prepared = Prepared {
        item: item.as_ref(),
        data: field(0, "char_class_data").into_pointer_value(),
        length: field(1, "char_class_length").into_int_value(),
        low: field(2, "char_class_low").into_int_value(),
        high: field(3, "char_class_high").into_int_value(),
    };
    rt.char_classes.prepared.borrow_mut().push(prepared);
    true
}

/// Ends the loop [`prepare_loop`] prepared last.
pub(crate) fn finish_loop(rt: &RtFns<'_>) {
    rt.char_classes.prepared.borrow_mut().pop();
}

/// The `i64` value of the subscript's `index` on `fast`'s path, requiring
/// a small `int`.
#[allow(clippy::too_many_arguments)]
fn index_value<'ctx>(
    context: &'ctx Context,
    builder: &Builder<'ctx>,
    module: &inkwell::module::Module<'ctx>,
    rt: &RtFns<'ctx>,
    user_functions: &HashMap<&str, UserFunction<'ctx>>,
    locals: &HashMap<String, StorageSlot<'ctx>>,
    fast: &crate::fast_ops::Fast<'_, 'ctx>,
    index: &MirExpr,
) -> IntValue<'ctx> {
    match index {
        MirExpr::Name { ty: Ty::Int, .. } => {
            let Scalar::Int(word) = emit_expr(context, builder, module, rt, user_functions, locals, index) else {
                unreachable!("an int name reads as an int")
            };
            fast.small_word(word, "char_class_index")
        }
        _ => crate::tagged_locals::int_operand(
            context,
            builder,
            module,
            rt,
            user_functions,
            locals,
            fast,
            index,
            "char_class_index",
        ),
    }
}

/// `item in container` (`not in` when `negate`) as a `Scalar::Bool`, for
/// an `item` and `container` that [`applies`] admits (module docs).
#[allow(clippy::too_many_arguments)]
pub(crate) fn emit<'ctx>(
    context: &'ctx Context,
    builder: &Builder<'ctx>,
    module: &inkwell::module::Module<'ctx>,
    rt: &RtFns<'ctx>,
    user_functions: &HashMap<&str, UserFunction<'ctx>>,
    locals: &HashMap<String, StorageSlot<'ctx>>,
    negate: bool,
    item: &MirExpr,
    container: &MirExpr,
) -> Scalar<'ctx> {
    let Some(prepared) = rt.char_classes.find(item) else {
        return emit_each(context, builder, module, rt, user_functions, locals, negate, item, container);
    };
    let (_, index) = operands(item, container).expect("char_class::applies admitted the test");
    let i8_type = context.i8_type();
    let i64_type = context.i64_type();
    let answer = crate::fast_ops::guarded(
        context,
        builder,
        module,
        "char_class_prepared",
        |fast| {
            // A negative index, or any index when nothing is ready, takes
            // the test's own path.
            let at = index_value(context, builder, module, rt, user_functions, locals, fast, index);
            let within = builder
                .build_int_compare(IntPredicate::ULT, at, prepared.length, "char_class_within")
                .expect("icmp");
            fast.require(within, "char_class_within");
            let byte = fast.data_byte(prepared.data, at, "char_class_byte");
            let upper = builder
                .build_int_compare(IntPredicate::UGE, byte, i64_type.const_int(64, false), "char_class_upper")
                .expect("icmp");
            let word = builder
                .build_select(upper, prepared.high, prepared.low, "char_class_word")
                .expect("select")
                .into_int_value();
            let bit = builder
                .build_and(byte, i64_type.const_int(63, false), "char_class_bit")
                .expect("and");
            let shifted = builder.build_right_shift(word, bit, false, "char_class_shifted").expect("lshr");
            let member = builder.build_int_truncate(shifted, i8_type, "char_class_member").expect("trunc");
            let member = builder
                .build_and(member, i8_type.const_int(1, false), "char_class_member_bit")
                .expect("and");
            builder
                .build_xor(member, i8_type.const_int(u64::from(negate), false), "char_class_prepared_answer")
                .expect("xor")
                .into()
        },
        || {
            let Scalar::Bool(answer) =
                emit_each(context, builder, module, rt, user_functions, locals, negate, item, container)
            else {
                unreachable!("a membership test gives a bool")
            };
            answer.into()
        },
    );
    Scalar::Bool(answer.into_int_value())
}

/// [`emit`] with nothing prepared: the test's own checks at each test.
#[allow(clippy::too_many_arguments)]
fn emit_each<'ctx>(
    context: &'ctx Context,
    builder: &Builder<'ctx>,
    module: &inkwell::module::Module<'ctx>,
    rt: &RtFns<'ctx>,
    user_functions: &HashMap<&str, UserFunction<'ctx>>,
    locals: &HashMap<String, StorageSlot<'ctx>>,
    negate: bool,
    item: &MirExpr,
    container: &MirExpr,
) -> Scalar<'ctx> {
    let (base, index) = operands(item, container).expect("char_class::applies admitted the test");
    let i64_type = context.i64_type();
    let ptr = context.ptr_type(inkwell::AddressSpace::default());

    // The item: whether it is a character, its code, or the subscript's
    // object (a new reference), as `{i1, i64, ptr}`; LLVM splits the merge
    // into one value each, so the character's path branches to the bit
    // test with nothing to decode.
    let item_type = context.struct_type(&[context.bool_type().into(), i64_type.into(), ptr.into()], false);
    let item_value = |is_char: bool, byte: IntValue<'ctx>, object: PointerValue<'ctx>| {
        let value = item_type.get_undef();
        let value = builder
            .build_insert_value(value, context.bool_type().const_int(u64::from(is_char), false), 0, "char_class_kind")
            .expect("insertvalue");
        let value = builder.build_insert_value(value, byte, 1, "char_class_code").expect("insertvalue");
        builder
            .build_insert_value(value, object, 2, "char_class_object")
            .expect("insertvalue")
            .into_struct_value()
            .into()
    };
    let base_object = expect_object(emit_expr(context, builder, module, rt, user_functions, locals, base));
    let item_value = crate::fast_ops::guarded(
        context,
        builder,
        module,
        "char_class_item",
        |fast| {
            let length = fast.ascii_str(base_object, "char_class_base");
            let at = index_value(context, builder, module, rt, user_functions, locals, fast, index);
            let at = fast.index_within(at, length, "char_class_index");
            let byte = fast.ascii_byte(base_object, at, "char_class_byte");
            item_value(true, byte, ptr.const_null())
        },
        || {
            let object = expect_object(emit_expr(context, builder, module, rt, user_functions, locals, item));
            item_value(false, i64_type.const_zero(), object)
        },
    )
    .into_struct_value();
    let is_char = builder
        .build_extract_value(item_value, 0, "char_class_is_char")
        .expect("extractvalue")
        .into_int_value();
    let byte = builder
        .build_extract_value(item_value, 1, "char_class_code")
        .expect("extractvalue")
        .into_int_value();
    let owned = builder
        .build_extract_value(item_value, 2, "char_class_owned")
        .expect("extractvalue")
        .into_pointer_value();
    // Released by every failure edge of the container's evaluation.
    let held_item = crate::object_release::hold_new_reference(context, module, rt, owned);

    // The container, then the test.
    let constant = crate::const_container::is_constant_sequence(container);
    let container_scalar = if constant {
        Scalar::Object(crate::const_container::emit_cached_constant(
            context,
            builder,
            module,
            rt,
            user_functions,
            locals,
            container,
        ))
    } else {
        emit_expr(context, builder, module, rt, user_functions, locals, container)
    };
    let held_container = (!constant).then(|| crate::object_release::hold(context, module, rt, container, &container_scalar));
    held_item.consumed(rt);

    let function = builder
        .get_insert_block()
        .and_then(|block| block.get_parent())
        .expect("a membership test is emitted inside a function body");
    let char_block = context.append_basic_block(function, "char_class_char");
    let generic = context.append_basic_block(function, "char_class_generic");
    let join = context.append_basic_block(function, "char_class_join");
    let entry = builder.get_insert_block().expect("positioned");
    let branch = builder
        .build_conditional_branch(is_char, char_block, generic)
        .expect("build_conditional_branch should not fail for an i1");
    crate::int_fast::weight_branch(context, branch, true);

    // A character against an exact `frozenset` with a usable mask: 0 or 1;
    // otherwise 2, its object made for the generic test.
    builder.position_at_end(char_block);
    let status = match container_scalar {
        Scalar::Object(set) => crate::fast_ops::guarded(
            context,
            builder,
            module,
            "char_class_mask",
            |fast| {
                builder
                    .build_int_z_extend(fast.ascii_byte_member(set, byte, "char_class_mask"), i64_type, "char_class_member")
                    .expect("zext")
                    .into()
            },
            || i64_type.const_int(2, false).into(),
        )
        .into_int_value(),
        _ => i64_type.const_int(2, false),
    };
    let answered = builder
        .build_int_compare(IntPredicate::ULT, status, i64_type.const_int(2, false), "char_class_answered")
        .expect("icmp");
    let answer_block = context.append_basic_block(function, "char_class_answer");
    let boxing = context.append_basic_block(function, "char_class_box");
    let branch = builder
        .build_conditional_branch(answered, answer_block, boxing)
        .expect("build_conditional_branch should not fail for an i1");
    crate::int_fast::weight_branch(context, branch, true);
    builder.position_at_end(answer_block);
    let member = builder
        .build_int_truncate(status, context.i8_type(), "char_class_bool")
        .expect("trunc");
    let fast_answer = builder
        .build_xor(member, context.i8_type().const_int(u64::from(negate), false), "char_class_fast_answer")
        .expect("xor");
    builder.build_unconditional_branch(join).expect("branch");

    builder.position_at_end(boxing);
    let edge = ForeignFailEdge::for_current(builder);
    let code = builder
        .build_int_truncate(byte, context.i32_type(), "char_class_code32")
        .expect("trunc");
    let from_ordinal = crate::foreign_pack::shim_fn(
        module,
        "PyUnicode_FromOrdinal",
        ptr.fn_type(&[context.i32_type().into()], false),
    );
    let boxed = builder
        .build_call(from_ordinal, &[code.into()], "char_class_boxed")
        .expect("build_call should not fail for PyUnicode_FromOrdinal")
        .try_as_basic_value()
        .expect_basic("PyUnicode_FromOrdinal returns PyObject *")
        .into_pointer_value();
    route_null(context, builder, module, rt, edge, boxed, "char_class_box");
    let boxed_end = builder.get_insert_block().expect("positioned");
    builder.build_unconditional_branch(generic).expect("branch");

    // The generic test of the subscript's or the boxed character's object.
    builder.position_at_end(generic);
    let object = builder.build_phi(ptr, "char_class_item").expect("phi");
    object.add_incoming(&[(&owned, entry), (&boxed, boxed_end)]);
    let object = object.as_basic_value().into_pointer_value();
    let held_object = crate::object_release::hold_new_reference(context, module, rt, object);
    let Scalar::Bool(generic_answer) =
        crate::foreign_compare::emit_contains(context, builder, module, rt, negate, Scalar::Object(object), container_scalar)
    else {
        unreachable!("a membership test gives a bool")
    };
    held_object.release(builder, rt);
    let generic_end = builder.get_insert_block().expect("positioned");
    builder.build_unconditional_branch(join).expect("branch");

    builder.position_at_end(join);
    let answer = builder.build_phi(context.i8_type(), "char_class_answer").expect("phi");
    answer.add_incoming(&[(&fast_answer, answer_block), (&generic_answer, generic_end)]);
    if let Some(held) = held_container {
        held.release(builder, rt);
    }
    Scalar::Bool(answer.as_basic_value().into_int_value())
}
