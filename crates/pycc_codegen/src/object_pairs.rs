//! A comprehension over a CPython object that unpacks its item into two
//! names (`[f(k, v) for k, v in src]`, docs/design/compile-little.md,
//! section 2.6).
//!
//! The desugaring rewrites the tuple target to one variable whose every read
//! is an item of it (`pycc_ast::is_unpack_target`; `_pycc_unpack_2_N[0]`,
//! `[1]`). Here the variable is never bound: each step of the loop binds the
//! two halves straight into two slots of their own, and each of those reads
//! loads its slot ([`pair_read`]). A step is one call of
//! `pycc_ext_obj_pair_next` (`src/ext/module/object_pairs.c`), which unpacks
//! an iterator's item as CPython's `UNPACK_SEQUENCE` does, with its messages,
//! and releases the item at once, as CPython drops it once unpacked.
//!
//! Over `d.items()` of an exact `dict` `d` (`PyccExtFast`'s type, so never
//! with the fast paths off) no `items()` call, view, iterator or pair tuple is
//! made: the step reads the dict's entries with `PyDict_Next`, raising what
//! the dict's items iterator raises when the dict changes size or keys while
//! the loop runs. Every other receiver calls `items()` as before.
//!
//! Ownership follows [`crate::object_comprehension`]: the two slots are held
//! below the result and the iterator (an exact dict stands for the iterator,
//! with a reference of its own), so a failure edge releases the iterator, the
//! result, then the first half and the second; the normal exit releases the
//! iterator, then the two halves.

use super::comprehension::CompCx;
use super::foreign_fail::{ForeignFailEdge, emit_failure, route_null};
use super::foreign_pack::shim_fn;
use super::{EXT_OBJ_NEW_COLLECTION_SYMBOL, ObjCollectionKind, Scalar, StorageSlot, emit_expr, truthy};
use super::release_scalar_if_temporary;
use inkwell::values::PointerValue;
use pycc_mir::{MirExpr, Ty};
use std::collections::HashMap;

/// `long long pycc_ext_obj_pair_next(PyObject *it, long long *state,
/// PyObject **out)`: one step, 1 with both halves in `out`, 0 at the end,
/// -1 with the exception set.
const EXT_OBJ_PAIR_NEXT_SYMBOL: &str = "pycc_ext_obj_pair_next";

/// `state[0]` for an exact dict not started yet, and for an iterator.
const STATE_DICT: u64 = 2;
const STATE_ITERATOR: u64 = 0;

/// The `locals` key of half `index` of the unpacked variable `var`: a name no
/// program binds.
fn half_key(var: &str, index: i64) -> String {
    format!("{var}\u{0}{index}")
}

/// Whether the comprehension over variable `var` (which unpacked its item:
/// `unpacks`) is lowered here: its target had two names and every read of
/// `var` in `exprs` is one of its two items.
pub(super) fn eligible(var: &str, unpacks: bool, exprs: &[&MirExpr]) -> bool {
    unpacks
        && pycc_hir::unpack_target_arity(var) == Some(2)
        && exprs.iter().all(|expr| reads_only_halves(var, expr))
}

fn reads_only_halves(var: &str, expr: &MirExpr) -> bool {
    match expr {
        MirExpr::ObjSubscript { base, index }
            if matches!(base.as_ref(), MirExpr::Name { name, .. } if name == var)
                && matches!(index.as_ref(), MirExpr::IntLiteral(0 | 1)) =>
        {
            true
        }
        MirExpr::Name { name, .. } => name != var,
        other => other.children().into_iter().all(|child| reads_only_halves(var, child)),
    }
}

/// `var[0]` or `var[1]` of a variable this module bound by halves: a new
/// reference to the half, as the subscript would have returned.
pub(super) fn pair_read<'ctx>(
    context: &'ctx inkwell::context::Context,
    builder: &inkwell::builder::Builder<'ctx>,
    rt: &crate::RtFns<'ctx>,
    locals: &HashMap<String, StorageSlot<'ctx>>,
    base: &MirExpr,
    index: &MirExpr,
) -> Option<Scalar<'ctx>> {
    let (MirExpr::Name { name, .. }, MirExpr::IntLiteral(at @ (0 | 1))) = (base, index) else {
        return None;
    };
    let slot = locals.get(&half_key(name, *at))?;
    let half = builder
        .build_load(context.ptr_type(inkwell::AddressSpace::default()), slot.ptr, "pair_half")
        .expect("build_load should not fail for a slot this function allocated")
        .into_pointer_value();
    let half = Scalar::Object(half);
    crate::heap_rc::retain_scalar(context, builder, rt, &Ty::Object, &half);
    Some(half)
}

/// Emits the comprehension: `var` is its variable, `iterable` its source.
/// `kind` and `item` are what [`crate::object_comprehension`] collects.
pub(super) fn emit<'ctx>(
    cx: &CompCx<'_, 'ctx>,
    var: &str,
    iterable: &MirExpr,
    cond: Option<&MirExpr>,
    kind: ObjCollectionKind,
    item: Result<&MirExpr, (&MirExpr, &MirExpr)>,
) -> Scalar<'ctx> {
    let (context, builder, module, rt) = (cx.context, cx.builder, cx.module, cx.rt);
    let ptr = context.ptr_type(inkwell::AddressSpace::default());
    let i64_ty = context.i64_type();
    let function = ForeignFailEdge::for_current(builder).function();
    let (halves, state, out) = crate::entry_slots::build_at_entry_block(builder, function, |b| {
        let halves = b
            .build_array_alloca(ptr, i64_ty.const_int(2, false), "pair_halves")
            .expect("build_array_alloca should not fail");
        let state = b
            .build_array_alloca(i64_ty, i64_ty.const_int(4, false), "pair_state")
            .expect("build_array_alloca should not fail");
        let out = b
            .build_array_alloca(ptr, i64_ty.const_int(2, false), "pair_out")
            .expect("build_array_alloca should not fail");
        (halves, state, out)
    });
    let at = |base: PointerValue<'ctx>, ty: inkwell::types::BasicTypeEnum<'ctx>, index: u64| {
        // SAFETY: an index below the length of this function's own array.
        unsafe {
            builder
                .build_in_bounds_gep(ty, base, &[i64_ty.const_int(index, false)], "pair_at")
                .expect("build_in_bounds_gep should not fail")
        }
    };
    let slots = [at(halves, ptr.into(), 0), at(halves, ptr.into(), 1)];
    for slot in slots {
        builder
            .build_store(slot, ptr.const_null())
            .expect("build_store should not fail for this function's own slot");
    }
    let mut scoped = cx.locals.clone();
    for (index, slot) in slots.iter().enumerate() {
        scoped.insert(
            half_key(var, index as i64),
            StorageSlot {
                ptr: *slot,
                ty: Ty::Object,
                initialized: None,
            },
        );
    }
    let inner = CompCx {
        locals: &scoped,
        ..*cx
    };
    let emit = |expr: &MirExpr| {
        emit_expr(context, builder, module, rt, cx.user_functions, &scoped, expr)
    };

    // The halves are held below the result and the iterator; a failure edge
    // releases the first, then the second.
    let held_second = crate::object_release::hold_slot(context, module, rt, slots[1]);
    let held_first = crate::object_release::hold_slot(context, module, rt, slots[0]);
    let state_kind = at(state, i64_ty.into(), 0);
    let iterator = emit_source(cx, &scoped, iterable, state_kind);
    let held_iterator = crate::object_release::hold_new_reference(context, module, rt, iterator);

    let edge = ForeignFailEdge::for_current(builder);
    let new_collection = shim_fn(module, EXT_OBJ_NEW_COLLECTION_SYMBOL, ptr.fn_type(&[i64_ty.into()], false));
    let collection = builder
        .build_call(new_collection, &[i64_ty.const_int(kind.shim_code(), false).into()], "objcomp_result")
        .expect("build_call should not fail for pycc_ext_obj_new_collection")
        .try_as_basic_value()
        .expect_basic("pycc_ext_obj_new_collection returns PyObject *")
        .into_pointer_value();
    route_null(context, builder, module, rt, edge, collection, "objcomp_result");
    held_iterator.consumed(rt);
    let held_collection = crate::object_release::hold_new_reference(context, module, rt, collection);
    let held_iterator = crate::object_release::hold_new_reference(context, module, rt, iterator);

    // The loop: one step per trip, the halves stored into their slots, each
    // releasing what it held.
    let header_bb = context.append_basic_block(function, "pair_header");
    let body_bb = context.append_basic_block(function, "pair_body");
    let after_bb = context.append_basic_block(function, "pair_after");
    let fail_bb = context.append_basic_block(function, "pair_next_fail");
    builder.build_unconditional_branch(header_bb).expect("branch");
    builder.position_at_end(header_bb);
    let step = shim_fn(
        module,
        EXT_OBJ_PAIR_NEXT_SYMBOL,
        i64_ty.fn_type(&[ptr.into(), ptr.into(), ptr.into()], false),
    );
    let status = builder
        .build_call(step, &[iterator.into(), state.into(), out.into()], "pair_status")
        .expect("build_call should not fail for pycc_ext_obj_pair_next")
        .try_as_basic_value()
        .expect_basic("pycc_ext_obj_pair_next returns long long")
        .into_int_value();
    builder
        .build_switch(
            status,
            fail_bb,
            &[(i64_ty.const_int(1, false), body_bb), (i64_ty.const_zero(), after_bb)],
        )
        .expect("build_switch should not fail for an i64 selector");
    builder.position_at_end(fail_bb);
    emit_failure(context, builder, module, rt, edge);
    builder.position_at_end(body_bb);
    for (index, slot) in slots.iter().enumerate() {
        let half = builder
            .build_load(ptr, at(out, ptr.into(), index as u64), "pair_new_half")
            .expect("build_load should not fail for this function's own slot");
        let old = crate::heap_rc::take_slot_old(context, builder, rt, &Ty::Object, *slot);
        builder
            .build_store(*slot, half)
            .expect("build_store should not fail for this function's own slot");
        crate::heap_rc::release_old(context, builder, rt, old);
    }

    if let Some(cond) = cond {
        let scalar = emit(cond);
        let held = crate::object_release::hold(context, module, rt, cond, &scalar);
        let test = truthy(context, builder, module, rt, scalar);
        held.release(builder, rt);
        release_scalar_if_temporary(context, builder, rt, cond, &scalar);
        let keep_bb = context.append_basic_block(function, "pair_keep");
        builder
            .build_conditional_branch(test, keep_bb, header_bb)
            .expect("build_conditional_branch should not fail for an i1 condition");
        builder.position_at_end(keep_bb);
    }
    match item {
        Ok(elt) => super::object_comprehension::collect_object(&inner, collection, kind, elt),
        Err((key, value)) => super::object_comprehension::collect_object_pair(&inner, collection, key, value),
    }
    builder.build_unconditional_branch(header_bb).expect("branch");

    builder.position_at_end(after_bb);
    held_iterator.release(builder, rt);
    for slot in slots {
        let old = crate::heap_rc::take_slot_old(context, builder, rt, &Ty::Object, slot);
        builder
            .build_store(slot, ptr.const_null())
            .expect("build_store should not fail for this function's own slot");
        crate::heap_rc::release_old(context, builder, rt, old);
    }
    held_collection.consumed(rt);
    held_first.consumed(rt);
    held_second.consumed(rt);
    Scalar::Object(collection)
}

/// Evaluates the source and answers what the loop steps over, a new
/// reference: `iter(iterable)`, or, for `d.items()` of an exact dict, the
/// dict itself. Records which in `state_kind`.
fn emit_source<'ctx>(
    cx: &CompCx<'_, 'ctx>,
    locals: &HashMap<String, StorageSlot<'ctx>>,
    iterable: &MirExpr,
    state_kind: PointerValue<'ctx>,
) -> PointerValue<'ctx> {
    let (context, builder, module, rt) = (cx.context, cx.builder, cx.module, cx.rt);
    let i64_ty = context.i64_type();
    let emit = |expr: &MirExpr| emit_expr(context, builder, module, rt, cx.user_functions, locals, expr);
    let store_kind = |kind: u64| {
        builder
            .build_store(state_kind, i64_ty.const_int(kind, false))
            .expect("build_store should not fail for this function's own slot");
    };
    let iterate = |source: Scalar<'ctx>| {
        let iterator = super::foreign_call::emit_get_iter(context, builder, module, rt, source);
        store_kind(STATE_ITERATOR);
        iterator
    };
    if let MirExpr::ObjMethodCall { base, method, args } = iterable
        && method == "items"
        && args.is_empty()
    {
        // An `items` of this module's own classes is called directly off
        // the fast path; an exact dict's is never one of them.
        let direct = crate::direct_call::method_target(module, cx.user_functions, method, 0, &[]);
        let receiver = emit(base);
        let held_receiver = crate::object_release::hold(context, module, rt, base, &receiver);
        let receiver_ptr = crate::foreign_attr::expect_object_pointer(receiver);
        let iterator = crate::fast_ops::guarded(
            context,
            builder,
            module,
            "pair_dict",
            |fast| {
                fast.require_class_in(receiver_ptr, &[crate::fast_ops::DICT], "pair_dict");
                crate::heap_rc::retain_scalar(context, builder, rt, &Ty::Object, &Scalar::Object(receiver_ptr));
                store_kind(STATE_DICT);
                receiver_ptr.into()
            },
            || {
                let view = match &direct {
                    None => super::foreign_method_call::emit_method_call_fused(
                        context,
                        builder,
                        module,
                        rt,
                        receiver,
                        method,
                        &[],
                    ),
                    Some(target) => {
                        let (callable, bound, site) =
                            super::foreign_method_call::emit_load_method(context, builder, module, rt, receiver, method);
                        super::foreign_method_call::emit_call_loaded(
                            context,
                            builder,
                            module,
                            rt,
                            callable,
                            bound,
                            site,
                            &[],
                            &[],
                            Some(target),
                        )
                    }
                };
                let view_ptr = crate::foreign_attr::expect_object_pointer(view);
                let held_view = crate::object_release::hold_new_reference(context, module, rt, view_ptr);
                let iterator = iterate(view);
                held_view.release(builder, rt);
                iterator.into()
            },
        );
        held_receiver.release(builder, rt);
        return iterator.into_pointer_value();
    }
    let source = emit(iterable);
    let held_source = crate::object_release::hold(context, module, rt, iterable, &source);
    let iterator = iterate(source);
    held_source.release(builder, rt);
    iterator
}
