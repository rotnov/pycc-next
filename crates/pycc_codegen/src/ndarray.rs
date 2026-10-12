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

use super::*;
use crate::foreign_fail::{ForeignFailEdge, emit_failure, route_negative};
use crate::foreign_pack::shim_fn;
use inkwell::IntPredicate;
use inkwell::builder::Builder;
use inkwell::values::{BasicValue as _, IntValue, PointerValue};
use pycc_hir::ndarray::{NdDtype, NdOp, NdTy};
use pycc_mir::MirNdExpr;

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
    let header = Header {
        object,
        cache: cache_key
            .as_ref()
            .and_then(|key| rt.nd_caches.0.borrow().get(key).copied()),
    };
    if nd.op != NdOp::Check {
        match header.cache {
            Some(cache) => refresh_if_stale(context, builder, module, rt, &header, cache, nd.nd),
            // Anything but a cache-safe local is checked at each use: CPython
            // code can rebind a global or an attribute, or change any array
            // in place, without passing a compiled binding's check.
            None => emit_check(context, builder, module, rt, object, nd.nd),
        }
    }
    let result = match nd.op {
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
            let index_count = match nd.op {
                NdOp::Set { .. } => nd.args.len() - 1,
                _ => nd.args.len(),
            };
            let mut raw = Vec::with_capacity(index_count);
            let mut words = Vec::with_capacity(index_count);
            for index in &nd.args[..index_count] {
                let (value, word) =
                    raw_index(context, builder, module, rt, user_functions, locals, index);
                raw.push(value);
                words.extend(word);
            }
            let address = element_address(context, builder, module, rt, &header, &raw, chained);
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
                _ => load_element(context, builder, rt, address, nd.nd.dtype),
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
    emit_check(context, builder, module, rt, header.object, nd);
    write_cache(context, builder, header.object, cache, nd.ndim);
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
    let function = builder
        .get_insert_block()
        .and_then(|b| b.get_parent())
        .expect("inside a function");
    let cache = header.cache.unwrap_or_else(|| {
        let ptr_ty = context.ptr_type(inkwell::AddressSpace::default());
        let i64_type = context.i64_type();
        let cache = crate::build_at_entry_block(builder, function, |b| {
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
        });
        rt.nd_caches.0.borrow_mut().insert(key, cache);
        cache
    });
    write_cache(context, builder, header.object, cache, ndim);
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

/// Clears every [`NdCache::fresh`] of a function after each call in it that
/// may run CPython code: everything but an LLVM intrinsic other than a
/// coroutine suspension (a generator's consumer runs while it is
/// suspended), pycc_rt's number arithmetic, and this module's own shim
/// helpers. Run once the module is fully emitted.
pub(crate) fn clear_fresh_after_calls<'ctx>(
    context: &'ctx Context,
    module: &inkwell::module::Module<'ctx>,
    builder: &Builder<'ctx>,
    rt: &RtFns<'ctx>,
) {
    let mut by_function: HashMap<String, Vec<PointerValue<'ctx>>> = HashMap::new();
    for ((function, _), cache) in rt.nd_caches.0.borrow().iter() {
        by_function
            .entry(function.clone())
            .or_default()
            .push(cache.fresh);
    }
    let stale = context.bool_type().const_zero();
    for (name, flags) in by_function {
        let Some(function) = module.get_function(&name) else {
            continue;
        };
        let mut calls = Vec::new();
        for block in function.get_basic_blocks() {
            let mut next = block.get_first_instruction();
            while let Some(instruction) = next {
                next = instruction.get_next_instruction();
                if instruction.get_opcode() == inkwell::values::InstructionOpcode::Call
                    && may_run_python(instruction)
                    && let Some(after) = instruction.get_next_instruction()
                {
                    calls.push(after);
                }
            }
        }
        for after in calls {
            builder.position_before(&after);
            for flag in &flags {
                builder
                    .build_store(*flag, stale)
                    .expect("build_store should not fail");
            }
        }
    }
}

/// Whether `call` may run CPython code ([`clear_fresh_after_calls`]).
fn may_run_python(call: inkwell::values::InstructionValue<'_>) -> bool {
    let Ok(site) = inkwell::values::CallSiteValue::try_from(call) else {
        return true;
    };
    let Some(callee) = site.get_called_fn_value() else {
        return true;
    };
    let name = callee.get_name().to_string_lossy();
    if name.starts_with("llvm.") {
        return name.starts_with("llvm.coro.suspend");
    }
    !(name.starts_with("pycc_rt_int_")
        || name.starts_with("pycc_rt_bigint_")
        || name.starts_with("pycc_rt_float")
        || name.starts_with("pycc_ext_nd_"))
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
fn untag_index<'ctx>(
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
        .build_int_compare(IntPredicate::EQ, folded, i64_type.const_int(6, false), "nd_is_marker")
        .expect("build_int_compare should not fail");
    let decodable = builder.build_or(is_small, is_marker, "nd_decodable").expect("or");
    let big = builder.build_not(decodable, "nd_big_index").expect("not");
    cold_raise(context, builder, module, rt, big, "nd_index", |b| {
        let ptr = context.ptr_type(inkwell::AddressSpace::default());
        let raise = shim_fn(
            module,
            ND_INDEX_WORD_SYMBOL,
            context.i32_type().fn_type(&[i64_type.into(), ptr.into()], false),
        );
        b.build_call(raise, &[word.into(), ptr.const_null().into()], "nd_big_index")
            .expect("build_call should not fail");
    });
    let small = builder.build_right_shift(word, one, true, "nd_untag").expect("ashr");
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
fn is_index_leaf(expr: &MirExpr) -> bool {
    match expr {
        MirExpr::Name { ty, .. } => *ty == pycc_mir::Ty::Int,
        MirExpr::IntLiteral(k) => k.unsigned_abs() < (1 << 61),
        _ => false,
    }
}

/// The machine value of the index expression `expr`.
///
/// `i`, `i - 1` and `i + j` -- an `int` name or small literal, or the sum
/// or difference of two -- are computed on untagged machine words: each
/// leaf is below 2**62 in magnitude, so the sum cannot overflow `i64`, and
/// no `int` temporary is created. Anything else is evaluated as an `int`
/// and decoded ([`untag_index`]); the second element of the pair is its
/// word, for the caller to release.
#[allow(clippy::too_many_arguments)]
fn raw_index<'ctx, 'm>(
    context: &'ctx Context,
    builder: &Builder<'ctx>,
    module: &inkwell::module::Module<'ctx>,
    rt: &RtFns<'ctx>,
    user_functions: &HashMap<&str, UserFunction<'ctx>>,
    locals: &HashMap<String, StorageSlot<'ctx>>,
    expr: &'m MirExpr,
) -> (IntValue<'ctx>, Option<(&'m MirExpr, IntValue<'ctx>)>) {
    let leaf = |e: &MirExpr| match e {
        MirExpr::IntLiteral(k) => context.i64_type().const_int(*k as u64, true),
        _ => {
            let scalar = emit_expr(context, builder, module, rt, user_functions, locals, e);
            let word = to_numeric_encoded_int(context, builder, scalar);
            untag_index(context, builder, module, rt, word)
        }
    };
    match expr {
        MirExpr::BinOp {
            op: op @ (pycc_mir::BinOpKind::Add | pycc_mir::BinOpKind::Sub),
            left,
            right,
            ty: pycc_mir::Ty::Int,
        } if is_index_leaf(left) && is_index_leaf(right) => {
            let l = leaf(left);
            let r = leaf(right);
            let value = if *op == pycc_mir::BinOpKind::Add {
                builder.build_int_add(l, r, "nd_index_sum")
            } else {
                builder.build_int_sub(l, r, "nd_index_diff")
            }
            .expect("integer arithmetic should not fail");
            (value, None)
        }
        e if is_index_leaf(e) => (leaf(e), None),
        other => {
            let scalar = emit_expr(context, builder, module, rt, user_functions, locals, other);
            let word = to_numeric_encoded_int(context, builder, scalar);
            (
                untag_index(context, builder, module, rt, word),
                Some((other, word)),
            )
        }
    }
}

fn untag_word<'ctx>(
    context: &'ctx Context,
    builder: &Builder<'ctx>,
    module: &inkwell::module::Module<'ctx>,
    rt: &RtFns<'ctx>,
    word: IntValue<'ctx>,
    slow_symbol: &str,
    label: &str,
) -> IntValue<'ctx> {
    let i64_type = context.i64_type();
    if let Some(constant) = word.get_sign_extended_constant()
        && constant & 1 == 1
    {
        return i64_type.const_int((constant >> 1) as u64, true);
    }
    let edge = ForeignFailEdge::for_current(builder);
    let function = edge.function();
    let small_bb = context.append_basic_block(function, &format!("{label}_small"));
    let slow_bb = context.append_basic_block(function, &format!("{label}_slow"));
    let join_bb = context.append_basic_block(function, &format!("{label}_join"));
    let low = builder
        .build_and(word, i64_type.const_int(1, false), "nd_low")
        .expect("build_and should not fail");
    let is_small = builder
        .build_int_compare(IntPredicate::NE, low, i64_type.const_zero(), "nd_is_small")
        .expect("build_int_compare should not fail");
    crate::int_fast::likely_branch(context, builder, is_small, small_bb, slow_bb);
    builder.position_at_end(small_bb);
    let small = builder
        .build_right_shift(word, i64_type.const_int(1, false), true, "nd_untag")
        .expect("build_right_shift should not fail");
    builder
        .build_unconditional_branch(join_bb)
        .expect("br should not fail");
    builder.position_at_end(slow_bb);
    let out = crate::build_at_entry_block(builder, function, |b| {
        b.build_alloca(i64_type, "nd_word_out")
            .expect("build_alloca should not fail")
    });
    let ptr = context.ptr_type(inkwell::AddressSpace::default());
    let slow = shim_fn(
        module,
        slow_symbol,
        context
            .i32_type()
            .fn_type(&[i64_type.into(), ptr.into()], false),
    );
    let status = builder
        .build_call(slow, &[word.into(), out.into()], label)
        .expect("build_call should not fail")
        .try_as_basic_value()
        .expect_basic("an nd word helper returns int")
        .into_int_value();
    route_negative(context, builder, module, rt, edge, status, label);
    let decoded = builder
        .build_load(i64_type, out, "nd_word")
        .expect("build_load should not fail")
        .into_int_value();
    let slow_end = builder.get_insert_block().expect("positioned");
    builder
        .build_unconditional_branch(join_bb)
        .expect("br should not fail");
    builder.position_at_end(join_bb);
    let phi = builder
        .build_phi(i64_type, label)
        .expect("build_phi should not fail");
    phi.add_incoming(&[(&small, small_bb), (&decoded, slow_end)]);
    phi.as_basic_value().into_int_value()
}

/// The address of the element at `indices`, after bounds-checking each one
/// against its axis (a negative index counts from the end).
///
/// `chained` is `a[i][j]`: the second index indexes a row, a 1-D array, so
/// numpy reports it against axis 0.
fn element_address<'ctx>(
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

fn load_element<'ctx>(
    context: &'ctx Context,
    builder: &Builder<'ctx>,
    rt: &RtFns<'ctx>,
    address: PointerValue<'ctx>,
    dtype: NdDtype,
) -> Scalar<'ctx> {
    let element_ty: inkwell::types::BasicTypeEnum<'ctx> = match dtype {
        NdDtype::F64 => context.f64_type().into(),
        NdDtype::I64 => context.i64_type().into(),
        NdDtype::Bool => context.i8_type().into(),
    };
    let load = builder
        .build_load(element_ty, address, "nd_load")
        .expect("build_load should not fail");
    if let Some(instruction) = load.as_instruction_value() {
        instruction
            .set_alignment(1)
            .expect("a load accepts an alignment");
        crate::tbaa::tag(context, instruction, TAG_ELEMENT);
    }
    match dtype {
        NdDtype::F64 => Scalar::Float(load.into_float_value()),
        NdDtype::I64 => Scalar::Int(tag_i64(context, builder, rt, load.into_int_value())),
        NdDtype::Bool => {
            let nonzero = builder
                .build_int_compare(
                    IntPredicate::NE,
                    load.into_int_value(),
                    context.i8_type().const_zero(),
                    "nd_bool",
                )
                .expect("build_int_compare should not fail");
            Scalar::Bool(
                builder
                    .build_int_z_extend(nonzero, context.i8_type(), "nd_bool_byte")
                    .expect("build_int_z_extend should not fail"),
            )
        }
    }
}

/// Encodes a machine `int64` as an `int` word: a smallint inline when it
/// fits 63 bits, a bigint through `pycc_rt_int_from_i64` otherwise.
fn tag_i64<'ctx>(
    context: &'ctx Context,
    builder: &Builder<'ctx>,
    rt: &RtFns<'ctx>,
    value: IntValue<'ctx>,
) -> IntValue<'ctx> {
    let i64_type = context.i64_type();
    let function = builder
        .get_insert_block()
        .and_then(|b| b.get_parent())
        .expect("inside a function");
    let one = i64_type.const_int(1, false);
    let shifted = builder.build_left_shift(value, one, "nd_shl").expect("shl");
    let back = builder
        .build_right_shift(shifted, one, true, "nd_sar")
        .expect("sar");
    let fits = builder
        .build_int_compare(IntPredicate::EQ, back, value, "nd_fits")
        .expect("build_int_compare should not fail");
    let small_bb = context.append_basic_block(function, "nd_tag_small");
    let big_bb = context.append_basic_block(function, "nd_tag_big");
    let join_bb = context.append_basic_block(function, "nd_tag_join");
    crate::int_fast::likely_branch(context, builder, fits, small_bb, big_bb);
    builder.position_at_end(small_bb);
    let small = builder.build_or(shifted, one, "nd_tagged").expect("or");
    builder.build_unconditional_branch(join_bb).expect("br");
    builder.position_at_end(big_bb);
    let big = builder
        .build_call(rt.int_from_i64, &[value.into()], "nd_bigint")
        .expect("build_call should not fail")
        .try_as_basic_value()
        .expect_basic("pycc_rt_int_from_i64 returns i64")
        .into_int_value();
    builder.build_unconditional_branch(join_bb).expect("br");
    builder.position_at_end(join_bb);
    let phi = builder.build_phi(i64_type, "nd_int").expect("phi");
    phi.add_incoming(&[(&small, small_bb), (&big, big_bb)]);
    phi.as_basic_value().into_int_value()
}

fn emit_writeable_check<'ctx>(
    context: &'ctx Context,
    builder: &Builder<'ctx>,
    module: &inkwell::module::Module<'ctx>,
    rt: &RtFns<'ctx>,
    header: &Header<'ctx>,
) {
    let flags = header.flags(context, builder);
    let i32_type = context.i32_type();
    let bit = builder
        .build_and(flags, i32_type.const_int(WRITEABLE, false), "nd_writeable")
        .expect("build_and should not fail");
    let read_only = builder
        .build_int_compare(IntPredicate::EQ, bit, i32_type.const_zero(), "nd_read_only")
        .expect("build_int_compare should not fail");
    cold_raise(
        context,
        builder,
        module,
        rt,
        read_only,
        "nd_readonly",
        |b| {
            let raise = shim_fn(module, ND_READONLY_SYMBOL, i32_type.fn_type(&[], false));
            b.build_call(raise, &[], "nd_readonly")
                .expect("build_call should not fail");
        },
    );
}

#[allow(clippy::too_many_arguments)]
fn store_element<'ctx>(
    context: &'ctx Context,
    builder: &Builder<'ctx>,
    module: &inkwell::module::Module<'ctx>,
    rt: &RtFns<'ctx>,
    address: PointerValue<'ctx>,
    dtype: NdDtype,
    value_expr: &MirExpr,
    value: Scalar<'ctx>,
) {
    let stored: inkwell::values::BasicValueEnum<'ctx> = match dtype {
        NdDtype::F64 => {
            let word = match value {
                Scalar::Int(word) => Some(word),
                _ => None,
            };
            let float = to_float(context, builder, rt, value);
            if let Some(word) = word {
                release_if_int_temporary(context, builder, rt, value_expr, word);
            }
            float.into()
        }
        NdDtype::I64 => match value {
            Scalar::Bool(byte) => builder
                .build_int_z_extend(byte, context.i64_type(), "nd_bool_word")
                .expect("build_int_z_extend should not fail")
                .into(),
            other => {
                let word = to_numeric_encoded_int(context, builder, other);
                let raw = untag_word(
                    context,
                    builder,
                    module,
                    rt,
                    word,
                    ND_INT_WORD_SYMBOL,
                    "nd_int_value",
                );
                release_if_int_temporary(context, builder, rt, value_expr, word);
                raw.into()
            }
        },
        NdDtype::Bool => match value {
            Scalar::Bool(byte) => byte.into(),
            _ => panic!("pycc_codegen: internal error: a bool array stores a non-bool value"),
        },
    };
    let store = builder
        .build_store(address, stored)
        .expect("build_store should not fail");
    store
        .set_alignment(1)
        .expect("a store accepts an alignment");
    crate::tbaa::tag(context, store, TAG_ELEMENT);
}
