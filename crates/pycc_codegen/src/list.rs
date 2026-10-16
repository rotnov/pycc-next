//! `list[T]` emission over `pycc_rt::list::PyListObj`, the one runtime
//! object every list uses whatever its element type.
//!
//! An element crosses into the list as one 64-bit word in its native
//! representation (`element_word`) and comes back out by the declared
//! element type (`word_element`) -- the same word encoding an instance slot
//! uses (`attr_slot.rs`), except that an `int` element keeps D-141's encoded
//! word, bool markers included, so `xs.append(True)` on a `list[int]` still
//! prints `True`. The runtime records the element kind (`list_kind`) so its
//! generic operations can compare, render and release words.
//!
//! The hot paths -- `len(xs)`, `xs[i]`, `xs[i] = v` and iteration -- are
//! inlined against `PyListObj`'s `#[repr(C)]` prefix `{ i64 len, ptr data }`
//! instead of calling the runtime, with the out-of-range `IndexError` on a
//! cold branch straight to the innermost exception target.

use super::*;
use crate::attr_slot::{scalar_to_slot_word, slot_word_to_scalar};
use crate::exception::jump_to_exception_target;
use inkwell::values::BasicValue as _;
use pycc_mir::Ty;

/// `pycc_rt::LIST_KIND_*` for a list whose elements are `elem`.
pub(crate) fn list_kind(rt: &RtFns<'_>, elem: &Ty) -> u64 {
    crate::containers::word_kind(rt, elem)
}

/// The element type of a `list[T]`-typed MIR value.
pub(crate) fn element_ty(list_ty: &Ty) -> &Ty {
    let (Ty::List(elem) | Ty::VarTuple(elem)) = list_ty else {
        panic!(
            "pycc_codegen: internal error: expected a list, got `{}`",
            list_ty.name()
        )
    };
    elem
}

/// `pycc_rt_list_new(kind, capacity)`.
pub(crate) fn build_list_new<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    rt: &RtFns<'ctx>,
    elem: &Ty,
    capacity: u64,
) -> PointerValue<'ctx> {
    let i64_type = context.i64_type();
    builder
        .build_call(
            rt.list_new,
            &[
                i64_type.const_int(list_kind(rt, elem), false).into(),
                i64_type.const_int(capacity, false).into(),
            ],
            "list_new",
        )
        .expect("build_call should not fail for a well-formed list construction")
        .try_as_basic_value()
        .expect_basic("pycc_rt_list_new returns a non-void pointer")
        .into_pointer_value()
}

/// `pycc_rt_list_append(list, word)`; the list adopts `word`'s reference.
pub(crate) fn build_list_append<'ctx>(
    builder: &inkwell::builder::Builder<'ctx>,
    rt: &RtFns<'ctx>,
    list: PointerValue<'ctx>,
    word: IntValue<'ctx>,
) {
    builder
        .build_call(rt.list_append, &[list.into(), word.into()], "")
        .expect("build_call should not fail for a well-formed list append");
}

/// Branches on `(word & 1) != 0`: the inline smallint case takes `fast`,
/// every other encoded word (a bool marker, or a bigint, which
/// `pycc_rt_int_untag_checked` raises for, by `use_kind`) takes
/// `build_untag_checked`.
fn untag_or_check<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    rt: &RtFns<'ctx>,
    use_kind: UntagUse,
    word: IntValue<'ctx>,
) -> IntValue<'ctx> {
    let i64_type = context.i64_type();
    let function = builder.get_insert_block().unwrap().get_parent().unwrap();
    let small = builder
        .build_and(word, i64_type.const_int(1, false), "small_tag")
        .expect("build_and should not fail for two i64 operands");
    let is_small = builder
        .build_int_compare(IntPredicate::NE, small, i64_type.const_zero(), "is_small")
        .expect("build_int_compare should not fail comparing two i64 operands");
    let entry = builder.get_insert_block().unwrap();
    let slow = context.append_basic_block(function, "untag_slow");
    let join = context.append_basic_block(function, "untag_join");
    let fast = builder
        .build_right_shift(word, i64_type.const_int(1, false), true, "untag_fast")
        .expect("build_right_shift should not fail for a constant shift amount");
    builder
        .build_conditional_branch(is_small, join, slow)
        .expect("build_conditional_branch should not fail for a well-formed i1 condition");
    builder.position_at_end(slow);
    let checked = build_untag_checked(context, builder, rt, use_kind, word, "untag_slow_value");
    builder
        .build_unconditional_branch(join)
        .expect("build_unconditional_branch should not fail");
    let slow_end = builder.get_insert_block().unwrap();
    builder.position_at_end(join);
    let phi = builder
        .build_phi(i64_type, "untagged")
        .expect("build_phi should not fail in a fresh join block");
    phi.add_incoming(&[(&fast, entry), (&checked, slow_end)]);
    phi.as_basic_value().into_int_value()
}

/// Decodes an `int`/`bool` index scalar to a raw `i64` position.
pub(crate) fn raw_index<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    rt: &RtFns<'ctx>,
    index: Scalar<'ctx>,
) -> IntValue<'ctx> {
    let encoded = to_numeric_encoded_int(context, builder, index);
    untag_or_check(context, builder, rt, UntagUse::Index, encoded)
}

/// The bigint-exclusion check every container ingress applies to an
/// encoded `int` word (`OverflowError` beyond the inline range).
pub(crate) fn check_element_int<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    rt: &RtFns<'ctx>,
    encoded: IntValue<'ctx>,
) {
    let _ = untag_or_check(context, builder, rt, UntagUse::Element, encoded);
}

/// Converts an element `scalar` (the value of `source`) into the word a
/// container of `elem` stores, taking the reference the container will own:
/// a borrowed counted value is retained, and a struct tuple is boxed into a
/// new tuple object. An `int` element keeps its D-141 encoded word after the
/// bigint-exclusion check every container ingress applies.
pub(crate) fn element_word<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    rt: &RtFns<'ctx>,
    elem: &Ty,
    source: &MirExpr,
    scalar: Scalar<'ctx>,
) -> IntValue<'ctx> {
    match (elem, scalar) {
        (Ty::Int, scalar) => {
            let encoded = to_encoded_int(context, builder, scalar);
            check_element_int(context, builder, rt, encoded);
            encoded
        }
        (Ty::Tuple(elements), Scalar::Tuple(tuple)) => {
            let boxed = crate::containers::box_struct_tuple(context, builder, rt, elements, tuple);
            builder
                .build_ptr_to_int(boxed, context.i64_type(), "boxed_tuple_word")
                .expect("build_ptr_to_int should not fail for a pointer")
        }
        // A counted element (`str`, list, tuple object, instance): the
        // container owns one reference, so a borrowed one is retained.
        (_, scalar) => {
            let scalar =
                crate::heap_rc::retain_if_heap_duplicate(context, builder, rt, source, scalar);
            scalar_to_slot_word(context, builder, scalar)
        }
    }
}

/// The element a stored word denotes, by the container's element type. A
/// counted element comes back borrowed (`heap_rc::is_borrowed_read` classifies
/// a container read as such); a struct tuple is unboxed by value.
pub(crate) fn word_element<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    elem: &Ty,
    word: IntValue<'ctx>,
) -> Scalar<'ctx> {
    match elem {
        Ty::Int => Scalar::Int(word),
        Ty::Tuple(elements) => {
            let ptr = builder
                .build_int_to_ptr(
                    word,
                    context.ptr_type(inkwell::AddressSpace::default()),
                    "tuple_word_ptr",
                )
                .expect("build_int_to_ptr should not fail reinterpreting an i64 as a pointer");
            if pycc_hir::tuple_is_boxed(elements) {
                Scalar::List(ptr)
            } else {
                Scalar::Tuple(crate::containers::unbox_struct_tuple(
                    context, builder, elements, ptr,
                ))
            }
        }
        other => slot_word_to_scalar(context, builder, word, other),
    }
}

/// A pointer to field `field` of `PyListObj`'s `#[repr(C)]` prefix.
fn field_ptr<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    list: PointerValue<'ctx>,
    field: u32,
) -> PointerValue<'ctx> {
    let ptr_type = context.ptr_type(inkwell::AddressSpace::default());
    let header = context.struct_type(&[context.i64_type().into(), ptr_type.into()], false);
    builder
        .build_struct_gep(header, list, field, "list_field")
        .expect("build_struct_gep should not fail for a field of the list header")
}

/// `len(xs)` as a raw `i64`: a load of `PyListObj.len`.
pub(crate) fn build_list_len<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    list: PointerValue<'ctx>,
) -> IntValue<'ctx> {
    let len = builder
        .build_load(
            context.i64_type(),
            field_ptr(context, builder, list, 0),
            "list_len",
        )
        .expect("build_load should not fail for the list header");
    tag(
        context,
        len.as_instruction_value(),
        crate::tbaa::list_field_name(0),
    );
    len.into_int_value()
}

/// Tags a list access for `tbaa` (see `crate::tbaa`).
fn tag<'ctx>(
    context: &'ctx Context,
    instruction: Option<inkwell::values::InstructionValue<'ctx>>,
    name: &str,
) {
    let instruction = instruction.expect("a load or store is an instruction");
    crate::tbaa::tag(context, instruction, name);
}

/// The in-memory element type of a list of `elem`: a byte for `list[bool]`
/// (`pycc_rt::list`'s storage contract), an `i64` word otherwise.
fn storage_type<'ctx>(context: &'ctx Context, elem: &Ty) -> inkwell::types::IntType<'ctx> {
    if *elem == Ty::Bool {
        context.i8_type()
    } else {
        context.i64_type()
    }
}

/// A pointer to element `position` (already known in range).
fn element_ptr<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    elem: &Ty,
    list: PointerValue<'ctx>,
    position: IntValue<'ctx>,
) -> PointerValue<'ctx> {
    let ptr_type = context.ptr_type(inkwell::AddressSpace::default());
    let data = builder
        .build_load(ptr_type, field_ptr(context, builder, list, 1), "list_data")
        .expect("build_load should not fail for the list header");
    tag(
        context,
        data.as_instruction_value(),
        crate::tbaa::list_field_name(1),
    );
    let data = data.into_pointer_value();
    unsafe {
        builder
            .build_in_bounds_gep(storage_type(context, elem), data, &[position], "list_slot")
            .expect("build_in_bounds_gep should not fail for a list element")
    }
}

/// The word at `position`, which the caller guarantees is in range
/// (`for x in xs` reads below the length it has just compared against).
pub(crate) fn build_list_get_unchecked<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    elem: &Ty,
    list: PointerValue<'ctx>,
    position: IntValue<'ctx>,
) -> IntValue<'ctx> {
    let storage = storage_type(context, elem);
    let stored = builder
        .build_load(
            storage,
            element_ptr(context, builder, elem, list, position),
            "list_word",
        )
        .expect("build_load should not fail for a list element");
    tag(
        context,
        stored.as_instruction_value(),
        crate::tbaa::list_element_name(storage.get_bit_width()),
    );
    let stored = stored.into_int_value();
    builder
        .build_int_z_extend_or_bit_cast(stored, context.i64_type(), "list_word_ext")
        .expect("zext of a stored element")
}

/// Resolves a Python index against the list's length (a negative index
/// counts from the end) and returns the in-range position. An index outside
/// `[-len, len)` raises CPython's `IndexError` -- `store` picks the store
/// message -- on a cold branch to the innermost exception target.
fn resolve_index<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    rt: &RtFns<'ctx>,
    list: PointerValue<'ctx>,
    index: IntValue<'ctx>,
    store: bool,
) -> IntValue<'ctx> {
    let i64_type = context.i64_type();
    let len = build_list_len(context, builder, list);
    let negative = builder
        .build_int_compare(
            IntPredicate::SLT,
            index,
            i64_type.const_zero(),
            "index_negative",
        )
        .expect("build_int_compare should not fail comparing two i64 operands");
    let from_end = builder
        .build_int_add(index, len, "index_from_end")
        .expect("build_int_add should not fail for two i64 operands");
    let position = builder
        .build_select(negative, from_end, index, "index_position")
        .expect("build_select should not fail for two i64 operands")
        .into_int_value();
    let in_range = builder
        .build_int_compare(IntPredicate::ULT, position, len, "index_in_range")
        .expect("build_int_compare should not fail comparing two i64 operands");
    let function = builder.get_insert_block().unwrap().get_parent().unwrap();
    let ok = context.append_basic_block(function, "index_ok");
    let out_of_range = context.append_basic_block(function, "index_out_of_range");
    builder
        .build_conditional_branch(in_range, ok, out_of_range)
        .expect("build_conditional_branch should not fail for a well-formed i1 condition");
    builder.position_at_end(out_of_range);
    builder
        .build_call(
            rt.list_index_error,
            &[list.into(), i64_type.const_int(u64::from(store), false).into()],
            "",
        )
        .expect("build_call should not fail for a declared runtime function");
    jump_to_exception_target(context, builder, rt);
    builder.position_at_end(ok);
    position
}

/// `xs[index]`: the borrowed word at a Python index.
pub(crate) fn build_list_get<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    rt: &RtFns<'ctx>,
    elem: &Ty,
    list: PointerValue<'ctx>,
    index: IntValue<'ctx>,
) -> IntValue<'ctx> {
    let position = resolve_index(context, builder, rt, list, index, false);
    build_list_get_unchecked(context, builder, elem, list, position)
}

/// `xs[index] = word`. A `str` list goes through `pycc_rt_list_set`, which
/// also releases the overwritten element; every other kind holds no
/// reference, so the store is inlined.
pub(crate) fn build_list_set<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    rt: &RtFns<'ctx>,
    elem: &Ty,
    list: PointerValue<'ctx>,
    index: IntValue<'ctx>,
    word: IntValue<'ctx>,
) {
    // A counted element goes through the runtime, which releases the
    // element it overwrites.
    if crate::heap_rc::heap_kind(rt, elem).is_some() {
        builder
            .build_call(rt.list_set, &[list.into(), index.into(), word.into()], "")
            .expect("build_call should not fail for a declared runtime function");
        guard_statement_effects(context, builder, rt);
        return;
    }
    let position = resolve_index(context, builder, rt, list, index, true);
    let storage = storage_type(context, elem);
    let stored = builder
        .build_int_truncate_or_bit_cast(word, storage, "list_store")
        .expect("trunc of an element word");
    let store = builder
        .build_store(element_ptr(context, builder, elem, list, position), stored)
        .expect("build_store should not fail for a list element");
    tag(
        context,
        Some(store),
        crate::tbaa::list_element_name(storage.get_bit_width()),
    );
}

/// `if xs:` -- a list is truthy exactly when it is non-empty.
pub(crate) fn list_truthy<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    list: PointerValue<'ctx>,
) -> IntValue<'ctx> {
    let len = build_list_len(context, builder, list);
    let nonempty = builder
        .build_int_compare(
            IntPredicate::NE,
            len,
            context.i64_type().const_zero(),
            "list_truthy",
        )
        .expect("build_int_compare should not fail comparing two i64 operands");
    builder
        .build_int_z_extend(nonempty, context.i8_type(), "list_truthy_i8")
        .expect("build_int_z_extend should not fail widening i1 to i8")
}

/// Calls a pointer-returning list runtime function.
fn call_list<'ctx>(
    builder: &inkwell::builder::Builder<'ctx>,
    callee: FunctionValue<'ctx>,
    args: &[inkwell::values::BasicMetadataValueEnum<'ctx>],
) -> PointerValue<'ctx> {
    builder
        .build_call(callee, args, "list_result")
        .expect("build_call should not fail for a declared runtime function")
        .try_as_basic_value()
        .expect_basic("the list runtime function returns a pointer")
        .into_pointer_value()
}

/// `a + b` (two lists) and `xs * n` / `n * xs`.
pub(crate) fn emit_list_binop<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    rt: &RtFns<'ctx>,
    op: pycc_mir::BinOpKind,
    left: Scalar<'ctx>,
    right: Scalar<'ctx>,
) -> Scalar<'ctx> {
    let result = match (op, left, right) {
        (pycc_mir::BinOpKind::Add, Scalar::List(a), Scalar::List(b)) => {
            call_list(builder, rt.list_concat, &[a.into(), b.into()])
        }
        (pycc_mir::BinOpKind::Mul, Scalar::List(list), count)
        | (pycc_mir::BinOpKind::Mul, count, Scalar::List(list)) => {
            let count = raw_index(context, builder, rt, count);
            call_list(builder, rt.list_repeat, &[list.into(), count.into()])
        }
        _ => panic!(
            "pycc_codegen: internal error: list operator {op:?} -- pycc_types::list_ops should \
             have rejected this before codegen"
        ),
    };
    Scalar::List(result)
}

/// `xs[start:stop:step]` as a new list, CPython's slice semantics in
/// `pycc_rt_list_slice`; `present` says which bounds were written.
pub(crate) fn build_list_slice<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    rt: &RtFns<'ctx>,
    list: PointerValue<'ctx>,
    bounds: [Option<IntValue<'ctx>>; 3],
) -> PointerValue<'ctx> {
    let i64_type = context.i64_type();
    let mut present = 0u64;
    let mut args: Vec<inkwell::values::BasicMetadataValueEnum<'ctx>> = vec![list.into()];
    for (bit, bound) in bounds.iter().enumerate() {
        if bound.is_some() {
            present |= 1 << bit;
        }
        args.push(bound.unwrap_or_else(|| i64_type.const_zero()).into());
    }
    args.push(i64_type.const_int(present, false).into());
    call_list(builder, rt.list_slice, &args)
}

/// `xs.pop()` / `xs.pop(i)`, handing the element's reference to the caller.
pub(crate) fn build_list_pop<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    rt: &RtFns<'ctx>,
    list: PointerValue<'ctx>,
    index: Option<IntValue<'ctx>>,
) -> IntValue<'ctx> {
    let i64_type = context.i64_type();
    let has_index = i64_type.const_int(u64::from(index.is_some()), false);
    let index = index.unwrap_or_else(|| i64_type.const_zero());
    builder
        .build_call(
            rt.list_pop,
            &[list.into(), has_index.into(), index.into()],
            "list_pop",
        )
        .expect("build_call should not fail for a declared runtime function")
        .try_as_basic_value()
        .expect_basic("pycc_rt_list_pop returns i64")
        .into_int_value()
}

/// `xs.insert(index, word)`; the list adopts `word`'s reference.
pub(crate) fn build_list_insert<'ctx>(
    builder: &inkwell::builder::Builder<'ctx>,
    rt: &RtFns<'ctx>,
    list: PointerValue<'ctx>,
    index: IntValue<'ctx>,
    word: IntValue<'ctx>,
) {
    builder
        .build_call(
            rt.list_insert,
            &[list.into(), index.into(), word.into()],
            "",
        )
        .expect("build_call should not fail for a declared runtime function");
}
