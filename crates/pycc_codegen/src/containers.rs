//! Container words and the runtime objects behind tuples, dicts and sets.
//!
//! Every container element -- a list element, a tuple field, a dict key or
//! value, a set element -- crosses into the runtime as one 64-bit *word* of
//! a *kind* (`pycc_rt::value`): [`word_kind`] names the kind of a type, and
//! `list::element_word`/`list::word_element` convert a value to and from its
//! word.
//!
//! A tuple of only `int`/`bool`/`float` elements is an SSA struct; it is
//! boxed into a `pycc_rt` tuple object ([`box_struct_tuple`]) to be stored
//! as a word and unboxed ([`unbox_struct_tuple`]) when read back. Every
//! other tuple (`pycc_hir::tuple_is_boxed`) *is* a tuple object: a
//! `PyListObj` held as [`Scalar::List`] and reference counted exactly as a
//! list (`heap_rc`), whose words compiled code writes and reads inline
//! through the list header.

use super::*;
use inkwell::types::BasicTypeEnum;
use inkwell::values::{BasicValueEnum, StructValue};
use pycc_mir::Ty;

/// The runtime functions of the generic container objects.
pub(crate) struct ContainerFns<'ctx> {
    pub(crate) tuple_new: FunctionValue<'ctx>,
    pub(crate) value_compare: FunctionValue<'ctx>,
    pub(crate) dict_contains: FunctionValue<'ctx>,
    pub(crate) set_new: FunctionValue<'ctx>,
    pub(crate) set_add: FunctionValue<'ctx>,
    pub(crate) set_contains: FunctionValue<'ctx>,
    pub(crate) set_copy: FunctionValue<'ctx>,
    pub(crate) set_from_list: FunctionValue<'ctx>,
    pub(crate) set_repr: FunctionValue<'ctx>,
    pub(crate) set_freeze: FunctionValue<'ctx>,
}

impl<'ctx> ContainerFns<'ctx> {
    pub(crate) fn declare(context: &'ctx Context, module: &inkwell::module::Module<'ctx>) -> Self {
        let i64_type = context.i64_type();
        let i8_type = context.i8_type();
        let void_type = context.void_type();
        let ptr_type = context.ptr_type(inkwell::AddressSpace::default());
        let declare = |name: &str, fn_type: inkwell::types::FunctionType<'ctx>| {
            module
                .get_function(name)
                .unwrap_or_else(|| module.add_function(name, fn_type, Some(Linkage::External)))
        };
        ContainerFns {
            tuple_new: declare(
                "pycc_rt_tuple_new",
                ptr_type.fn_type(&[i64_type.into(), ptr_type.into()], false),
            ),
            value_compare: declare(
                "pycc_rt_value_compare",
                i8_type.fn_type(
                    &[
                        i64_type.into(),
                        i64_type.into(),
                        i64_type.into(),
                        i64_type.into(),
                    ],
                    false,
                ),
            ),
            dict_contains: declare(
                "pycc_rt_dict_contains",
                i8_type.fn_type(&[ptr_type.into(), i64_type.into()], false),
            ),
            set_new: declare(
                "pycc_rt_set_new",
                ptr_type.fn_type(&[i64_type.into()], false),
            ),
            set_add: declare(
                "pycc_rt_set_add",
                void_type.fn_type(&[ptr_type.into(), i64_type.into()], false),
            ),
            set_contains: declare(
                "pycc_rt_set_contains",
                i8_type.fn_type(&[ptr_type.into(), i64_type.into()], false),
            ),
            set_copy: declare(
                "pycc_rt_set_copy",
                ptr_type.fn_type(&[ptr_type.into()], false),
            ),
            set_freeze: declare(
                "pycc_rt_set_freeze",
                ptr_type.fn_type(&[ptr_type.into()], false),
            ),
            set_from_list: declare(
                "pycc_rt_set_from_list",
                ptr_type.fn_type(&[ptr_type.into(), i64_type.into()], false),
            ),
            set_repr: declare(
                "pycc_rt_set_repr",
                ptr_type.fn_type(&[ptr_type.into(), i8_type.into()], false),
            ),
        }
    }
}

/// `pycc_rt::LIST_KIND_*` of a word of type `ty`. An instance of an
/// exception class carries no reference count, so its kind is the uncounted
/// one (`6`).
pub(crate) fn word_kind(rt: &RtFns<'_>, ty: &Ty) -> u64 {
    match ty {
        Ty::Int => 0,
        Ty::Float => 1,
        Ty::Bool => 2,
        Ty::Str => 3,
        Ty::Instance(_) if crate::heap_rc::heap_kind(rt, ty).is_some() => 4,
        Ty::Instance(_) => 6,
        Ty::List(_) => 5,
        Ty::Tuple(_) => 7,
        Ty::Dict(_) => 8,
        Ty::Set(_) | Ty::FrozenSet(_) => 9,
        other => panic!(
            "pycc_codegen: internal error: `{}` is not a container element type -- \
             pycc_hir::is_list_element_ty should have rejected it before codegen",
            other.name()
        ),
    }
}

fn i64_const<'ctx>(context: &'ctx Context, value: u64) -> IntValue<'ctx> {
    context.i64_type().const_int(value, false)
}

/// A new tuple object for elements of types `elements`, every word zero
/// until written with [`store_tuple_word`]. Count one.
pub(crate) fn build_tuple_new<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    rt: &RtFns<'ctx>,
    elements: &[Ty],
) -> PointerValue<'ctx> {
    let i8_type = context.i8_type();
    let kinds: Vec<IntValue<'ctx>> = elements
        .iter()
        .map(|ty| i8_type.const_int(word_kind(rt, ty), false))
        .collect();
    let array_type = i8_type.array_type(kinds.len() as u32);
    let slot = builder
        .build_alloca(array_type, "tuple_kinds")
        .expect("build_alloca should not fail for a small byte array");
    builder
        .build_store(slot, i8_type.const_array(&kinds))
        .expect("build_store should not fail for a constant byte array");
    builder
        .build_call(
            rt.containers.tuple_new,
            &[
                i64_const(context, elements.len() as u64).into(),
                slot.into(),
            ],
            "tuple_new",
        )
        .expect("build_call should not fail for a well-formed tuple construction")
        .try_as_basic_value()
        .expect_basic("pycc_rt_tuple_new returns a pointer")
        .into_pointer_value()
}

/// A pointer to word `index` of the tuple object `tuple`.
fn tuple_word_ptr<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    tuple: PointerValue<'ctx>,
    index: u64,
) -> PointerValue<'ctx> {
    let ptr_type = context.ptr_type(inkwell::AddressSpace::default());
    let header = context.struct_type(&[context.i64_type().into(), ptr_type.into()], false);
    let data_field = builder
        .build_struct_gep(header, tuple, 1, "tuple_data_field")
        .expect("build_struct_gep should not fail for the list header");
    let data = builder
        .build_load(ptr_type, data_field, "tuple_data")
        .expect("build_load should not fail for the data pointer")
        .into_pointer_value();
    unsafe {
        builder
            .build_in_bounds_gep(
                context.i64_type(),
                data,
                &[i64_const(context, index)],
                "tuple_word_ptr",
            )
            .expect("build_in_bounds_gep should not fail for a constant index")
    }
}

/// Writes word `index` of a freshly built tuple object (the tuple adopts
/// the word's reference).
pub(crate) fn store_tuple_word<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    tuple: PointerValue<'ctx>,
    index: u64,
    word: IntValue<'ctx>,
) {
    let ptr = tuple_word_ptr(context, builder, tuple, index);
    builder
        .build_store(ptr, word)
        .expect("build_store should not fail for an i64 word");
}

/// Word `index` of the tuple object `tuple`, borrowed.
pub(crate) fn load_tuple_word<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    tuple: PointerValue<'ctx>,
    index: u64,
) -> IntValue<'ctx> {
    let ptr = tuple_word_ptr(context, builder, tuple, index);
    builder
        .build_load(context.i64_type(), ptr, "tuple_word")
        .expect("build_load should not fail for an i64 word")
        .into_int_value()
}

/// Field `index` of a struct tuple as its scalar.
fn struct_field<'ctx>(
    builder: &inkwell::builder::Builder<'ctx>,
    tuple: StructValue<'ctx>,
    index: u32,
    ty: &Ty,
) -> Scalar<'ctx> {
    let value = builder
        .build_extract_value(tuple, index, "tuple_field")
        .expect("build_extract_value should not fail for an in-range field");
    match ty {
        Ty::Int => Scalar::Int(value.into_int_value()),
        Ty::Bool => Scalar::Bool(value.into_int_value()),
        Ty::Float => Scalar::Float(value.into_float_value()),
        other => panic!(
            "pycc_codegen: internal error: a struct tuple field of type `{}`",
            other.name()
        ),
    }
}

/// Boxes the struct tuple `tuple` of `elements` into a new tuple object
/// (count one).
pub(crate) fn box_struct_tuple<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    rt: &RtFns<'ctx>,
    elements: &[Ty],
    tuple: StructValue<'ctx>,
) -> PointerValue<'ctx> {
    let boxed = build_tuple_new(context, builder, rt, elements);
    for (index, ty) in elements.iter().enumerate() {
        let field = struct_field(builder, tuple, index as u32, ty);
        let word = match ty {
            Ty::Int => {
                let encoded = to_encoded_int(context, builder, field);
                list::check_element_int(context, builder, rt, encoded);
                encoded
            }
            _ => scalar_to_slot_word(context, builder, field),
        };
        store_tuple_word(context, builder, boxed, index as u64, word);
    }
    boxed
}

/// The struct tuple of `elements` a tuple object holds. A null `boxed` --
/// the sentinel a runtime call returns when it raised (a missing dict key,
/// an empty `pop`) -- yields the zero tuple instead of being read, so the
/// pending exception reaches its handler before anything uses the value.
pub(crate) fn unbox_struct_tuple<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    elements: &[Ty],
    boxed: PointerValue<'ctx>,
) -> StructValue<'ctx> {
    let field_types: Vec<BasicTypeEnum<'ctx>> = elements
        .iter()
        .map(|ty| ty_to_basic_type(context, ty.clone()))
        .collect();
    let struct_type = context.struct_type(&field_types, false);
    let function = builder
        .get_insert_block()
        .and_then(|block| block.get_parent())
        .expect("a tuple is unboxed inside a function");
    let is_null = builder
        .build_is_null(boxed, "tuple_is_null")
        .expect("build_is_null should not fail for a pointer");
    let entry = builder.get_insert_block().expect("an insertion block");
    let read_bb = context.append_basic_block(function, "tuple_unbox_read");
    let join_bb = context.append_basic_block(function, "tuple_unbox_join");
    builder
        .build_conditional_branch(is_null, join_bb, read_bb)
        .expect("build_conditional_branch should not fail");
    builder.position_at_end(read_bb);
    let mut aggregate = struct_type.get_undef();
    for (index, ty) in elements.iter().enumerate() {
        let word = load_tuple_word(context, builder, boxed, index as u64);
        let value: BasicValueEnum<'ctx> = match slot_word_to_scalar(context, builder, word, ty) {
            Scalar::Int(v) | Scalar::Bool(v) => v.into(),
            Scalar::Float(v) => v.into(),
            _ => unreachable!("a struct tuple field is int, bool or float"),
        };
        aggregate = builder
            .build_insert_value(aggregate, value, index as u32, "tuple_unbox")
            .expect("build_insert_value should not fail for an in-range field")
            .into_struct_value();
    }
    let read_end = builder.get_insert_block().expect("an insertion block");
    builder
        .build_unconditional_branch(join_bb)
        .expect("build_unconditional_branch should not fail");
    builder.position_at_end(join_bb);
    let phi = builder
        .build_phi(struct_type, "tuple_unboxed")
        .expect("build_phi should not fail for a struct type");
    phi.add_incoming(&[(&struct_type.const_zero(), entry), (&aggregate, read_end)]);
    phi.as_basic_value().into_struct_value()
}

/// `list::word_element` for a word a runtime call returned as a new
/// reference: a struct tuple is unboxed and its tuple object released.
pub(crate) fn owned_word_element<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    rt: &RtFns<'ctx>,
    elem: &Ty,
    word: inkwell::values::IntValue<'ctx>,
) -> Scalar<'ctx> {
    let scalar = list::word_element(context, builder, elem, word);
    if let (Ty::Tuple(elements), Scalar::Tuple(_)) = (elem, &scalar)
        && !pycc_hir::tuple_is_boxed(elements)
    {
        let boxed = builder
            .build_int_to_ptr(
                word,
                context.ptr_type(inkwell::AddressSpace::default()),
                "tuple_box",
            )
            .expect("build_int_to_ptr should not fail reinterpreting an i64 as a pointer");
        crate::heap_rc::emit_decref(context, builder, rt, crate::heap_rc::HeapKind::List, boxed);
    }
    scalar
}

/// A tuple element read `t[index]` of a value of tuple type `elements`
/// held in `scalar`: a struct field, or a borrowed word of a tuple object.
pub(crate) fn tuple_element<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    elements: &[Ty],
    scalar: Scalar<'ctx>,
    index: u64,
) -> Scalar<'ctx> {
    match scalar {
        Scalar::Tuple(tuple) => {
            struct_field(builder, tuple, index as u32, &elements[index as usize])
        }
        Scalar::List(boxed) => {
            let word = load_tuple_word(context, builder, boxed, index);
            list::word_element(context, builder, &elements[index as usize], word)
        }
        _ => panic!("pycc_codegen: internal error: a tuple read of a non-tuple value"),
    }
}

/// The word a value `scalar` of type `ty` crosses into a runtime call that
/// only borrows it, plus the tuple object boxed for it (which the caller
/// releases after the call), if one was built.
pub(crate) fn borrowed_word<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    rt: &RtFns<'ctx>,
    ty: &Ty,
    scalar: Scalar<'ctx>,
) -> (IntValue<'ctx>, Option<PointerValue<'ctx>>) {
    match (ty, scalar) {
        (Ty::Int, scalar) => (to_encoded_int(context, builder, scalar), None),
        (Ty::Tuple(elements), Scalar::Tuple(tuple)) => {
            let boxed = box_struct_tuple(context, builder, rt, elements, tuple);
            let word = builder
                .build_ptr_to_int(boxed, context.i64_type(), "boxed_tuple_word")
                .expect("build_ptr_to_int should not fail for a pointer");
            (word, Some(boxed))
        }
        (_, scalar) => (scalar_to_slot_word(context, builder, scalar), None),
    }
}

/// Releases a tuple object [`borrowed_word`] boxed.
pub(crate) fn release_boxed<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    rt: &RtFns<'ctx>,
    boxed: Option<PointerValue<'ctx>>,
) {
    if let Some(boxed) = boxed {
        heap_rc::emit_decref(context, builder, rt, heap_rc::HeapKind::List, boxed);
    }
}

/// Whether `item in container` has a native membership test for a
/// container of type `ty`: a list, a dict, or a set of hashable elements.
pub(crate) fn has_native_contains(ty: &Ty) -> bool {
    match ty {
        Ty::List(_) | Ty::Dict(_) => true,
        Ty::Set(elem) | Ty::FrozenSet(elem) => !matches!(**elem, Ty::Instance(_)),
        _ => false,
    }
}

/// `item in container` / `item not in container`. The needle is borrowed by
/// the runtime (a struct tuple needle is boxed for the call and released);
/// the caller releases fresh operands.
pub(crate) fn emit_contains<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    rt: &RtFns<'ctx>,
    container_ty: &Ty,
    item: Scalar<'ctx>,
    container: Scalar<'ctx>,
    negate: bool,
) -> Scalar<'ctx> {
    let (function, pointer, elem) = match (container_ty, container) {
        (Ty::List(elem), Scalar::List(ptr)) => (rt.list_contains, ptr, (**elem).clone()),
        (Ty::Dict(kv), Scalar::Dict(ptr)) => (rt.containers.dict_contains, ptr, kv.0.clone()),
        (Ty::Set(elem) | Ty::FrozenSet(elem), Scalar::Set(ptr)) => {
            (rt.containers.set_contains, ptr, (**elem).clone())
        }
        _ => panic!("pycc_codegen: internal error: a membership test of a non-container"),
    };
    let item = coerce_scalar_to_type(context, builder, item, elem.clone());
    let (word, boxed) = borrowed_word(context, builder, rt, &elem, item);
    let found = builder
        .build_call(function, &[pointer.into(), word.into()], "contains")
        .expect("build_call should not fail for a declared runtime function")
        .try_as_basic_value()
        .expect_basic("a membership test returns i8")
        .into_int_value();
    release_boxed(context, builder, rt, boxed);
    let found = if negate {
        builder
            .build_xor(found, context.i8_type().const_int(1, false), "not_in")
            .expect("build_xor should not fail for two i8 operands")
    } else {
        found
    };
    Scalar::Bool(found)
}

/// Whether `ty` is a container compared element-wise by the runtime.
pub(crate) fn is_container_ty(ty: &Ty) -> bool {
    matches!(
        ty,
        Ty::List(_) | Ty::Tuple(_) | Ty::Dict(_) | Ty::Set(_) | Ty::FrozenSet(_)
    )
}

/// `l <op> r` for two containers of type `ty` (`code` is
/// `pycc_rt::value::CMP_*`), as an `i8` bool. Both operands are borrowed.
pub(crate) fn emit_value_compare<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    rt: &RtFns<'ctx>,
    ty: &Ty,
    l: Scalar<'ctx>,
    r: Scalar<'ctx>,
    code: u64,
) -> IntValue<'ctx> {
    let (left, left_box) = borrowed_word(context, builder, rt, ty, l);
    let (right, right_box) = borrowed_word(context, builder, rt, ty, r);
    let result = builder
        .build_call(
            rt.containers.value_compare,
            &[
                i64_const(context, word_kind(rt, ty)).into(),
                left.into(),
                right.into(),
                i64_const(context, code).into(),
            ],
            "value_compare",
        )
        .expect("build_call should not fail for a declared runtime function")
        .try_as_basic_value()
        .expect_basic("pycc_rt_value_compare returns i8")
        .into_int_value();
    release_boxed(context, builder, rt, left_box);
    release_boxed(context, builder, rt, right_box);
    result
}

/// The key and value types of a dict type.
pub(crate) fn dict_kv(ty: &Ty) -> (Ty, Ty) {
    match ty {
        Ty::Dict(kv) => (kv.0.clone(), kv.1.clone()),
        other => panic!(
            "pycc_codegen: internal error: expected a dict, got `{}`",
            other.name()
        ),
    }
}

/// A new empty dict with keys of `key` and values of `value`.
pub(crate) fn build_dict_new<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    rt: &RtFns<'ctx>,
    key: &Ty,
    value: &Ty,
) -> PointerValue<'ctx> {
    builder
        .build_call(
            rt.dict_new,
            &[
                i64_const(context, word_kind(rt, key)).into(),
                i64_const(context, word_kind(rt, value)).into(),
            ],
            "dict_new",
        )
        .expect("build_call should not fail for a well-formed dict construction")
        .try_as_basic_value()
        .expect_basic("pycc_rt_dict_new returns a pointer")
        .into_pointer_value()
}

/// A new empty set of elements of `elem` (not a user-class instance, whose
/// set is `int_set.rs`'s object).
pub(crate) fn build_set_new<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    rt: &RtFns<'ctx>,
    elem: &Ty,
) -> PointerValue<'ctx> {
    builder
        .build_call(
            rt.containers.set_new,
            &[i64_const(context, word_kind(rt, elem)).into()],
            "set_new",
        )
        .expect("build_call should not fail for a well-formed set construction")
        .try_as_basic_value()
        .expect_basic("pycc_rt_set_new returns a pointer")
        .into_pointer_value()
}

/// `s.add(word)`; the set adopts the word's reference.
pub(crate) fn build_set_add<'ctx>(
    builder: &inkwell::builder::Builder<'ctx>,
    rt: &RtFns<'ctx>,
    set: PointerValue<'ctx>,
    word: IntValue<'ctx>,
) {
    builder
        .build_call(rt.containers.set_add, &[set.into(), word.into()], "")
        .expect("build_call should not fail for a well-formed set add");
}

/// The element types of a struct tuple, read off its LLVM field types
/// (`i64` an `int`, `i8` a `bool`, `f64` a `float`).
fn struct_tuple_types(tuple: StructValue<'_>) -> Vec<Ty> {
    tuple
        .get_type()
        .get_field_types()
        .iter()
        .map(|field| match field {
            BasicTypeEnum::FloatType(_) => Ty::Float,
            BasicTypeEnum::IntType(int) if int.get_bit_width() == 8 => Ty::Bool,
            _ => Ty::Int,
        })
        .collect()
}

/// `str(t)` of a struct tuple as a new `str`.
pub(crate) fn struct_tuple_repr<'ctx>(
    builder: &inkwell::builder::Builder<'ctx>,
    rt: &RtFns<'ctx>,
    tuple: StructValue<'ctx>,
) -> PointerValue<'ctx> {
    let context = rt.context;
    let types = struct_tuple_types(tuple);
    let boxed = box_struct_tuple(context, builder, rt, &types, tuple);
    let text = builder
        .build_call(rt.list_repr, &[boxed.into()], "tuple_repr")
        .expect("build_call should not fail for a well-formed repr")
        .try_as_basic_value()
        .expect_basic("pycc_rt_list_repr returns a pointer")
        .into_pointer_value();
    heap_rc::emit_decref(context, builder, rt, heap_rc::HeapKind::List, boxed);
    text
}
