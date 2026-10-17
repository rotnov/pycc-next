//! The *packer contract* shared by every foreign object operation that
//! hands a pycc value to the C shim as a `PyObject *`: a method or direct
//! call's arguments, a subscript key (`foreign_call.rs`), a rich
//! comparison's scalar operand (`foreign_compare.rs`) and an attribute
//! store's value (`foreign_store.rs`, #1457).
//!
//! Each operand goes through exactly one `pycc_ext_obj_pack_*` helper,
//! which *borrows* the pycc-side value and returns a *new* reference, and
//! that reference is handed to a shim helper that releases it on every
//! path. Keeping the three primitives here -- the declaration helper
//! [`shim_fn`], the symbol mapping [`packer_for`] and the call
//! [`emit_pack`] -- is what keeps that contract from forking into one
//! spelling per operation. Split out of `foreign_call.rs` in Part 2a of
//! #1371, when an `object` operand gained its own packer
//! (`pycc_ext_obj_pack_object`, `Py_INCREF` and return).

use super::*;
use inkwell::builder::Builder;
use inkwell::values::BasicValueEnum;

/// Declares one of the shim's helpers once per module, returning the
/// existing declaration on every later call.
///
/// `foreign_attr.rs`'s `obj_getattr_fn` generalized over the parameter
/// list, because the foreign modules declare many symbols rather than one
/// and a second `add_function` of any one name is an LLVM module-verifier
/// error.
pub(crate) fn shim_fn<'ctx>(
    module: &inkwell::module::Module<'ctx>,
    symbol: &str,
    fn_type: inkwell::types::FunctionType<'ctx>,
) -> FunctionValue<'ctx> {
    if let Some(existing) = module.get_function(symbol) {
        return existing;
    }
    module.add_function(symbol, fn_type, None)
}

/// The packer symbol and the LLVM argument value for one marshalled
/// operand.
///
/// Every admitted operand has exactly one packer, and the mapping is total
/// over what `pycc_types` admits as an argument of a call on a `Ty::Object`
/// or as the key of a `Ty::Object` subscript: `int`/`float`/`bool`/`str`,
/// since Part 2a of #1371 a second `object`, and since #1435 an instance of
/// a regular user class -- a call argument, (#1457) an attribute store's
/// value and, since #1470, a key, a list-display element or a rich
/// comparison operand. A `None` call argument (Part 8 of
/// #1371) reaches here already replaced by [`none_pointer`]'s `object`. Any
/// other `Scalar` is a front-end defect: the checker refuses a container
/// there before lowering ever runs.
pub(super) fn packer_for<'ctx>(scalar: Scalar<'ctx>) -> (&'static str, BasicValueEnum<'ctx>) {
    match scalar {
        Scalar::Int(value) => (EXT_OBJ_PACK_INT_SYMBOL, value.into()),
        Scalar::Float(value) => (EXT_OBJ_PACK_FLOAT_SYMBOL, value.into()),
        Scalar::Bool(value) => (EXT_OBJ_PACK_BOOL_SYMBOL, value.into()),
        Scalar::Str(value) => (EXT_OBJ_PACK_STR_SYMBOL, value.into()),
        Scalar::Object(value) => (EXT_OBJ_PACK_OBJECT_SYMBOL, value.into()),
        Scalar::Instance(value) => (EXT_OBJ_PACK_INSTANCE_SYMBOL, value.into()),
        // Non-strict mode: a fresh native list copied into a CPython one.
        Scalar::List(value) => (EXT_OBJ_PACK_LIST_SYMBOL, value.into()),
        // A frozenset of scalars (`pycc_types`' `is_packable_frozenset`;
        // a mutable `set` is never admitted), copied into a CPython one.
        Scalar::Set(value) => (EXT_OBJ_PACK_FROZENSET_SYMBOL, value.into()),
        _ => panic!(
            "pycc_codegen: internal error: an operand of a foreign object operation did not \
             evaluate to a marshallable scalar -- pycc_types admits only `int`, `float`, \
             `bool`, `str`, `object` and a class instance there"
        ),
    }
}

/// Emits the packer call for one already-evaluated operand, returning the
/// new `PyObject *` reference (or `NULL` with a CPython exception set,
/// which every consuming shim helper tolerates -- so no NULL check is
/// emitted here and the operation keeps exactly one failure edge).
pub(super) fn emit_pack<'ctx>(
    context: &'ctx Context,
    builder: &Builder<'ctx>,
    module: &inkwell::module::Module<'ctx>,
    scalar: Scalar<'ctx>,
    name: &str,
) -> PointerValue<'ctx> {
    let ptr = context.ptr_type(inkwell::AddressSpace::default());
    if let Scalar::Tuple(aggregate) = scalar {
        return emit_pack_tuple(context, builder, module, aggregate, name);
    }
    let (symbol, value) = packer_for(scalar);
    let packer = shim_fn(
        module,
        symbol,
        ptr.fn_type(&[value.get_type().into()], false),
    );
    builder
        .build_call(packer, &[value.into()], name)
        .unwrap_or_else(|_| panic!("build_call should not fail for {symbol}"))
        .try_as_basic_value()
        .expect_basic("a pycc_ext_obj_pack_* helper returns PyObject *")
        .into_pointer_value()
}

/// Packs a native tuple of `int`/`float`/`bool` (`pycc_types`'
/// `is_packable_tuple`) into a new CPython `tuple`: each field through its
/// scalar packer, then `pycc_ext_obj_pack_tuple`, which consumes the packed
/// elements. A field's kind is read off its LLVM type: `i64` is an `int`
/// word, `double` a `float`, `i8` a `bool`, a pointer a `str` (the only
/// pointer field `is_packable_tuple` admits).
fn emit_pack_tuple<'ctx>(
    context: &'ctx Context,
    builder: &Builder<'ctx>,
    module: &inkwell::module::Module<'ctx>,
    aggregate: inkwell::values::StructValue<'ctx>,
    name: &str,
) -> PointerValue<'ctx> {
    let ptr = context.ptr_type(inkwell::AddressSpace::default());
    let fields = aggregate.get_type().get_field_types();
    let count = u32::try_from(fields.len()).expect("a tuple has few fields");
    let function = builder
        .get_insert_block()
        .and_then(|b| b.get_parent())
        .expect("packing happens inside a function");
    let items = crate::build_at_entry_block(builder, function, |b| {
        b.build_alloca(ptr.array_type(count), "pack_tuple_items")
            .expect("build_alloca should not fail")
    });
    for (index, field_ty) in fields.iter().enumerate() {
        let index = u32::try_from(index).expect("a tuple has few fields");
        let value = builder
            .build_extract_value(aggregate, index, "pack_tuple_field")
            .expect("build_extract_value should not fail");
        let scalar = match field_ty {
            inkwell::types::BasicTypeEnum::FloatType(_) => Scalar::Float(value.into_float_value()),
            inkwell::types::BasicTypeEnum::IntType(int) if int.get_bit_width() == 64 => {
                Scalar::Int(value.into_int_value())
            }
            inkwell::types::BasicTypeEnum::IntType(_) => Scalar::Bool(value.into_int_value()),
            inkwell::types::BasicTypeEnum::PointerType(_) => {
                Scalar::Str(value.into_pointer_value())
            }
            _ => panic!(
                "pycc_codegen: internal error: a packed tuple holds only int, float, bool and str"
            ),
        };
        let packed = emit_pack(context, builder, module, scalar, "pack_tuple_item");
        // SAFETY (LLVM): `index < count`, the array's length.
        let slot = unsafe {
            builder
                .build_in_bounds_gep(
                    ptr.array_type(count),
                    items,
                    &[
                        context.i64_type().const_zero(),
                        context.i64_type().const_int(u64::from(index), false),
                    ],
                    "pack_tuple_slot",
                )
                .expect("build_in_bounds_gep should not fail")
        };
        builder
            .build_store(slot, packed)
            .expect("build_store should not fail");
    }
    let pack = shim_fn(
        module,
        "pycc_ext_obj_pack_tuple",
        ptr.fn_type(&[context.i64_type().into(), ptr.into()], false),
    );
    builder
        .build_call(
            pack,
            &[
                context.i64_type().const_int(u64::from(count), false).into(),
                items.into(),
            ],
            name,
        )
        .expect("build_call should not fail for pycc_ext_obj_pack_tuple")
        .try_as_basic_value()
        .expect_basic("pycc_ext_obj_pack_tuple returns PyObject *")
        .into_pointer_value()
}

/// A borrowed pointer to CPython's `None` ([`EXT_OBJ_NONE_SYMBOL`]).
///
/// Shared by an identity test's `None` side (`foreign_compare.rs`, Part 1
/// of #1371), a `None` call argument (Part 8 of #1371) and a `None`
/// returned as an object (`object_return.rs`, Part 1 of #1387). pycc's own
/// `None` is an all-zero placeholder with no `PyObject *`, so
/// [`packer_for`] has no row for it; a `None` argument instead becomes this
/// pointer as a [`Scalar::Object`], which [`emit_pack`] packs through the
/// `object` packer like any other object operand. The packer's new
/// reference is the one the consuming call helper releases, so `None`'s
/// refcount is unchanged by the call (and `None` is immortal on every
/// supported CPython anyway).
pub(crate) fn none_pointer<'ctx>(
    context: &'ctx Context,
    builder: &Builder<'ctx>,
    module: &inkwell::module::Module<'ctx>,
) -> PointerValue<'ctx> {
    let ptr = context.ptr_type(inkwell::AddressSpace::default());
    let none_fn = shim_fn(module, EXT_OBJ_NONE_SYMBOL, ptr.fn_type(&[], false));
    builder
        .build_call(none_fn, &[], "foreign_none")
        .expect("build_call should not fail for pycc_ext_obj_none")
        .try_as_basic_value()
        .expect_basic("pycc_ext_obj_none returns PyObject *")
        .into_pointer_value()
}
