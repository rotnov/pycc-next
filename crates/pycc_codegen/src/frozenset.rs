//! Emission for `frozenset(...)` and for a `set[int]`/`frozenset[int]`
//! value's truthiness (Part 1 of #1319).
//!
//! `frozenset[int]` shares `set[int]`'s whole runtime representation -- a
//! `PyIntSetObj` pointer carried as `Scalar::Set` -- because the two differ
//! only in mutability, which `pycc_types` enforces before codegen runs. So
//! the constructor is the one new emission: it picks the runtime copy
//! constructor from the source's static type.

use super::*;

/// Emits `MirExpr::FrozenSetFrom`: `frozenset()` allocates an empty set,
/// `frozenset(s)` for a `set[int]`/`frozenset[int]` source copies it with
/// `pycc_rt_int_set_copy`, and `frozenset(xs)` for a `list[int]` source
/// builds one with `pycc_rt_int_set_from_int_list`, whose dedup keeps each
/// value's first occurrence exactly as a set literal does. `pycc_types`
/// admits no other source type, so every non-list source is a set.
pub(super) fn emit_frozenset_from<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    module: &inkwell::module::Module<'ctx>,
    rt: &RtFns<'ctx>,
    user_functions: &HashMap<&str, UserFunction<'ctx>>,
    locals: &HashMap<String, StorageSlot<'ctx>>,
    source: Option<&MirExpr>,
) -> Scalar<'ctx> {
    let (callee, args): (_, Vec<inkwell::values::BasicMetadataValueEnum<'ctx>>) = match source {
        None => (rt.int_set_new, Vec::new()),
        Some(source) => {
            let scalar = emit_expr(context, builder, module, rt, user_functions, locals, source);
            match source.ty() {
                pycc_mir::Ty::List(_) => (
                    rt.int_set_from_int_list,
                    vec![expect_list_pointer(scalar, "`frozenset`'s argument").into()],
                ),
                _ => (
                    rt.int_set_copy,
                    vec![expect_set_pointer(scalar, "`frozenset`'s argument").into()],
                ),
            }
        }
    };
    let set_ptr = builder
        .build_call(callee, &args, "frozenset")
        .expect("build_call should not fail for a well-formed frozenset construction")
        .try_as_basic_value()
        .expect_basic("every frozenset constructor returns a non-void pointer")
        .into_pointer_value();
    Scalar::Set(set_ptr)
}

/// A `set[int]`/`frozenset[int]` value's truthiness, as the `i8` `truthy`
/// answers: CPython's `bool(s)` is `len(s) != 0`, so this reads the length
/// through the same `pycc_rt_int_set_len` call `len(s)` makes.
pub(super) fn set_truthy<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    rt: &RtFns<'ctx>,
    set_ptr: PointerValue<'ctx>,
) -> IntValue<'ctx> {
    let len = build_int_set_len(builder, rt, set_ptr);
    let nonempty = builder
        .build_int_compare(
            IntPredicate::NE,
            len,
            context.i64_type().const_zero(),
            "set_nonempty",
        )
        .expect("build_int_compare should not fail for two i64 operands");
    builder
        .build_int_z_extend(nonempty, context.i8_type(), "bool_from_set_truthy")
        .expect("build_int_z_extend should not fail widening i1 to i8")
}
