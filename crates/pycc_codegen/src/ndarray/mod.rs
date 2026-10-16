//! Typed numpy array operations (`MirExpr::Nd`, `pycc_hir::ndarray`).
//!
//! An array is the `PyObject *` of a `numpy.ndarray` whose dtype and number
//! of dimensions `pycc_hir::ndarray::lower`'s check (`NdOp::Check`, the
//! shim's `pycc_ext_nd_check`) has established. Element access reads the
//! array's data pointer, shape and strides straight from the object, at the
//! offsets of numpy's `PyArrayObject_fields` (the shim validates them once
//! per process, `src/ext/module/ndarray.c`), bounds-checks each index
//! (negative indices count from the end), and loads or stores the element
//! inline. No CPython call is made on the success path.
//!
//! Every failure takes the foreign failure edge immediately
//! (`foreign_fail`): the cold block calls a shim helper that sets numpy's
//! own exception (`IndexError: index 5 is out of bounds for axis 0 with size
//! 3`, `ValueError: assignment destination is read-only`, ...) and answers
//! -1. So an `Nd` node never leaves a pending pycc exception behind for the
//! post-node guard, and `exception::expression_can_set_exception` answers
//! `false` for it: the hot path carries no exception check at all.
//!
//! Alias analysis: header fields (`data`, `nd`, `dimensions`, `strides`,
//! `flags`) are tagged `pycc nd header`, the shape and stride arrays `pycc
//! nd shape`, and every element access `pycc nd element` (one tag for every
//! dtype, since two arrays of different dtypes may view the same memory).
//! A store to an element therefore cannot invalidate a header load, which
//! is what lets LLVM hoist an array's shape and data pointer out of a loop
//! that writes to it. Calls -- including every CPython call that could
//! reshape an array -- still alias everything.
//!
//! Every operation checks its array first, except one on a cache-safe local
//! (`NdExpr::cached`) whose header snapshot ([`NdCache`]) is fresh. The check
//! is inline on its success path (exact `numpy.ndarray` type, dimension
//! count, canonical dtype pointer, all read from `pycc_ext_nd_canon`, which
//! the shim fills once it has validated the layout); anything else calls
//! `pycc_ext_nd_check`.
//!
//! A byte buffer (`NdDtype::Byte`, a CPython `bytearray`) is read through
//! the same snapshot, but never from the object's own fields: its check,
//! the shim's `pycc_ext_ba_check`, answers the buffer pointer and size
//! (`PyByteArray_AsString`, `PyByteArray_Size`) into a snapshot, so no
//! `bytearray` layout is assumed. An operation on a base that is not a
//! cache-safe local checks into a snapshot of its own, used once.

use super::*;
use crate::foreign_fail::{ForeignFailEdge, emit_failure, route_negative};
use crate::foreign_pack::shim_fn;
use inkwell::IntPredicate;
use inkwell::builder::Builder;
use inkwell::values::{BasicValue as _, IntValue, PointerValue};
use pycc_hir::ndarray::{NdDtype, NdOp, NdTy};
use pycc_mir::MirNdExpr;

mod element;
mod fresh;
mod index;

use element::{byte_value, emit_writeable_check, load_element, store_element};
pub(crate) use fresh::clear_fresh_after_calls;
pub(crate) use index::is_simple_index;
use index::{PendingIndex, decode_index, element_address, evaluate_index};

/// `sizeof(PyObject)` on the 64-bit, non-free-threaded CPython builds pycc
/// targets; `pycc_ext_nd_check` refuses any other header size.
const HEAD: u64 = 16;
const DATA_OFFSET: u64 = HEAD;
const DIMS_OFFSET: u64 = HEAD + 16;
const STRIDES_OFFSET: u64 = HEAD + 24;
const FLAGS_OFFSET: u64 = HEAD + 48;
/// numpy's `NPY_ARRAY_WRITEABLE`.
const WRITEABLE: u64 = 0x0400;

const TAG_HEADER: &str = "pycc nd header";
const TAG_SHAPE: &str = "pycc nd shape";
const TAG_ELEMENT: &str = "pycc nd element";

const ND_CHECK_SYMBOL: &str = "pycc_ext_nd_check";
const ND_INDEX_ERROR_SYMBOL: &str = "pycc_ext_nd_index_error";
const ND_INDEX_WORD_SYMBOL: &str = "pycc_ext_nd_index_word";
const ND_INT_WORD_SYMBOL: &str = "pycc_ext_nd_int_word";
const ND_READONLY_SYMBOL: &str = "pycc_ext_nd_readonly";
const BA_CHECK_SYMBOL: &str = "pycc_ext_ba_check";
const BA_INDEX_ERROR_SYMBOL: &str = "pycc_ext_ba_index_error";
const BA_INDEX_WORD_SYMBOL: &str = "pycc_ext_ba_index_word";
const BA_BYTE_ERROR_SYMBOL: &str = "pycc_ext_ba_byte_error";

/// The header snapshot of one cache-safe typed array local: entry-block
/// slots holding its data pointer, shape and strides as of its last check,
/// and whether that check is still current (`fresh`).
///
/// A cache-safe local's every binding is followed by an `NdOp::Check`
/// (`pycc_hir::ndarray::lower`), which stores the fields here and sets
/// `fresh`. [`clear_fresh_after_calls`] clears `fresh` after every call that
/// may run CPython code, which could change the array in place (`resize`,
/// `__setstate__`, a new shape); an operation that finds `fresh` clear
/// re-checks the array and refreshes the snapshot first. Otherwise it reads
/// the slots instead of the array object. The slots are never
/// address-taken, so LLVM promotes them to registers: a loop that calls no
/// CPython code holds the array's shape and data pointer in registers
/// however much it stores.
#[derive(Clone, Copy)]
pub(crate) struct NdCache<'ctx> {
    data: PointerValue<'ctx>,
    dims: [PointerValue<'ctx>; 2],
    strides: [PointerValue<'ctx>; 2],
    /// The array's `flags` (for the writeable bit).
    flags: PointerValue<'ctx>,
    fresh: PointerValue<'ctx>,
}

/// [`NdCache`]s by `(function, local)`.
#[derive(Default)]
pub(crate) struct NdCaches<'ctx>(std::cell::RefCell<HashMap<(String, String), NdCache<'ctx>>>);

/// The array's header and the pieces of it an operation reads.
struct Header<'ctx> {
    object: PointerValue<'ctx>,
    cache: Option<NdCache<'ctx>>,
}

impl<'ctx> Header<'ctx> {
    fn field_ptr(
        &self,
        context: &'ctx Context,
        builder: &Builder<'ctx>,
        offset: u64,
    ) -> PointerValue<'ctx> {
        // SAFETY (LLVM): an in-bounds byte offset into the array object.
        unsafe {
            builder
                .build_in_bounds_gep(
                    context.i8_type(),
                    self.object,
                    &[context.i64_type().const_int(offset, false)],
                    "nd_field",
                )
                .expect("build_in_bounds_gep should not fail")
        }
    }

    fn load_ptr(
        &self,
        context: &'ctx Context,
        builder: &Builder<'ctx>,
        offset: u64,
        name: &str,
    ) -> PointerValue<'ctx> {
        let ptr_ty = context.ptr_type(inkwell::AddressSpace::default());
        let load = builder
            .build_load(ptr_ty, self.field_ptr(context, builder, offset), name)
            .expect("build_load should not fail");
        tag(context, load, TAG_HEADER);
        load.into_pointer_value()
    }

    fn data(&self, context: &'ctx Context, builder: &Builder<'ctx>) -> PointerValue<'ctx> {
        if let Some(cache) = self.cache {
            let ptr_ty = context.ptr_type(inkwell::AddressSpace::default());
            return builder
                .build_load(ptr_ty, cache.data, "nd_data")
                .expect("build_load should not fail")
                .into_pointer_value();
        }
        self.load_ptr(context, builder, DATA_OFFSET, "nd_data")
    }

    /// `dimensions[axis]` (`stride == false`) or `strides[axis]`.
    fn dim(
        &self,
        context: &'ctx Context,
        builder: &Builder<'ctx>,
        axis: u8,
        stride: bool,
    ) -> IntValue<'ctx> {
        if let Some(cache) = self.cache {
            let slot = if stride { cache.strides } else { cache.dims }[usize::from(axis)];
            return builder
                .build_load(context.i64_type(), slot, "nd_cached")
                .expect("build_load should not fail")
                .into_int_value();
        }
        self.raw_dim(context, builder, axis, stride)
    }

    /// [`Self::dim`] read from the array object itself.
    fn raw_dim(
        &self,
        context: &'ctx Context,
        builder: &Builder<'ctx>,
        axis: u8,
        stride: bool,
    ) -> IntValue<'ctx> {
        let array = self.load_ptr(
            context,
            builder,
            if stride { STRIDES_OFFSET } else { DIMS_OFFSET },
            if stride { "nd_strides" } else { "nd_dims" },
        );
        let i64_type = context.i64_type();
        // SAFETY (LLVM): `axis < ndim`, which the check established.
        let slot = unsafe {
            builder
                .build_in_bounds_gep(
                    i64_type,
                    array,
                    &[i64_type.const_int(u64::from(axis), false)],
                    "nd_dim_slot",
                )
                .expect("build_in_bounds_gep should not fail")
        };
        let load = builder
            .build_load(i64_type, slot, if stride { "nd_stride" } else { "nd_size" })
            .expect("build_load should not fail");
        tag(context, load, TAG_SHAPE);
        load.into_int_value()
    }

    fn flags(&self, context: &'ctx Context, builder: &Builder<'ctx>) -> IntValue<'ctx> {
        if let Some(cache) = self.cache {
            return builder
                .build_load(context.i32_type(), cache.flags, "nd_cached_flags")
                .expect("build_load should not fail")
                .into_int_value();
        }
        let load = builder
            .build_load(
                context.i32_type(),
                self.field_ptr(context, builder, FLAGS_OFFSET),
                "nd_flags",
            )
            .expect("build_load should not fail");
        tag(context, load, TAG_HEADER);
        load.into_int_value()
    }
}

fn tag<'ctx>(context: &'ctx Context, value: inkwell::values::BasicValueEnum<'ctx>, name: &str) {
    if let Some(instruction) = value.as_instruction_value() {
        crate::tbaa::tag(context, instruction, name);
    }
}

/// Emits `nd` and returns its value.
#[allow(clippy::too_many_arguments)]
pub(crate) fn emit_nd<'ctx>(
    context: &'ctx Context,
    builder: &Builder<'ctx>,
    module: &inkwell::module::Module<'ctx>,
    rt: &RtFns<'ctx>,
    user_functions: &HashMap<&str, UserFunction<'ctx>>,
    locals: &HashMap<String, StorageSlot<'ctx>>,
    nd: &MirNdExpr,
) -> Scalar<'ctx> {
    emit_nd_value(
        context,
        builder,
        module,
        rt,
        user_functions,
        locals,
        nd,
        false,
    )
}

/// Whether `nd` reads an element of an `int64` array or a byte buffer,
/// which [`emit_nd_raw_i64`] can load as a machine integer.
pub(crate) fn is_i64_element(nd: &MirNdExpr) -> bool {
    matches!(nd.op, NdOp::Get { .. }) && matches!(nd.nd.dtype, NdDtype::I64 | NdDtype::Byte)
}

/// Emits an `int64` element read ([`is_i64_element`]) and returns the
/// element as a machine `i64`, not encoded as an `int` word: for a caller
/// that does its own arithmetic on it and so never creates the `int`.
pub(crate) fn emit_nd_raw_i64<'ctx>(
    context: &'ctx Context,
    builder: &Builder<'ctx>,
    module: &inkwell::module::Module<'ctx>,
    rt: &RtFns<'ctx>,
    user_functions: &HashMap<&str, UserFunction<'ctx>>,
    locals: &HashMap<String, StorageSlot<'ctx>>,
    nd: &MirNdExpr,
) -> IntValue<'ctx> {
    assert!(
        is_i64_element(nd),
        "pycc_codegen: internal error: not an int64 element read"
    );
    match emit_nd_value(
        context,
        builder,
        module,
        rt,
        user_functions,
        locals,
        nd,
        true,
    ) {
        Scalar::Int(raw) => raw,
        _ => unreachable!("an int64 element is an integer"),
    }
}

/// [`emit_nd`]; with `raw_i64`, an `int64` element read answers the
/// machine integer ([`emit_nd_raw_i64`]).
#[allow(clippy::too_many_arguments)]
fn emit_nd_value<'ctx>(
    context: &'ctx Context,
    builder: &Builder<'ctx>,
    module: &inkwell::module::Module<'ctx>,
    rt: &RtFns<'ctx>,
    user_functions: &HashMap<&str, UserFunction<'ctx>>,
    locals: &HashMap<String, StorageSlot<'ctx>>,
    nd: &MirNdExpr,
    raw_i64: bool,
) -> Scalar<'ctx> {
    // A store evaluates its value first, then the array, then the indices,
    // as CPython evaluates `a[i] = v`.
    let stored = match nd.op {
        NdOp::Set { .. } => {
            let value = nd.args.last().expect("a store carries its value");
            Some((
                value,
                emit_expr(context, builder, module, rt, user_functions, locals, value),
            ))
        }
        _ => None,
    };
    let base_scalar = emit_expr(
        context,
        builder,
        module,
        rt,
        user_functions,
        locals,
        &nd.base,
    );
    let held = crate::object_release::hold(context, module, rt, &nd.base, &base_scalar);
    let object = crate::foreign_attr::expect_object_pointer(base_scalar);
    let cache_key = match &nd.base {
        MirExpr::Name { name, .. } if nd.cached && locals.contains_key(name) => builder
            .get_insert_block()
            .and_then(|b| b.get_parent())
            .map(|f| (f.get_name().to_string_lossy().into_owned(), name.clone())),
        _ => None,
    };
    let mut header = Header {
        object,
        cache: cache_key
            .as_ref()
            .and_then(|key| rt.nd_caches.0.borrow().get(key).copied()),
    };
    let byte = nd.nd.dtype == NdDtype::Byte;
    // The indices are evaluated before the array is checked: evaluating one
    // may run CPython code (`a[f()]`) that resizes, reshapes or retypes the
    // array, so a check made before them would vouch for a header that no
    // longer exists. They are only evaluated here; decoding them (which
    // raises nothing but `IndexError` and runs no CPython code) waits for
    // the check, so a wrong array still raises its `TypeError` first.
    let pending: Vec<PendingIndex<'ctx, '_>> = match nd.op {
        NdOp::Get { .. } | NdOp::Set { .. } => {
            let index_count = match nd.op {
                NdOp::Set { .. } => nd.args.len() - 1,
                _ => nd.args.len(),
            };
            nd.args[..index_count]
                .iter()
                .map(|index| {
                    evaluate_index(context, builder, module, rt, user_functions, locals, index)
                })
                .collect()
        }
        _ => Vec::new(),
    };
    // Nothing between this check and the element access below may run
    // CPython code: index decoding, the bounds and writeable checks and the
    // stored value's conversion call only `pycc_ext_nd_index_word`,
    // `pycc_ext_nd_index_error`, `pycc_ext_nd_readonly`,
    // `pycc_ext_nd_int_word` and pycc_rt number helpers (`may_run_python`'s
    // allowlist).
    if nd.op != NdOp::Check {
        match header.cache {
            Some(cache) => refresh_if_stale(context, builder, module, rt, &header, cache, nd.nd),
            // Anything but a cache-safe local is checked at each use: CPython
            // code can rebind a global or an attribute, or change any array
            // in place, without passing a compiled binding's check.
            None if byte => {
                let scratch = new_cache(context, builder);
                emit_byte_check(context, builder, module, rt, object, scratch);
                header.cache = Some(scratch);
            }
            None => emit_check(context, builder, module, rt, object, nd.nd),
        }
    }
    let result = match nd.op {
        NdOp::Check if byte => {
            let cache = match cache_key {
                Some(key) => cache_slots(context, builder, rt, &header, key),
                None => new_cache(context, builder),
            };
            emit_byte_check(context, builder, module, rt, object, cache);
            none_value(context)
        }
        NdOp::Check => {
            emit_check(context, builder, module, rt, object, nd.nd);
            if let Some(key) = cache_key {
                store_cache(context, builder, rt, &header, key, nd.nd.ndim);
            }
            none_value(context)
        }
        NdOp::Shape => {
            let int_ty = pycc_mir::Ty::Int;
            let tuple_ty = ty_to_basic_type(
                context,
                pycc_mir::Ty::Tuple(Box::new(vec![int_ty; usize::from(nd.nd.ndim)])),
            )
            .into_struct_type();
            let mut aggregate = tuple_ty.get_undef();
            for axis in 0..nd.nd.ndim {
                let size = header.dim(context, builder, axis, false);
                let word = raw_i64_to_tagged_int(context, builder, size);
                aggregate = builder
                    .build_insert_value(aggregate, word, u32::from(axis), "nd_shape")
                    .expect("build_insert_value should not fail")
                    .into_struct_value();
            }
            Scalar::Tuple(aggregate)
        }
        NdOp::Dim(axis) => {
            let size = header.dim(context, builder, axis, false);
            Scalar::Int(raw_i64_to_tagged_int(context, builder, size))
        }
        NdOp::Size => {
            let mut size = header.dim(context, builder, 0, false);
            for axis in 1..nd.nd.ndim {
                let next = header.dim(context, builder, axis, false);
                size = builder
                    .build_int_mul(size, next, "nd_size")
                    .expect("build_int_mul should not fail");
            }
            Scalar::Int(raw_i64_to_tagged_int(context, builder, size))
        }
        NdOp::Ndim => Scalar::Int(
            context
                .i64_type()
                .const_int((u64::from(nd.nd.ndim) << 1) | 1, false),
        ),
        NdOp::Get { chained } | NdOp::Set { chained } => {
            let mut raw = Vec::with_capacity(pending.len());
            let mut words = Vec::with_capacity(pending.len());
            for index in pending {
                let symbol = if byte {
                    BA_INDEX_WORD_SYMBOL
                } else {
                    ND_INDEX_WORD_SYMBOL
                };
                let (value, word) = decode_index(context, builder, module, rt, index, symbol);
                raw.push(value);
                words.extend(word);
            }
            let stored = match stored {
                Some((value_expr, value)) if byte => Some((
                    value_expr,
                    byte_value(context, builder, module, rt, value_expr, value),
                )),
                other => other,
            };
            let address = element_address(
                context, builder, module, rt, &header, &raw, chained, byte,
            );
            let value = match (nd.op, stored) {
                (NdOp::Set { .. }, Some((value_expr, value))) => {
                    emit_writeable_check(context, builder, module, rt, &header);
                    store_element(
                        context,
                        builder,
                        module,
                        rt,
                        address,
                        nd.nd.dtype,
                        value_expr,
                        value,
                    );
                    none_value(context)
                }
                _ => load_element(context, builder, rt, address, nd.nd.dtype, !raw_i64),
            };
            for (index, word) in words {
                release_if_int_temporary(context, builder, rt, index, word);
            }
            value
        }
    };
    held.release(builder, rt);
    result
}

/// Re-checks the array and refreshes its snapshot when `cache` is not
/// fresh; leaves the builder on the continuation.
fn refresh_if_stale<'ctx>(
    context: &'ctx Context,
    builder: &Builder<'ctx>,
    module: &inkwell::module::Module<'ctx>,
    rt: &RtFns<'ctx>,
    header: &Header<'ctx>,
    cache: NdCache<'ctx>,
    nd: NdTy,
) {
    let function = builder
        .get_insert_block()
        .and_then(|b| b.get_parent())
        .expect("inside a function");
    let fresh = builder
        .build_load(context.bool_type(), cache.fresh, "nd_fresh")
        .expect("build_load should not fail")
        .into_int_value();
    let stale = context.append_basic_block(function, "nd_stale");
    let cont = context.append_basic_block(function, "nd_fresh_cont");
    builder
        .build_conditional_branch(fresh, cont, stale)
        .expect("build_conditional_branch should not fail");
    builder.position_at_end(stale);
    if nd.dtype == NdDtype::Byte {
        emit_byte_check(context, builder, module, rt, header.object, cache);
    } else {
        emit_check(context, builder, module, rt, header.object, nd);
        write_cache(context, builder, header.object, cache, nd.ndim);
    }
    builder
        .build_unconditional_branch(cont)
        .expect("build_unconditional_branch should not fail");
    builder.position_at_end(cont);
}

/// Snapshots `header`'s fields into the local's [`NdCache`], creating its
/// slots in the entry block on first use.
fn store_cache<'ctx>(
    context: &'ctx Context,
    builder: &Builder<'ctx>,
    rt: &RtFns<'ctx>,
    header: &Header<'ctx>,
    key: (String, String),
    ndim: u8,
) {
    let cache = cache_slots(context, builder, rt, header, key);
    write_cache(context, builder, header.object, cache, ndim);
}

/// The local's [`NdCache`] slots, created in the entry block (stale) on
/// first use.
fn cache_slots<'ctx>(
    context: &'ctx Context,
    builder: &Builder<'ctx>,
    rt: &RtFns<'ctx>,
    header: &Header<'ctx>,
    key: (String, String),
) -> NdCache<'ctx> {
    header.cache.unwrap_or_else(|| {
        let cache = new_cache(context, builder);
        rt.nd_caches.0.borrow_mut().insert(key, cache);
        cache
    })
}

/// A new, stale snapshot's entry-block slots.
fn new_cache<'ctx>(context: &'ctx Context, builder: &Builder<'ctx>) -> NdCache<'ctx> {
    let function = builder
        .get_insert_block()
        .and_then(|b| b.get_parent())
        .expect("inside a function");
    {
        let ptr_ty = context.ptr_type(inkwell::AddressSpace::default());
        let i64_type = context.i64_type();
        crate::build_at_entry_block(builder, function, |b| {
            let cache = NdCache {
                data: b.build_alloca(ptr_ty, "nd_cache_data").expect("alloca"),
                dims: [
                    b.build_alloca(i64_type, "nd_cache_dim0").expect("alloca"),
                    b.build_alloca(i64_type, "nd_cache_dim1").expect("alloca"),
                ],
                strides: [
                    b.build_alloca(i64_type, "nd_cache_stride0")
                        .expect("alloca"),
                    b.build_alloca(i64_type, "nd_cache_stride1")
                        .expect("alloca"),
                ],
                flags: b
                    .build_alloca(context.i32_type(), "nd_cache_flags")
                    .expect("alloca"),
                fresh: b
                    .build_alloca(context.bool_type(), "nd_cache_fresh")
                    .expect("alloca"),
            };
            b.build_store(cache.fresh, context.bool_type().const_zero())
                .expect("build_store should not fail");
            cache
        })
    }
}

/// Checks that `object` is exactly a `bytearray` (`pycc_ext_ba_check`,
/// raising `TypeError` otherwise) and snapshots its buffer and size into
/// `cache`, marking it fresh. The shim writes through two scratch slots of
/// its own, so `cache`'s slots stay unaddressed (promotable).
fn emit_byte_check<'ctx>(
    context: &'ctx Context,
    builder: &Builder<'ctx>,
    module: &inkwell::module::Module<'ctx>,
    rt: &RtFns<'ctx>,
    object: PointerValue<'ctx>,
    cache: NdCache<'ctx>,
) {
    let function = builder
        .get_insert_block()
        .and_then(|b| b.get_parent())
        .expect("inside a function");
    let ptr = context.ptr_type(inkwell::AddressSpace::default());
    let i64_type = context.i64_type();
    let (data_out, size_out) = crate::build_at_entry_block(builder, function, |b| {
        (
            b.build_alloca(ptr, "ba_data_out").expect("alloca"),
            b.build_alloca(i64_type, "ba_size_out").expect("alloca"),
        )
    });
    let edge = ForeignFailEdge::for_current(builder);
    let check = shim_fn(
        module,
        BA_CHECK_SYMBOL,
        context
            .i32_type()
            .fn_type(&[ptr.into(), ptr.into(), ptr.into()], false),
    );
    let status = builder
        .build_call(
            check,
            &[object.into(), data_out.into(), size_out.into()],
            "ba_check",
        )
        .expect("build_call should not fail")
        .try_as_basic_value()
        .expect_basic("pycc_ext_ba_check returns int")
        .into_int_value();
    route_negative(context, builder, module, rt, edge, status, "ba_check");
    let data = builder
        .build_load(ptr, data_out, "ba_data")
        .expect("build_load should not fail");
    let size = builder
        .build_load(i64_type, size_out, "ba_size")
        .expect("build_load should not fail");
    let stores = [
        (cache.data, data),
        (cache.dims[0], size),
        (cache.strides[0], i64_type.const_int(1, false).into()),
        (
            cache.flags,
            context.i32_type().const_int(WRITEABLE, false).into(),
        ),
        (cache.fresh, context.bool_type().const_int(1, false).into()),
    ];
    for (slot, value) in stores {
        builder
            .build_store(slot, value)
            .expect("build_store should not fail");
    }
}

/// Stores `object`'s header fields into `cache` and marks it fresh.
fn write_cache<'ctx>(
    context: &'ctx Context,
    builder: &Builder<'ctx>,
    object: PointerValue<'ctx>,
    cache: NdCache<'ctx>,
    ndim: u8,
) {
    let raw = Header {
        object,
        cache: None,
    };
    builder
        .build_store(cache.fresh, context.bool_type().const_int(1, false))
        .expect("build_store should not fail");
    builder
        .build_store(cache.data, raw.data(context, builder))
        .expect("build_store should not fail");
    builder
        .build_store(cache.flags, raw.flags(context, builder))
        .expect("build_store should not fail");
    for axis in 0..ndim {
        let index = usize::from(axis);
        builder
            .build_store(
                cache.dims[index],
                raw.raw_dim(context, builder, axis, false),
            )
            .expect("build_store should not fail");
        builder
            .build_store(
                cache.strides[index],
                raw.raw_dim(context, builder, axis, true),
            )
            .expect("build_store should not fail");
    }
}

/// The value a `None`-typed node evaluates to (`call_result`'s carrier).
fn none_value(context: &Context) -> Scalar<'_> {
    Scalar::Bool(context.i8_type().const_zero())
}

/// The shim's `pycc_ext_nd_canon`: `numpy.ndarray` and the canonical
/// `float64`, `int64` and `bool` dtypes, all null until the first
/// `pycc_ext_nd_check` has validated numpy's layout.
fn canon_global<'ctx>(
    context: &'ctx Context,
    module: &inkwell::module::Module<'ctx>,
) -> inkwell::values::GlobalValue<'ctx> {
    const CANON_SYMBOL: &str = "pycc_ext_nd_canon";
    module.get_global(CANON_SYMBOL).unwrap_or_else(|| {
        let ptr = context.ptr_type(inkwell::AddressSpace::default());
        let global = module.add_global(ptr.array_type(4), None, CANON_SYMBOL);
        global.set_linkage(inkwell::module::Linkage::External);
        global
    })
}

/// Checks that `object` is a `numpy.ndarray` of `nd`'s dtype and dimension
/// count, raising `TypeError` otherwise: inline when it is an exact
/// `numpy.ndarray` with the canonical dtype object, through
/// `pycc_ext_nd_check` for everything else (a subclass, an equal but
/// distinct dtype object, the first check of the process, a failure).
fn emit_check<'ctx>(
    context: &'ctx Context,
    builder: &Builder<'ctx>,
    module: &inkwell::module::Module<'ctx>,
    rt: &RtFns<'ctx>,
    object: PointerValue<'ctx>,
    nd: NdTy,
) {
    let function = builder
        .get_insert_block()
        .and_then(|b| b.get_parent())
        .expect("inside a function");
    let i64_type = context.i64_type();
    let ptr = context.ptr_type(inkwell::AddressSpace::default());
    let canon = canon_global(context, module).as_pointer_value();
    let load_canon = |index: u64, name: &str| {
        // SAFETY (LLVM): an in-bounds element of the four-pointer array.
        let slot = unsafe {
            builder
                .build_in_bounds_gep(ptr, canon, &[i64_type.const_int(index, false)], name)
                .expect("build_in_bounds_gep should not fail")
        };
        let load = builder
            .build_load(ptr, slot, name)
            .expect("build_load should not fail");
        tag(context, load, TAG_HEADER);
        load.into_pointer_value()
    };
    let nonnull = builder
        .build_is_not_null(object, "nd_nonnull")
        .expect("build_is_not_null should not fail");
    let typed = context.append_basic_block(function, "nd_check_fields");
    let slow = context.append_basic_block(function, "nd_check_slow");
    let done = context.append_basic_block(function, "nd_check_done");
    builder
        .build_conditional_branch(nonnull, typed, slow)
        .expect("build_conditional_branch should not fail");
    builder.position_at_end(typed);
    let header = Header {
        object,
        cache: None,
    };
    let ob_type = header.load_ptr(context, builder, 8, "nd_ob_type");
    let ndim = builder
        .build_load(
            context.i32_type(),
            header.field_ptr(context, builder, HEAD + 8),
            "nd_ndim",
        )
        .expect("build_load should not fail");
    tag(context, ndim, TAG_HEADER);
    let descr = header.load_ptr(context, builder, HEAD + 40, "nd_descr");
    let same_type = builder
        .build_int_compare(
            IntPredicate::EQ,
            ob_type,
            load_canon(0, "nd_canon_type"),
            "nd_same_type",
        )
        .expect("build_int_compare should not fail");
    let same_ndim = builder
        .build_int_compare(
            IntPredicate::EQ,
            ndim.into_int_value(),
            context.i32_type().const_int(u64::from(nd.ndim), false),
            "nd_same_ndim",
        )
        .expect("build_int_compare should not fail");
    let same_dtype = builder
        .build_int_compare(
            IntPredicate::EQ,
            descr,
            load_canon(1 + u64::from(nd.dtype.code()), "nd_canon_dtype"),
            "nd_same_dtype",
        )
        .expect("build_int_compare should not fail");
    let ok = builder
        .build_and(same_type, same_ndim, "nd_ok")
        .and_then(|both| builder.build_and(both, same_dtype, "nd_ok"))
        .expect("build_and should not fail");
    builder
        .build_conditional_branch(ok, done, slow)
        .expect("build_conditional_branch should not fail");
    builder.position_at_end(slow);
    let edge = ForeignFailEdge::for_current(builder);
    let check = shim_fn(
        module,
        ND_CHECK_SYMBOL,
        context
            .i32_type()
            .fn_type(&[ptr.into(), i64_type.into(), i64_type.into()], false),
    );
    let status = builder
        .build_call(
            check,
            &[
                object.into(),
                i64_type.const_int(u64::from(nd.dtype.code()), false).into(),
                i64_type.const_int(u64::from(nd.ndim), false).into(),
            ],
            "nd_check",
        )
        .expect("build_call should not fail")
        .try_as_basic_value()
        .expect_basic("pycc_ext_nd_check returns int")
        .into_int_value();
    route_negative(context, builder, module, rt, edge, status, "nd_check");
    builder
        .build_unconditional_branch(done)
        .expect("build_unconditional_branch should not fail");
    builder.position_at_end(done);
}

/// Branches to a cold block when `failed`, which calls `raise` (a shim
/// helper that sets an exception) and takes the foreign failure edge;
/// leaves the builder on the success continuation.
fn cold_raise<'ctx>(
    context: &'ctx Context,
    builder: &Builder<'ctx>,
    module: &inkwell::module::Module<'ctx>,
    rt: &RtFns<'ctx>,
    failed: IntValue<'ctx>,
    label: &str,
    raise: impl FnOnce(&Builder<'ctx>),
) {
    let edge = ForeignFailEdge::for_current(builder);
    let function = edge.function();
    let ok_bb = context.append_basic_block(function, &format!("{label}_ok"));
    let fail_bb = context.append_basic_block(function, &format!("{label}_fail"));
    crate::int_fast::likely_branch(
        context,
        builder,
        builder
            .build_not(failed, "nd_ok")
            .expect("build_not should not fail"),
        ok_bb,
        fail_bb,
    );
    builder.position_at_end(fail_bb);
    raise(builder);
    emit_failure(context, builder, module, rt, edge);
    builder.position_at_end(ok_bb);
}
