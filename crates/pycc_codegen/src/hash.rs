//! Emission for the `hash(x)` builtin (#1331, Part 1 of #1327).
//!
//! `pycc_types` admits exactly one argument, of type `int`, `bool` or a
//! tuple of `int`/`bool` elements, so the emitter picks one of three paths
//! from the argument's static type:
//!
//! - a `bool` is its own hash, `0` or `1`: the `i8` is widened, no call;
//! - an `int` (or anything `to_encoded_int` accepts) goes through
//!   `pycc_rt_hash_int`, which borrows the word;
//! - a tuple has each field's lane hash computed as above, spilled into one
//!   entry-block `i64` array, and folded by a single `pycc_rt_hash_tuple`.
//!
//! The raw `i64` hash then becomes an encoded int through
//! `pycc_rt_int_from_i64`, which births a heap bigint when a tuple hash
//! leaves the smallint range -- a fresh owned word, which is what
//! `bigint_rc` already classifies a `Call` result as (D-181).
//!
//! A class instance never reaches [`emit_hash`]: `pycc_mir` lowers
//! `hash(instance)` to `MirExpr::InstanceHash` (#1335, Part 1 of #1332),
//! which [`emit_instance_hash`] emits.

use super::*;

/// CPython's `PyObject_Hash` of the CPython object `object` (borrowed),
/// through the shim's `int pycc_ext_obj_hash(PyObject *, long long *)`: the
/// raw `i64` hash, or the foreign failure edge when the object raises
/// (an unhashable object's `TypeError`, a raising `__hash__`).
fn hash_object<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    module: &inkwell::module::Module<'ctx>,
    rt: &RtFns<'ctx>,
    object: PointerValue<'ctx>,
) -> IntValue<'ctx> {
    let edge = crate::foreign_fail::ForeignFailEdge::for_current(builder);
    let ptr = context.ptr_type(inkwell::AddressSpace::default());
    let i64_type = context.i64_type();
    let hash_fn = crate::foreign_pack::shim_fn(
        module,
        crate::ext::EXT_OBJ_HASH_SYMBOL,
        context
            .i32_type()
            .fn_type(&[ptr.into(), ptr.into()], false),
    );
    let out = build_at_entry_block(builder, edge.function(), |b| {
        b.build_alloca(i64_type, "foreign_hash_out")
            .expect("build_alloca should not fail")
    });
    let status = builder
        .build_call(hash_fn, &[object.into(), out.into()], "foreign_hash")
        .expect("build_call should not fail for pycc_ext_obj_hash")
        .try_as_basic_value()
        .expect_basic("pycc_ext_obj_hash returns int")
        .into_int_value();
    crate::foreign_fail::route_negative(context, builder, module, rt, edge, status, "foreign_hash");
    builder
        .build_load(i64_type, out, "foreign_hash_value")
        .expect("build_load should not fail")
        .into_int_value()
}

/// `hash(None)`: a constant since CPython 3.12 (`_Py_HashPointer` is no
/// longer used for `None`), the same in every process.
const NONE_HASH: u64 = 0xFCA8_6420;

/// `pycc_rt_hash_int(word)`, the raw `i64` hash of one encoded int.
fn hash_int_word<'ctx>(
    builder: &inkwell::builder::Builder<'ctx>,
    rt: &RtFns<'ctx>,
    word: IntValue<'ctx>,
) -> IntValue<'ctx> {
    builder
        .build_call(rt.hash_int, &[word.into()], "hash_int")
        .expect("build_call should not fail for pycc_rt_hash_int")
        .try_as_basic_value()
        .expect_basic("pycc_rt_hash_int returns an i64")
        .into_int_value()
}

/// `pycc_rt_hash_slot_int(word)`: CPython's `slot_tp_hash` reduction of a
/// user `__hash__`'s `int` result. Shared by `hash(instance)` and a set
/// insertion; each caller retires the word itself.
pub(super) fn hash_slot_int_word<'ctx>(
    builder: &inkwell::builder::Builder<'ctx>,
    rt: &RtFns<'ctx>,
    word: IntValue<'ctx>,
) -> IntValue<'ctx> {
    builder
        .build_call(rt.hash_slot_int, &[word.into()], "hash_slot_int")
        .expect("build_call should not fail for pycc_rt_hash_slot_int")
        .try_as_basic_value()
        .expect_basic("pycc_rt_hash_slot_int returns an i64")
        .into_int_value()
}

/// `pycc_rt_hash_pointer(pointer)`: `object.__hash__`, an instance's
/// identity hash. Shared by `hash(instance)` and a set insertion.
pub(super) fn hash_identity<'ctx>(
    builder: &inkwell::builder::Builder<'ctx>,
    rt: &RtFns<'ctx>,
    pointer: PointerValue<'ctx>,
) -> IntValue<'ctx> {
    builder
        .build_call(rt.hash_pointer, &[pointer.into()], "hash_pointer")
        .expect("build_call should not fail for pycc_rt_hash_pointer")
        .try_as_basic_value()
        .expect_basic("pycc_rt_hash_pointer returns an i64")
        .into_int_value()
}

/// A bool's hash: the `i8` `0`/`1` widened to `i64`.
pub(super) fn hash_bool<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    value: IntValue<'ctx>,
) -> IntValue<'ctx> {
    builder
        .build_int_z_extend(value, context.i64_type(), "hash_bool")
        .expect("build_int_z_extend should not fail widening i8 to i64")
}

/// `pycc_rt_hash_str(s)`, the raw hash of a borrowed `str`.
fn hash_str<'ctx>(
    builder: &inkwell::builder::Builder<'ctx>,
    rt: &RtFns<'ctx>,
    text: inkwell::values::PointerValue<'ctx>,
) -> IntValue<'ctx> {
    builder
        .build_call(rt.hash_str, &[text.into()], "hash_str")
        .expect("build_call should not fail for pycc_rt_hash_str")
        .try_as_basic_value()
        .expect_basic("pycc_rt_hash_str returns an i64")
        .into_int_value()
}

/// `pycc_rt_hash_float(x)`, the raw hash of a `float`.
fn hash_float<'ctx>(
    builder: &inkwell::builder::Builder<'ctx>,
    rt: &RtFns<'ctx>,
    value: inkwell::values::FloatValue<'ctx>,
) -> IntValue<'ctx> {
    builder
        .build_call(rt.hash_float, &[value.into()], "hash_float")
        .expect("build_call should not fail for pycc_rt_hash_float")
        .try_as_basic_value()
        .expect_basic("pycc_rt_hash_float returns an i64")
        .into_int_value()
}

/// The raw hash of a tuple value whose element types are `elements`. A
/// field is never released: a tuple field reader does not own its word
/// (D-182), and tuple fields are never released at all today (D-124).
fn hash_tuple<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    rt: &RtFns<'ctx>,
    elements: &[pycc_mir::Ty],
    tuple: inkwell::values::StructValue<'ctx>,
) -> IntValue<'ctx> {
    let i64_type = context.i64_type();
    let len = i64_type.const_int(elements.len() as u64, false);
    let function = builder
        .get_insert_block()
        .and_then(|block| block.get_parent())
        .expect("a `hash` call is emitted inside a function");
    let lanes = build_at_entry_block(builder, function, |b| {
        b.build_array_alloca(i64_type, len, "hash_lanes")
            .expect("build_array_alloca should not fail")
    });
    for (index, element) in elements.iter().enumerate() {
        let field = builder
            .build_extract_value(tuple, index as u32, "hash_field")
            .expect("build_extract_value should not fail for an in-range field");
        let lane = match element {
            pycc_mir::Ty::Bool => hash_bool(context, builder, field.into_int_value()),
            pycc_mir::Ty::Str => hash_str(builder, rt, field.into_pointer_value()),
            pycc_mir::Ty::Float => hash_float(builder, rt, field.into_float_value()),
            _ => hash_int_word(builder, rt, field.into_int_value()),
        };
        let slot = unsafe {
            builder
                .build_in_bounds_gep(
                    i64_type,
                    lanes,
                    &[i64_type.const_int(index as u64, false)],
                    "hash_lane",
                )
                .expect("build_in_bounds_gep should not fail")
        };
        builder
            .build_store(slot, lane)
            .expect("build_store should not fail");
    }
    builder
        .build_call(rt.hash_tuple, &[lanes.into(), len.into()], "hash_tuple")
        .expect("build_call should not fail for pycc_rt_hash_tuple")
        .try_as_basic_value()
        .expect_basic("pycc_rt_hash_tuple returns an i64")
        .into_int_value()
}

/// Emits `hash(arg)` and returns its `int` result.
pub(super) fn emit_hash<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    module: &inkwell::module::Module<'ctx>,
    rt: &RtFns<'ctx>,
    user_functions: &HashMap<&str, UserFunction<'ctx>>,
    locals: &HashMap<String, StorageSlot<'ctx>>,
    arg: &MirExpr,
) -> Scalar<'ctx> {
    // `hash(None)`: CPython's constant, after the argument's effects (a
    // call returning `None`); the literal itself has none.
    if arg.ty() == pycc_mir::Ty::None {
        if !matches!(arg, MirExpr::NoneLiteral) {
            emit_expr(context, builder, module, rt, user_functions, locals, arg);
        }
        return Scalar::Int(
            builder
                .build_call(
                    rt.int_from_i64,
                    &[context.i64_type().const_int(NONE_HASH, false).into()],
                    "hash_result",
                )
                .expect("build_call should not fail for pycc_rt_int_from_i64")
                .try_as_basic_value()
                .expect_basic("pycc_rt_int_from_i64 returns an encoded int")
                .into_int_value(),
        );
    }
    let scalar = emit_expr(context, builder, module, rt, user_functions, locals, arg);
    let raw = match (arg.ty(), scalar) {
        (pycc_mir::Ty::Tuple(elements), Scalar::Tuple(tuple)) => {
            hash_tuple(context, builder, rt, &elements, tuple)
        }
        (_, Scalar::Object(object)) => {
            let held = crate::object_release::hold(context, module, rt, arg, &scalar);
            let hash = hash_object(context, builder, module, rt, object);
            held.release(builder, rt);
            hash
        }
        (_, Scalar::Bool(value)) => hash_bool(context, builder, value),
        (_, Scalar::Float(value)) => hash_float(builder, rt, value),
        // CPython's hash of the value an `int | float` holds.
        (_, Scalar::Num(word)) => {
            let hash = crate::num::emit_unary(builder, rt.num.hash, word).into_int_value();
            release_scalar_if_temporary(context, builder, rt, arg, &scalar);
            hash
        }
        // A tuple display or call result held as a tuple object.
        (_, Scalar::List(tuple)) => {
            let hash = builder
                .build_call(rt.hash_tuple_object, &[tuple.into()], "hash_tuple_object")
                .expect("build_call should not fail for pycc_rt_hash_tuple_object")
                .try_as_basic_value()
                .expect_basic("pycc_rt_hash_tuple_object returns an i64")
                .into_int_value();
            release_scalar_if_temporary(context, builder, rt, arg, &scalar);
            hash
        }
        (_, Scalar::Str(text)) => {
            let hash = hash_str(builder, rt, text);
            release_scalar_if_temporary(context, builder, rt, arg, &scalar);
            hash
        }
        // An instance argument is `MirExpr::InstanceHash`, never this call.
        (_, other) => {
            let word = to_encoded_int(context, builder, other);
            let hash = hash_int_word(builder, rt, word);
            // `hash(n + n)`: the argument's birth reference is retired
            // once hashed, since this path bypasses generic argument
            // handling. A borrowed read releases nothing.
            release_if_int_temporary(context, builder, rt, arg, word);
            hash
        }
    };
    Scalar::Int(
        builder
            .build_call(rt.int_from_i64, &[raw.into()], "hash_result")
            .expect("build_call should not fail for pycc_rt_int_from_i64")
            .try_as_basic_value()
            .expect_basic("pycc_rt_int_from_i64 returns an encoded int")
            .into_int_value(),
    )
}

/// Emits `MirExpr::InstanceHash` (#1335, Part 1 of #1332) and returns its
/// `int` result.
///
/// - [`InstanceHashVia::Identity`](pycc_mir::InstanceHashVia::Identity):
///   `operand` is the instance, hashed by address through
///   `pycc_rt_hash_pointer`. An instance is never freed, so a temporary
///   operand releases nothing.
/// - [`InstanceHashVia::Method`](pycc_mir::InstanceHashVia::Method):
///   `operand` is the `__hash__` call, whose own `emit_expr` guard stops a
///   raising method before this node runs. A `bool` result is its own hash;
///   an `int` result goes through `pycc_rt_hash_slot_int`, CPython's
///   `slot_tp_hash` reduction, and its birth reference is then retired.
#[allow(clippy::too_many_arguments)]
pub(super) fn emit_instance_hash<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    module: &inkwell::module::Module<'ctx>,
    rt: &RtFns<'ctx>,
    user_functions: &HashMap<&str, UserFunction<'ctx>>,
    locals: &HashMap<String, StorageSlot<'ctx>>,
    operand: &MirExpr,
    via: pycc_mir::InstanceHashVia,
) -> Scalar<'ctx> {
    let scalar = emit_expr(
        context,
        builder,
        module,
        rt,
        user_functions,
        locals,
        operand,
    );
    let raw = match (via, scalar) {
        (pycc_mir::InstanceHashVia::Identity, scalar) => {
            let pointer = expect_instance_pointer(scalar, "a `hash()` identity operand");
            hash_identity(builder, rt, pointer)
        }
        (pycc_mir::InstanceHashVia::Method, Scalar::Bool(value)) => {
            hash_bool(context, builder, value)
        }
        (pycc_mir::InstanceHashVia::Method, other) => {
            let word = to_encoded_int(context, builder, other);
            let hash = hash_slot_int_word(builder, rt, word);
            release_if_int_temporary(context, builder, rt, operand, word);
            hash
        }
    };
    Scalar::Int(
        builder
            .build_call(rt.int_from_i64, &[raw.into()], "hash_result")
            .expect("build_call should not fail for pycc_rt_int_from_i64")
            .try_as_basic_value()
            .expect_basic("pycc_rt_int_from_i64 returns an encoded int")
            .into_int_value(),
    )
}
