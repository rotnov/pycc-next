//! A list or set comprehension over a CPython object (Part 1 of #1255):
//! `[elt for x in <object> if cond]` and `{elt for x in <object> if cond}`,
//! producing a fresh CPython `list` or `set` (`Ty::Object`, D-258).
//!
//! # Shape
//!
//! The iterable is evaluated once in the enclosing scope, then iterated
//! through the same two halves `for x in <object>:` uses
//! ([`foreign_call::emit_get_iter`] and [`foreign_call::emit_iter_header`]).
//! The result collection is created *after* `iter()` succeeds, as CPython's
//! own comprehension does, so a non-iterable source raises before anything
//! else happens. Each element is packed across the boundary and handed to
//! [`EXT_OBJ_COLLECT_SYMBOL`], which consumes it on every path.
//!
//! Every failure -- `iter()`, `next()`, the result constructor, the
//! condition's truth test, the element's own evaluation, packing, and the
//! insertion itself (an unhashable set item) -- takes the current function's
//! foreign failure edge ([`ForeignFailEdge::for_current`]): the
//! `EXT_MODULE_EXEC_FAILED` return in the module body outside every
//! module-level `try`, and otherwise the error bridge plus the enclosing
//! handler (Part 1 of #1096).
//!
//! # Ownership
//!
//! A produced source (`[e for e in s.items]`) is released right after
//! `iter()` has taken its own reference (Part 1 of #1092). The iterator and
//! the result collection are held on `pending_object_releases` from the
//! moment each exists (Part 3 of #1092), so every failure edge below
//! releases both; the normal exit releases the iterator and hands the
//! result on as the node's value, which `object_release::is_produced`
//! lists as a producer, so an unbound result is released by its consumer.
//! Each item is a new reference the loop variable's slot owns, exactly as
//! `MirStmt::ForObject`'s target: the next trip releases it, and the loop's
//! exit releases the last one.
//! A pycc `int` temporary built by the condition or the element is released
//! once it has been tested or packed, since the packers borrow their operand,
//! and so is a produced CPython object condition or element (`if x.ok()`,
//! `x.a for ...`, Part 1 of #1092): the condition is held across its truth
//! test, and the element is released once the packer has taken a reference
//! of its own.

use super::comprehension::{CompCx, CompElts};
use super::foreign_call;
use super::foreign_fail::{ForeignFailEdge, route_negative, route_null};
use super::foreign_pack::{emit_pack, shim_fn};
use super::release_scalar_if_temporary;
use super::{EXT_OBJ_COLLECT_SYMBOL, EXT_OBJ_NEW_COLLECTION_SYMBOL, ObjCollectionKind};
use super::{Scalar, emit_expr, truthy};
use inkwell::values::PointerValue;
use pycc_mir::MirExpr;

/// Emits the comprehension and returns its result, a `Scalar::Object`.
/// `var_ptr` is the loop variable's pointer slot, already allocated and
/// visible in `cx.locals`; `iterable` is the source expression.
pub(super) fn emit_object_comprehension<'ctx>(
    cx: &CompCx<'_, 'ctx>,
    var_ptr: PointerValue<'ctx>,
    iterable: &MirExpr,
    cond: Option<&MirExpr>,
    elts: CompElts<'_>,
) -> Scalar<'ctx> {
    let (context, builder, module, rt) = (cx.context, cx.builder, cx.module, cx.rt);
    let (kind, elt) = match elts {
        CompElts::List(elt) => (ObjCollectionKind::List, elt),
        CompElts::Set(elt, None) => (ObjCollectionKind::Set, elt),
        CompElts::Objects(elt, kind) => (kind, elt),
        CompElts::Set(_, Some(_)) | CompElts::Dict(..) => panic!(
            "pycc_codegen: internal error: only a list or set comprehension of packable \
             elements may iterate a CPython object -- pycc_types::check (I0404) should have \
             rejected this before codegen"
        ),
    };
    let emit = |expr: &MirExpr| {
        emit_expr(
            context,
            builder,
            module,
            rt,
            cx.user_functions,
            cx.locals,
            expr,
        )
    };
    let ptr = context.ptr_type(inkwell::AddressSpace::default());
    let i64_ty = context.i64_type();

    let source = emit(iterable);
    let held_source = crate::object_release::hold(context, module, rt, iterable, &source);
    let iterator = foreign_call::emit_get_iter(context, builder, module, rt, source);
    held_source.release(builder, rt);
    // Part 3 of #1092: the iterator and, once it exists, the result are
    // held for the comprehension's whole extent -- an expression, so no
    // `try` can catch inside it -- and every failure edge below releases
    // both.
    let held_iterator = crate::object_release::hold_new_reference(context, module, rt, iterator);

    let edge = ForeignFailEdge::for_current(builder);
    let new_collection = shim_fn(
        module,
        EXT_OBJ_NEW_COLLECTION_SYMBOL,
        ptr.fn_type(&[i64_ty.into()], false),
    );
    let kind_code = i64_ty.const_int(kind.shim_code(), false);
    let collection = builder
        .build_call(new_collection, &[kind_code.into()], "objcomp_result")
        .expect("build_call should not fail for pycc_ext_obj_new_collection")
        .try_as_basic_value()
        .expect_basic("pycc_ext_obj_new_collection returns PyObject *")
        .into_pointer_value();
    route_null(
        context,
        builder,
        module,
        rt,
        edge,
        collection,
        "objcomp_result",
    );
    let held_collection =
        crate::object_release::hold_new_reference(context, module, rt, collection);

    let lp = foreign_call::emit_iter_header(context, builder, module, rt, iterator);
    // The loop variable's slot owns the item and releases the previous one
    // (`heap_rc`).
    let old = crate::heap_rc::take_slot_old(context, builder, rt, &pycc_mir::Ty::Object, var_ptr);
    builder
        .build_store(var_ptr, lp.item)
        .expect("build_store should not fail for the loop variable's own slot");
    crate::heap_rc::release_old(context, builder, rt, old);

    if let Some(cond) = cond {
        let scalar = emit(cond);
        let held = crate::object_release::hold(context, module, rt, cond, &scalar);
        let test = truthy(context, builder, module, rt, scalar);
        held.release(builder, rt);
        release_scalar_if_temporary(context, builder, rt, cond, &scalar);
        let function = edge.function();
        let keep_bb = context.append_basic_block(function, "objcomp_keep");
        builder
            .build_conditional_branch(test, keep_bb, lp.header_bb)
            .expect("build_conditional_branch should not fail for an i1 condition");
        builder.position_at_end(keep_bb);
    }

    let scalar = crate::object_unbox::emit_pack_operand(
        context,
        builder,
        module,
        rt,
        cx.user_functions,
        cx.locals,
        elt,
    );
    let item = emit_pack(context, builder, module, scalar, "objcomp_item");
    release_scalar_if_temporary(context, builder, rt, elt, &scalar);
    crate::object_release::release_if_produced(context, builder, module, elt, &scalar);
    let collect = shim_fn(
        module,
        EXT_OBJ_COLLECT_SYMBOL,
        context
            .i32_type()
            .fn_type(&[ptr.into(), i64_ty.into(), ptr.into()], false),
    );
    let status = builder
        .build_call(
            collect,
            &[collection.into(), kind_code.into(), item.into()],
            "objcomp_collect",
        )
        .expect("build_call should not fail for pycc_ext_obj_collect")
        .try_as_basic_value()
        .expect_basic("pycc_ext_obj_collect returns int")
        .into_int_value();
    route_negative(
        context,
        builder,
        module,
        rt,
        edge,
        status,
        "objcomp_collect",
    );
    builder
        .build_unconditional_branch(lp.header_bb)
        .expect("build_unconditional_branch should not fail closing the loop body");

    builder.position_at_end(lp.after_bb);
    // The last item is released and the slot cleared, so the next run of
    // this comprehension starts from a null slot. An exception out of the
    // loop leaves the item in the slot, which the next run's first store
    // releases.
    let last = crate::heap_rc::take_slot_old(context, builder, rt, &pycc_mir::Ty::Object, var_ptr);
    builder
        .build_store(var_ptr, ptr.const_null())
        .expect("build_store should not fail for the loop variable's own slot");
    crate::heap_rc::release_old(context, builder, rt, last);
    held_iterator.release(builder, rt);
    // The result is the comprehension's value: a producer
    // (`object_release::is_produced`), which its consumer releases or
    // binds.
    held_collection.consumed(rt);
    Scalar::Object(collection)
}

/// Creates the CPython collection a comprehension over a native source
/// collects objects into (`MirComprehension::collects_objects`), held on
/// `pending_object_releases` until [`super::object_release::Held::consumed`].
pub(super) fn new_object_collection<'ctx>(
    cx: &CompCx<'_, 'ctx>,
    kind: ObjCollectionKind,
) -> (PointerValue<'ctx>, crate::object_release::Held<'ctx>) {
    let (context, builder, module, rt) = (cx.context, cx.builder, cx.module, cx.rt);
    let ptr = context.ptr_type(inkwell::AddressSpace::default());
    let i64_ty = context.i64_type();
    let edge = ForeignFailEdge::for_current(builder);
    let new_collection = shim_fn(
        module,
        EXT_OBJ_NEW_COLLECTION_SYMBOL,
        ptr.fn_type(&[i64_ty.into()], false),
    );
    let kind_code = i64_ty.const_int(kind.shim_code(), false);
    let collection = builder
        .build_call(new_collection, &[kind_code.into()], "objcomp_result")
        .expect("build_call should not fail for pycc_ext_obj_new_collection")
        .try_as_basic_value()
        .expect_basic("pycc_ext_obj_new_collection returns PyObject *")
        .into_pointer_value();
    route_null(
        context,
        builder,
        module,
        rt,
        edge,
        collection,
        "objcomp_result",
    );
    let held = crate::object_release::hold_new_reference(context, module, rt, collection);
    (collection, held)
}

/// Packs `elt` and adds it to `collection` (`pycc_ext_obj_collect`, which
/// consumes the packed element), raising through the current function's
/// foreign failure edge.
pub(super) fn collect_object<'ctx>(
    cx: &CompCx<'_, 'ctx>,
    collection: PointerValue<'ctx>,
    kind: ObjCollectionKind,
    elt: &MirExpr,
) {
    let (context, builder, module, rt) = (cx.context, cx.builder, cx.module, cx.rt);
    let ptr = context.ptr_type(inkwell::AddressSpace::default());
    let i64_ty = context.i64_type();
    let edge = ForeignFailEdge::for_current(builder);
    let kind_code = i64_ty.const_int(kind.shim_code(), false);
    let scalar = crate::object_unbox::emit_pack_operand(
        context,
        builder,
        module,
        rt,
        cx.user_functions,
        cx.locals,
        elt,
    );
    let item = emit_pack(context, builder, module, scalar, "objcomp_item");
    release_scalar_if_temporary(context, builder, rt, elt, &scalar);
    crate::object_release::release_if_produced(context, builder, module, elt, &scalar);
    let collect = shim_fn(
        module,
        EXT_OBJ_COLLECT_SYMBOL,
        context
            .i32_type()
            .fn_type(&[ptr.into(), i64_ty.into(), ptr.into()], false),
    );
    let status = builder
        .build_call(
            collect,
            &[collection.into(), kind_code.into(), item.into()],
            "objcomp_collect",
        )
        .expect("build_call should not fail for pycc_ext_obj_collect")
        .try_as_basic_value()
        .expect_basic("pycc_ext_obj_collect returns int")
        .into_int_value();
    route_negative(
        context,
        builder,
        module,
        rt,
        edge,
        status,
        "objcomp_collect",
    );
}
