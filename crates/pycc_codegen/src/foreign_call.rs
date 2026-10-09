//! Emission for `MirExpr::ObjMethodCall` (Part 2 of #1026, PR 2b of #1081),
//! `MirExpr::ObjCall` (#1313, through the same argument marshalling; a
//! computed callee since Part 2a of #1371, see [`callee_is_produced`]),
//! their keyword-argument form `MirExpr::ObjKeywordCall` ([`emit_call_kw`],
//! Part 8 of #1371; `foreign_call_emit.rs` drives it from the MIR),
//! `MirExpr::ObjSubscript` (Part 3 of #1026, PR 3b of #1082),
//! `MirExpr::ObjList` (Part 2d of #1371, see [`emit_list`]) and
//! `MirStmt::ForObject` (PR 3c of #1082). A slice of an object, loaded or
//! deleted, is `foreign_slice.rs`'s (Parts 2b and 2c of #1371).
//!
//! They share this module because they share the *packer contract*: each
//! marshals a pycc operand into a `PyObject *` through a `pycc_ext_obj_pack_*`
//! helper and hands the resulting owned reference to a shim helper that
//! releases it. The contract's primitives live in `foreign_pack.rs` (split
//! out in Part 2a of #1371), which `foreign_compare.rs` uses too, so it
//! cannot fork into two spellings.
//!
//! The call counterpart of `foreign_attr.rs`, which this module reuses for
//! `expect_object_pointer`. Every failure edge is `foreign_fail.rs`'s: the
//! module-exec return inside `pycc_ext_module_exec` when no module-level
//! `try` encloses the operation, and otherwise the bridge plus an immediate
//! branch to the innermost exception target (#1316, Part 1 of #1096).
//! `emit_iter_loop` alone keeps the
//! `expect_module_exec_entry` assertion, because `pycc_types` still admits
//! `for x in <object>:` only in a module body (Part 2 of #1333); its two
//! halves, which a comprehension over an object shares (Part 1 of #1255),
//! take whichever edge the current function has.
//! What is new here is *argument marshalling*: each already-evaluated pycc
//! scalar becomes a `PyObject *` through one of the shim's
//! `pycc_ext_obj_pack_*` helpers, the results go into a stack array, and
//! `pycc_ext_obj_call` vectorcalls the already-resolved bound method.
//!
//! **Why the lookup is a separate step.** CPython resolves a call's
//! callable before it evaluates the arguments, so `obj.missing(1 // 0)`
//! raises `AttributeError` rather than `ZeroDivisionError`. `emit_lookup`
//! therefore emits `pycc_ext_obj_getattr` and its NULL check, and
//! `emit_expr`'s own arm runs it before the argument expressions; a fused
//! shim that did the load itself would necessarily run it last. The price
//! is a second NULL check and a second failure edge. The bound method still
//! never becomes a pycc value -- it is an LLVM temporary that
//! `pycc_ext_obj_call` releases -- so it does not join the boundary's
//! leaked set. `src/ext/pycc_ext_module.c`'s own comment on
//! `pycc_ext_obj_call` carries the full rationale.
//!
//! **Ownership** (`docs/RUNTIME.md`). The packers *borrow* their pycc-side
//! arguments -- ownership of every argument stays with the compiled module
//! body, so nothing here has to balance a transfer, and no
//! `pending_int_releases` bookkeeping (D-208, #638) is needed: that machinery
//! exists for a transfer, and this boundary performs none. The `PyObject *`
//! references the packers create are owned by the array and consumed by
//! `pycc_ext_obj_call` on every path, so the only thing that outlives the
//! call is its *result* -- a new reference whose release is its consumer's
//! business, exactly as `foreign_attr.rs` documents for an attribute load.
//! A *produced* argument (`o.m(p.q)`) is a different matter since Part 1 of
//! #1092: the packer took a reference of its own, so the argument is held
//! across the call and released after it (`foreign_call_emit.rs`).

use super::*;
use crate::foreign_attr::{expect_module_exec_entry, expect_object_pointer};
use crate::foreign_fail::{ForeignFailEdge, emit_failure, route_null};
use crate::foreign_pack::{emit_pack, shim_fn};
use inkwell::builder::Builder;
use inkwell::values::BasicMetadataValueEnum;

/// Allocates `slots` argument pointers in the *entry block* of `entry_fn`,
/// leaving the builder positioned exactly where it was.
///
/// An `alloca` is only reclaimed when its function returns, so emitting one
/// at the call site would make a module body's own loop grow the stack
/// without bound: `for i in range(n): gc.disable()` is an admitted program
/// -- and it segfaulted the hosting interpreter
/// at twenty million iterations before this hoist. The size is a compile-time
/// constant that depends on nothing in scope, so the entry block is always a
/// legal position for it, and LLVM's own convention is that every `alloca`
/// belongs there.
fn alloca_in_entry_block<'ctx>(
    context: &'ctx Context,
    builder: &Builder<'ctx>,
    entry_fn: FunctionValue<'ctx>,
    slots: usize,
) -> inkwell::values::PointerValue<'ctx> {
    super::build_at_entry_block(builder, entry_fn, |b| {
        b.build_array_alloca(
            context.ptr_type(inkwell::AddressSpace::default()),
            context.i64_type().const_int(slots as u64, false),
            "foreign_call_args",
        )
        .expect("build_array_alloca should not fail")
    })
}

/// Packs every scalar of `values` through its `pycc_ext_obj_pack_*` helper
/// into a `PyObject *` array hoisted into `entry_fn`'s entry block, in
/// order, and answers the array.
///
/// The one spelling of the array half of the packer contract, shared by a
/// call's arguments ([`emit_call_with`]) and a list display's elements
/// ([`emit_list`]): the helper each array is handed to consumes every slot
/// on every path, a failed packer's `NULL` included, so no slot is checked
/// here.
///
/// A zero-length array would be `alloca [0 x ptr]`, a pointer it is not
/// meaningful to GEP into -- and `gc.disable()` and an empty `[]` are both
/// zero-length shapes. One slot is always allocated and simply left
/// unread; the consumer reads only the count it is passed.
fn emit_packed_array<'ctx>(
    context: &'ctx Context,
    builder: &Builder<'ctx>,
    module: &inkwell::module::Module<'ctx>,
    entry_fn: FunctionValue<'ctx>,
    values: &[Scalar<'ctx>],
) -> inkwell::values::PointerValue<'ctx> {
    let ptr = context.ptr_type(inkwell::AddressSpace::default());
    let i64_type = context.i64_type();
    let slots = values.len().max(1);
    let array = alloca_in_entry_block(context, builder, entry_fn, slots);
    for (index, value) in values.iter().enumerate() {
        let packed = emit_pack(context, builder, module, *value, "foreign_call_arg");
        let slot = unsafe {
            builder
                .build_in_bounds_gep(
                    ptr,
                    array,
                    &[i64_type.const_int(index as u64, false)],
                    "foreign_call_arg_slot",
                )
                .expect("build_in_bounds_gep should not fail")
        };
        builder
            .build_store(slot, packed)
            .expect("build_store should not fail");
    }
    array
}

/// Emits one list display bound to a CPython object slot (Part 2d of
/// #1371), yielding the fresh CPython `list` as an opaque
/// [`Scalar::Object`].
///
/// The elements are already evaluated, left to right, by `emit_expr`'s own
/// arm. Each is packed into a hoisted array ([`emit_packed_array`]) and
/// handed to `pycc_ext_obj_build_list`, which consumes every packed
/// reference on every path -- a failed packer's `NULL` and a failed
/// `PyList_New` included -- so the display keeps exactly one failure edge,
/// on a `NULL` result. The list is a new reference, owned like every other
/// object this boundary produces: released by its consumer when it is an
/// unbound temporary (Part 1 of #1092), owned by a module global that binds
/// it (Part 1 of #1499), leaked otherwise.
pub(super) fn emit_list<'ctx>(
    context: &'ctx Context,
    builder: &Builder<'ctx>,
    module: &inkwell::module::Module<'ctx>,
    rt: &RtFns<'ctx>,
    elements: &[Scalar<'ctx>],
) -> Scalar<'ctx> {
    let edge = ForeignFailEdge::for_current(builder);
    let ptr = context.ptr_type(inkwell::AddressSpace::default());
    let i64_type = context.i64_type();
    let items = emit_packed_array(context, builder, module, edge.function(), elements);
    let build_list = shim_fn(
        module,
        EXT_OBJ_BUILD_LIST_SYMBOL,
        ptr.fn_type(&[ptr.into(), i64_type.into()], false),
    );
    let result = builder
        .build_call(
            build_list,
            &[
                items.into(),
                i64_type.const_int(elements.len() as u64, false).into(),
            ],
            "foreign_list",
        )
        .expect("build_call should not fail for pycc_ext_obj_build_list")
        .try_as_basic_value()
        .expect_basic("pycc_ext_obj_build_list returns PyObject *")
        .into_pointer_value();
    route_null(context, builder, module, rt, edge, result, "foreign_list");
    Scalar::Object(result)
}

/// The blocks and per-iteration item of a lowered `for x in <object>:`
/// loop, handed back to `emit_stmt` so it can emit the body between them.
pub(super) struct ForeignIterLoop<'ctx> {
    /// The block holding the `pycc_ext_obj_iter_next` call and the
    /// three-way switch; the body's back-edge targets it.
    pub header_bb: inkwell::basic_block::BasicBlock<'ctx>,
    /// Where control resumes after clean exhaustion.
    pub after_bb: inkwell::basic_block::BasicBlock<'ctx>,
    /// A *new* reference to this iteration's item. A module-global loop
    /// target owns it and releases it on the next trip (Part 1 of #1499,
    /// `object_slot::store_new_reference`). `pycc_types` admits an object
    /// `for` statement only in a module body, so its target is always a
    /// module global; `store_new_reference`'s frame-slot arm is defensive.
    /// A comprehension shares this struct and stores `item` into a scoped
    /// slot that does not own it, so it still leaks one item per trip
    /// (#1499).
    pub item: PointerValue<'ctx>,
}

/// Emits the preheader and header of `for x in <object>:` (Part 3 of
/// #1026, PR 3c of #1082), leaving the builder positioned at the start of
/// the loop body with the first item already loaded.
///
/// # Shape
///
/// The preheader calls [`EXT_OBJ_GET_ITER_SYMBOL`] once ([`emit_get_iter`])
/// -- Python binds the iterator the `for` statement evaluated, so a
/// body-level rebinding cannot retarget the loop, exactly the reasoning
/// `MirStmt::ForList`'s own `list_ptr` read carries -- and routes a NULL
/// through the failure edge. The header ([`emit_iter_header`]) calls
/// [`EXT_OBJ_ITER_NEXT_SYMBOL`] and **switches** on its three-valued
/// result: `1` enters the body, `0` exits the loop, and anything else --
/// `-1` and, fail-closed, any value the shim could not produce -- takes a
/// second failure edge of its own.
///
/// Those two are the only failure edges the loop adds; **exhaustion is
/// deliberately not one of them**, which is the entire reason the shim
/// helper is three-valued rather than NULL-signalling (see
/// [`EXT_OBJ_ITER_NEXT_SYMBOL`]).
///
/// `pycc_types` admits the statement only in a module body (Part 2 of
/// #1333), so the edge is always a [`ForeignFailEdge::ModuleExec`] there:
/// the module-exec return outside every module-level `try`, bridged inside
/// one (Part 1 of #1096). A list or set comprehension over an object
/// (Part 1 of #1255, `object_comprehension.rs`) shares the two halves in
/// any function, which is why each takes its edge from
/// [`ForeignFailEdge::for_current`].
///
/// `held` is the iterable's hold (`object_release::hold`): a produced
/// iterable (`for x in o.items():`) is released once `iter()` has taken
/// what it needs, before the loop header (Part 1 of #1092).
///
/// The iterator itself is owned by the returned
/// [`LoopIterator`](crate::object_release::LoopIterator) from the moment
/// `iter()` succeeds (Part 3 of #1092): its cleanup target, installed
/// before the header, releases it on every exception edge out of the
/// header and the body, and the caller calls its `exit` at `after_bb`
/// once the body is emitted.
pub(super) fn emit_iter_loop<'ctx>(
    context: &'ctx Context,
    builder: &Builder<'ctx>,
    module: &inkwell::module::Module<'ctx>,
    rt: &RtFns<'ctx>,
    iterable: Scalar<'ctx>,
    held: crate::object_release::Held<'ctx>,
) -> (
    ForeignIterLoop<'ctx>,
    crate::object_release::LoopIterator<'ctx>,
) {
    expect_module_exec_entry(builder);
    let iterator = emit_get_iter(context, builder, module, rt, iterable);
    held.release(builder, rt);
    let owned = crate::object_release::enter_loop_iterator(context, builder, module, rt, iterator);
    (
        emit_iter_header(context, builder, module, rt, iterator),
        owned,
    )
}

/// The preheader half of [`emit_iter_loop`]: `iter(iterable)`, with its
/// NULL routed to the current function's failure edge. Returns the
/// iterator, a *new* reference the caller owns and releases on every exit
/// (Part 3 of #1092): [`emit_iter_loop`] through its
/// `object_release::LoopIterator`, a comprehension over an object through
/// the pending stack.
pub(super) fn emit_get_iter<'ctx>(
    context: &'ctx Context,
    builder: &Builder<'ctx>,
    module: &inkwell::module::Module<'ctx>,
    rt: &RtFns<'ctx>,
    iterable: Scalar<'ctx>,
) -> PointerValue<'ctx> {
    let edge = ForeignFailEdge::for_current(builder);
    let iterable_ptr = expect_object_pointer(iterable);
    let ptr = context.ptr_type(inkwell::AddressSpace::default());
    let get_iter = shim_fn(
        module,
        EXT_OBJ_GET_ITER_SYMBOL,
        ptr.fn_type(&[ptr.into()], false),
    );
    let iterator = builder
        .build_call(get_iter, &[iterable_ptr.into()], "foreign_iter_get")
        .expect("build_call should not fail for pycc_ext_obj_get_iter")
        .try_as_basic_value()
        .expect_basic("pycc_ext_obj_get_iter returns PyObject *")
        .into_pointer_value();
    route_null(
        context,
        builder,
        module,
        rt,
        edge,
        iterator,
        "foreign_iter_get",
    );
    iterator
}

/// The header half of [`emit_iter_loop`]: branches from the current block
/// into a header that advances `iterator` and switches on the result, and
/// leaves the builder at the start of the body with the item loaded.
///
/// The out-parameter is a single `alloca` hoisted into the entry block via
/// [`alloca_in_entry_block`], never the header: an `alloca` in a block that
/// executes once per iteration grows the frame without bound, which is the
/// defect that helper exists to prevent.
pub(super) fn emit_iter_header<'ctx>(
    context: &'ctx Context,
    builder: &Builder<'ctx>,
    module: &inkwell::module::Module<'ctx>,
    rt: &RtFns<'ctx>,
    iterator: PointerValue<'ctx>,
) -> ForeignIterLoop<'ctx> {
    let edge = ForeignFailEdge::for_current(builder);
    let function = edge.function();
    let ptr = context.ptr_type(inkwell::AddressSpace::default());

    let out_slot = alloca_in_entry_block(context, builder, function, 1);

    let header_bb = context.append_basic_block(function, "foreign_iter_header");
    let body_bb = context.append_basic_block(function, "foreign_iter_body");
    let after_bb = context.append_basic_block(function, "foreign_iter_after");
    let next_fail_bb = context.append_basic_block(function, "foreign_iter_next_fail");

    builder
        .build_unconditional_branch(header_bb)
        .expect("build_unconditional_branch should not fail entering the loop header");

    builder.position_at_end(header_bb);
    let iter_next = shim_fn(
        module,
        EXT_OBJ_ITER_NEXT_SYMBOL,
        context.i64_type().fn_type(&[ptr.into(), ptr.into()], false),
    );
    let status = builder
        .build_call(
            iter_next,
            &[iterator.into(), out_slot.into()],
            "foreign_iter_status",
        )
        .expect("build_call should not fail for pycc_ext_obj_iter_next")
        .try_as_basic_value()
        .expect_basic("pycc_ext_obj_iter_next returns long long")
        .into_int_value();
    builder
        .build_switch(
            status,
            next_fail_bb,
            &[
                (context.i64_type().const_int(1, false), body_bb),
                (context.i64_type().const_zero(), after_bb),
            ],
        )
        .expect("build_switch should not fail for an i64 selector");

    builder.position_at_end(next_fail_bb);
    emit_failure(context, builder, module, rt, edge);

    builder.position_at_end(body_bb);
    let item = builder
        .build_load(ptr, out_slot, "foreign_iter_item")
        .expect("build_load should not fail for a slot this function allocated")
        .into_pointer_value();

    ForeignIterLoop {
        header_bb,
        after_bb,
        item,
    }
}

/// Emits the *callable lookup* of one `obj.method(args)` call, yielding the
/// bound method as an owned `PyObject *` that [`emit_call`] consumes.
///
/// Split from [`emit_call`] so that `emit_expr`'s own arm can run it
/// *before* it evaluates the argument expressions. CPython resolves a
/// call's callable first and only then evaluates the arguments, so
/// `obj.missing(1 // 0)` raises `AttributeError`; an earlier revision of
/// this module performed the lookup inside the call shim, after every
/// argument, and raised `ZeroDivisionError` instead.
///
/// The bound method still never becomes a pycc value: it is an LLVM
/// temporary that dominates the `pycc_ext_obj_call` consuming it, so it is
/// released rather than joining the boundary's leaked set. If an argument
/// fails before that call, the bound method is still released: the caller
/// holds it across the arguments (`foreign_call_emit::emit_method_call`,
/// Part 1 of #1092). Keeping it in an
/// SSA value rather than an `alloca` also matters -- an `alloca` here would
/// grow a module-scope loop's stack per iteration, which is the defect
/// [`alloca_in_entry_block`] exists to prevent.
///
/// # Failure edge
///
/// A missing method returns `NULL` with CPython's `AttributeError` set,
/// which `foreign_fail::route_null` routes: the module-exec return in a
/// module body outside every module-level `try`, the bridge and an
/// immediate branch to the innermost exception target anywhere else
/// (#1316, #1096). The branch is
/// immediate because the arguments are evaluated next, with no guard in
/// between.
pub(super) fn emit_lookup<'ctx>(
    context: &'ctx Context,
    builder: &Builder<'ctx>,
    module: &inkwell::module::Module<'ctx>,
    rt: &RtFns<'ctx>,
    base: Scalar<'ctx>,
    method: &str,
) -> inkwell::values::PointerValue<'ctx> {
    let edge = ForeignFailEdge::for_current(builder);
    let base_ptr = expect_object_pointer(base);
    let ptr = context.ptr_type(inkwell::AddressSpace::default());
    let name = builder
        .build_global_string_ptr(method, &format!("pycc_foreign_method_{method}"))
        .expect("build_global_string_ptr should not fail")
        .as_pointer_value();
    let getattr = shim_fn(
        module,
        EXT_OBJ_GETATTR_SYMBOL,
        ptr.fn_type(&[ptr.into(), ptr.into()], false),
    );
    let bound = builder
        .build_call(
            getattr,
            &[base_ptr.into(), name.into()],
            "foreign_call_bound",
        )
        .expect("build_call should not fail for pycc_ext_obj_getattr")
        .try_as_basic_value()
        .expect_basic("pycc_ext_obj_getattr returns PyObject *")
        .into_pointer_value();
    route_null(
        context,
        builder,
        module,
        rt,
        edge,
        bound,
        "foreign_call_lookup",
    );
    bound
}

/// Marshals `args` and calls `bound`, yielding the call's result as an
/// opaque [`Scalar::Object`].
///
/// `bound` comes from [`emit_lookup`] and `args` are already-evaluated
/// scalars, so this function never recurses into the MIR and the evaluation
/// order visible in the emitted code is exactly CPython's: base, callable,
/// then each argument left to right.
///
/// `pycc_ext_obj_call` consumes `bound` and every packed argument on every
/// path; only the call's *result* outlives it, owned on the rule
/// `foreign_attr.rs` documents for an attribute load.
pub(super) fn emit_call<'ctx>(
    context: &'ctx Context,
    builder: &Builder<'ctx>,
    module: &inkwell::module::Module<'ctx>,
    rt: &RtFns<'ctx>,
    bound: inkwell::values::PointerValue<'ctx>,
    args: &[Scalar<'ctx>],
) -> Scalar<'ctx> {
    emit_call_with(
        context,
        builder,
        module,
        rt,
        EXT_OBJ_CALL_SYMBOL,
        bound,
        args,
        &[],
    )
}

/// Marshals `args` and calls `callee` *itself* (#1313), yielding the call's
/// result as an opaque [`Scalar::Object`].
///
/// `callee` is an already-evaluated *borrowed* object -- a `MirExpr::Name`
/// read of a reference the caller keeps (a retained module global, or a
/// `for` loop target's slot; `lib.rs`'s `Ty::Object` load arm), or any
/// other callee [`callee_is_produced`] does not list. It therefore goes to
/// [`EXT_OBJ_CALL_BORROWED_SYMBOL`], which takes its own reference before
/// delegating to the consuming `pycc_ext_obj_call`; passing the callee
/// straight to [`emit_call`]'s consuming helper would release the caller's
/// reference on the first call. The packed
/// arguments are consumed on every path, exactly as for a method call.
pub(super) fn emit_call_borrowed<'ctx>(
    context: &'ctx Context,
    builder: &Builder<'ctx>,
    module: &inkwell::module::Module<'ctx>,
    rt: &RtFns<'ctx>,
    callee: Scalar<'ctx>,
    args: &[Scalar<'ctx>],
) -> Scalar<'ctx> {
    let callee_ptr = expect_object_pointer(callee);
    emit_call_with(
        context,
        builder,
        module,
        rt,
        EXT_OBJ_CALL_BORROWED_SYMBOL,
        callee_ptr,
        args,
        &[],
    )
}

/// Whether an `ObjCall` callee evaluates to a *new* reference that nothing
/// else holds, so the call may consume it (Part 2a of #1371).
///
/// The allowlist of the shim's own new-reference producers is
/// `object_release::is_produced`'s, the one classification every consumer
/// of an object temporary shares (Part 1 of #1092): a subscript load, an
/// attribute load, a method or direct call's result, `type(o)`, a slice, a
/// list display, a tuple unpack and a rich comparison. Handing such a
/// result to the consuming `pycc_ext_obj_call` is what releases it. Every other
/// callee is a *borrow* and goes to `pycc_ext_obj_call_borrowed`: a `Name`
/// read (#1313), and also the pycc `__class_getitem__` method call that
/// `C[k]` lowers to for a pycc class base, whose `object` result is the
/// caller's pointer handed back without a new reference
/// (`docs/RUNTIME.md`). Consuming that would underflow the refcount; the
/// borrowed helper is at worst a leak, which is why an unlisted node
/// defaults to it. Unlike `object_release::hold`, this does not look through
/// `MirExpr::ObjectUnbox`: a callee is evaluated by `emit_expr` itself, never
/// through `object_unbox::emit_pack_operand`, so an unboxed read never
/// reaches here as an object.
pub(super) fn callee_is_produced(callee: &MirExpr) -> bool {
    crate::object_release::is_produced(callee)
}

/// Emits `callee(args)` for an `ObjCall` once the callee and arguments are
/// evaluated, routing the callee to the consuming or the borrowing shim
/// helper by its MIR shape ([`callee_is_produced`]). A produced callee whose
/// arguments raise before this point is released by the argument's own
/// failure edge (`foreign_call_emit::emit_direct_call` holds it there).
pub(super) fn emit_object_call<'ctx>(
    context: &'ctx Context,
    builder: &Builder<'ctx>,
    module: &inkwell::module::Module<'ctx>,
    rt: &RtFns<'ctx>,
    callee_mir: &MirExpr,
    callee: Scalar<'ctx>,
    args: &[Scalar<'ctx>],
) -> Scalar<'ctx> {
    if callee_is_produced(callee_mir) {
        let callee_ptr = expect_object_pointer(callee);
        emit_call(context, builder, module, rt, callee_ptr, args)
    } else {
        emit_call_borrowed(context, builder, module, rt, callee, args)
    }
}

/// Marshals `args` and calls `callable` with the last `names.len()` of them
/// passed as keyword arguments named `names` (Part 8 of #1371), yielding the
/// call's result as an opaque [`Scalar::Object`].
///
/// `args` holds the positional arguments followed by the keyword values, in
/// the order they were evaluated, which is CPython's. `consume` says who
/// owns `callable`: a bound method from [`emit_lookup`] or a callee
/// [`callee_is_produced`] lists is a new reference the call consumes
/// ([`EXT_OBJ_CALL_KW_SYMBOL`]); anything else is a borrow
/// ([`EXT_OBJ_CALL_KW_BORROWED_SYMBOL`]), exactly the split
/// [`emit_object_call`] makes for a positional call. The shim builds the
/// `kwnames` tuple from `names` and vectorcalls, consuming every packed
/// argument on every path.
#[allow(clippy::too_many_arguments)]
pub(super) fn emit_call_kw<'ctx>(
    context: &'ctx Context,
    builder: &Builder<'ctx>,
    module: &inkwell::module::Module<'ctx>,
    rt: &RtFns<'ctx>,
    consume: bool,
    callable: inkwell::values::PointerValue<'ctx>,
    args: &[Scalar<'ctx>],
    names: &[String],
) -> Scalar<'ctx> {
    debug_assert!(
        !names.is_empty() && names.len() <= args.len(),
        "a keyword call passes at least one keyword value"
    );
    let symbol = if consume {
        EXT_OBJ_CALL_KW_SYMBOL
    } else {
        EXT_OBJ_CALL_KW_BORROWED_SYMBOL
    };
    emit_call_with(context, builder, module, rt, symbol, callable, args, names)
}

/// The shared body of [`emit_call`], [`emit_call_borrowed`] and
/// [`emit_call_kw`]: the positional helpers take the same
/// `(callable, args, nargs)` parameters and differ only in who owns
/// `callable`; the keyword helpers append `(names, nkw)`, where the last
/// `nkw` of the `args` slots are the keyword values. The packer loop, the
/// hoisted argument array and the failure edge are written once.
#[allow(clippy::too_many_arguments)]
fn emit_call_with<'ctx>(
    context: &'ctx Context,
    builder: &Builder<'ctx>,
    module: &inkwell::module::Module<'ctx>,
    rt: &RtFns<'ctx>,
    symbol: &str,
    callable: inkwell::values::PointerValue<'ctx>,
    args: &[Scalar<'ctx>],
    names: &[String],
) -> Scalar<'ctx> {
    let edge = ForeignFailEdge::for_current(builder);
    let entry_fn = edge.function();
    let ptr = context.ptr_type(inkwell::AddressSpace::default());
    let i64_type = context.i64_type();

    let arg_array = emit_packed_array(context, builder, module, entry_fn, args);

    let nargs = i64_type.const_int((args.len() - names.len()) as u64, false);
    let (fn_type, operands): (_, Vec<BasicMetadataValueEnum<'ctx>>) = if names.is_empty() {
        (
            ptr.fn_type(&[ptr.into(), ptr.into(), i64_type.into()], false),
            vec![callable.into(), arg_array.into(), nargs.into()],
        )
    } else {
        let name_array = alloca_in_entry_block(context, builder, entry_fn, names.len());
        for (index, name) in names.iter().enumerate() {
            let text = builder
                .build_global_string_ptr(name, &format!("pycc_foreign_kwname_{name}"))
                .expect("build_global_string_ptr should not fail")
                .as_pointer_value();
            let slot = unsafe {
                builder
                    .build_in_bounds_gep(
                        ptr,
                        name_array,
                        &[i64_type.const_int(index as u64, false)],
                        "foreign_call_kwname_slot",
                    )
                    .expect("build_in_bounds_gep should not fail")
            };
            builder
                .build_store(slot, text)
                .expect("build_store should not fail");
        }
        (
            ptr.fn_type(
                &[
                    ptr.into(),
                    ptr.into(),
                    i64_type.into(),
                    ptr.into(),
                    i64_type.into(),
                ],
                false,
            ),
            vec![
                callable.into(),
                arg_array.into(),
                nargs.into(),
                name_array.into(),
                i64_type.const_int(names.len() as u64, false).into(),
            ],
        )
    };
    let call = shim_fn(module, symbol, fn_type);
    let result = builder
        .build_call(call, &operands, "foreign_call")
        .expect("build_call should not fail for a foreign call helper")
        .try_as_basic_value()
        .expect_basic("a foreign call helper returns PyObject *")
        .into_pointer_value();

    route_null(context, builder, module, rt, edge, result, "foreign_call");
    Scalar::Object(result)
}

/// Emits one `o[k]` load, yielding the result as an opaque
/// [`Scalar::Object`].
///
/// Lives here rather than in a module of its own because it reuses the
/// call path's primitives unchanged -- [`emit_pack`] for the key,
/// [`shim_fn`] for the declaration, [`route_null`] for the failure edge --
/// and because the *packer contract* is the thing that must not fork:
/// `pycc_ext_obj_getitem` consumes the packed key exactly as
/// `pycc_ext_obj_call` consumes a packed argument, so whatever a packer
/// creates is always released by the shim helper it is handed to.
///
/// That is also why no NULL check is emitted on the packed key. A failed
/// packer stores `NULL`, the shim tests for it and propagates the
/// already-set exception, and the operation therefore has exactly *one*
/// failure edge rather than two. The result is a new reference, owned
/// on the rule `foreign_attr.rs` documents for an attribute load.
pub(super) fn emit_subscript<'ctx>(
    context: &'ctx Context,
    builder: &Builder<'ctx>,
    module: &inkwell::module::Module<'ctx>,
    rt: &RtFns<'ctx>,
    base: Scalar<'ctx>,
    index: Scalar<'ctx>,
) -> Scalar<'ctx> {
    let edge = ForeignFailEdge::for_current(builder);
    let base_ptr = expect_object_pointer(base);
    let ptr = context.ptr_type(inkwell::AddressSpace::default());

    let key = emit_pack(context, builder, module, index, "foreign_subscript_key");

    let getitem = shim_fn(
        module,
        EXT_OBJ_GETITEM_SYMBOL,
        ptr.fn_type(&[ptr.into(), ptr.into()], false),
    );
    let result = builder
        .build_call(getitem, &[base_ptr.into(), key.into()], "foreign_subscript")
        .expect("build_call should not fail for pycc_ext_obj_getitem")
        .try_as_basic_value()
        .expect_basic("pycc_ext_obj_getitem returns PyObject *")
        .into_pointer_value();

    route_null(
        context,
        builder,
        module,
        rt,
        edge,
        result,
        "foreign_subscript",
    );
    Scalar::Object(result)
}

#[cfg(test)]
mod tests;
