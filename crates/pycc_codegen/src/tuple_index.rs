//! `t[k]` with a computed index `k` on a struct tuple whose elements share
//! one type (`int`, `bool` or `float`): the index is resolved against the
//! tuple's length as CPython resolves it (a negative one counts from the
//! end; one outside `[-len, len)` raises `IndexError: tuple index out of
//! range` on a cold branch), and the field is picked with a chain of
//! `select`s over the extracted fields -- no memory, no runtime call.

use super::*;
use inkwell::IntPredicate;
use inkwell::values::{IntValue, StructValue};
use pycc_mir::Ty;

/// The element of `tuple` (fields `elements`, all of one type) at the
/// Python index `index` (an already decoded raw `i64`).
pub(crate) fn emit_dynamic_tuple_element<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    rt: &RtFns<'ctx>,
    elements: &[Ty],
    tuple: StructValue<'ctx>,
    index: IntValue<'ctx>,
) -> Scalar<'ctx> {
    assert!(
        elements.iter().all(|element| *element == elements[0]),
        "pycc_codegen: internal error: a computed index into a struct tuple of mixed element \
         types -- pycc_types admits only one int, bool or float element type"
    );
    let i64_type = context.i64_type();
    let len = i64_type.const_int(elements.len() as u64, false);
    let negative = builder
        .build_int_compare(
            IntPredicate::SLT,
            index,
            i64_type.const_zero(),
            "tuple_index_neg",
        )
        .expect("build_int_compare should not fail comparing two i64 operands");
    let from_end = builder
        .build_int_add(index, len, "tuple_index_from_end")
        .expect("build_int_add should not fail for two i64 operands");
    let position = builder
        .build_select(negative, from_end, index, "tuple_index_position")
        .expect("build_select should not fail for two i64 operands")
        .into_int_value();
    let in_range = builder
        .build_int_compare(IntPredicate::ULT, position, len, "tuple_index_in_range")
        .expect("build_int_compare should not fail comparing two i64 operands");
    let function = builder
        .get_insert_block()
        .and_then(|block| block.get_parent())
        .expect("a tuple is indexed inside a function");
    let ok = context.append_basic_block(function, "tuple_index_ok");
    let out_of_range = context.append_basic_block(function, "tuple_index_out_of_range");
    builder
        .build_conditional_branch(in_range, ok, out_of_range)
        .expect("build_conditional_branch should not fail for a well-formed i1 condition");
    builder.position_at_end(out_of_range);
    builder
        .build_call(rt.tuple_index_error, &[], "")
        .expect("build_call should not fail for a declared runtime function");
    exception::jump_to_exception_target(context, builder, rt);
    builder.position_at_end(ok);
    let mut value = builder
        .build_extract_value(tuple, 0, "tuple_field")
        .expect("build_extract_value should not fail for field 0");
    for field in 1..elements.len() {
        let candidate = builder
            .build_extract_value(tuple, field as u32, "tuple_field")
            .expect("build_extract_value should not fail for an in-range field");
        let here = builder
            .build_int_compare(
                IntPredicate::EQ,
                position,
                i64_type.const_int(field as u64, false),
                "tuple_index_is",
            )
            .expect("build_int_compare should not fail comparing two i64 operands");
        value = builder
            .build_select(here, candidate, value, "tuple_index_pick")
            .expect("build_select should not fail for two values of one type");
    }
    match &elements[0] {
        Ty::Int => Scalar::Int(value.into_int_value()),
        Ty::Bool => Scalar::Bool(value.into_int_value()),
        Ty::Float => Scalar::Float(value.into_float_value()),
        other => panic!(
            "pycc_codegen: internal error: a computed index into a struct tuple of `{}` -- \
             pycc_types admits only int, bool and float elements",
            other.name()
        ),
    }
}
