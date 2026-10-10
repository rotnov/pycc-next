//! Reference counting of heap values: `str`, `list[T]` and instances of
//! the module's classes.
//!
//! **The ownership model** (the same for all three, and the one `str` has
//! had since D-060): every value `emit_expr` produces is either a
//! *borrowed* read of storage that keeps owning it ([`is_borrowed_read`]:
//! a name, an attribute or list element of a borrowed base) or a *fresh*
//! reference the expression was born with (a literal, a display, a call's
//! result, a constructor). Every storage location -- a local or parameter
//! slot, a module global, an instance slot, a list element -- owns one
//! reference to what it holds:
//!
//! - a store of a borrowed value takes its own reference first
//!   ([`retain_if_heap_duplicate`]) and releases the value it overwrites;
//! - a call's arguments are owned by the callee's parameter slots, and a
//!   return value is owned by the caller;
//! - a frame releases its slots on every way out (the owned-slot epilogue);
//! - a consumer that does not store a value releases a fresh one
//!   ([`release_scalar_if_temporary`]): a comparison operand, a condition, a
//!   discarded statement result, an operand a runtime call only borrows.
//!
//! A `str` counts through `pycc_rt_str_incref`/`_decref`. A list or an
//! instance keeps its count at `pycc_rt::RC_OFFSET` in both objects, so the
//! increment and the decrement are inlined here; only a decrement that
//! reaches zero calls the runtime, on a cold path, to free the object and
//! release what it holds. Instances of an exception class are not counted
//! (they are the runtime's leak-only exception objects), nor are dicts and
//! sets (still leak-only).

use super::*;
use inkwell::basic_block::BasicBlock;
use pycc_mir::Ty;

/// `pycc_rt::RC_OFFSET`: the byte offset of a list's or instance's count.
const RC_OFFSET: u64 = 32;
/// The `!tbaa` tag of the count word: no slot, list field or element
/// access aliases it.
const RC_TBAA: &str = "pycc rc";

/// A reference-counted kind of value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum HeapKind {
    Str,
    List,
    Instance,
}

/// Every reference-counted class of `class_defs` with its slot kind string
/// (see `RtFns::rc_class_kinds`).
pub(super) fn rc_class_kinds(
    class_defs: &[(String, pycc_hir::HirClassDef)],
) -> HashMap<String, String> {
    let by_name: HashMap<&str, &pycc_hir::HirClassDef> = class_defs
        .iter()
        .map(|(name, def)| (name.as_str(), def))
        .collect();
    let refcounted = |name: &str| {
        by_name
            .get(name)
            .is_some_and(|def| pycc_hir::is_refcounted_class(def))
    };
    class_defs
        .iter()
        .filter(|(_, def)| pycc_hir::is_refcounted_class(def))
        .map(|(name, def)| {
            let mro: Vec<&pycc_hir::HirClassDef> = def
                .mro
                .iter()
                .filter_map(|entry| by_name.get(entry.as_str()).copied())
                .collect();
            let kinds: String = pycc_hir::flat_attr_layout(&mro)
                .iter()
                .map(|(_, ty)| pycc_hir::slot_rc_kind(ty, &refcounted) as char)
                .collect();
            (name.clone(), kinds)
        })
        .collect()
}

/// The counted kind of a value of type `ty`, if it is counted.
pub(crate) fn heap_kind(rt: &RtFns<'_>, ty: &Ty) -> Option<HeapKind> {
    match ty {
        Ty::Str => Some(HeapKind::Str),
        Ty::List(_) => Some(HeapKind::List),
        Ty::Instance(class) if rt.rc_class_kinds.borrow().contains_key(class.as_str()) => {
            Some(HeapKind::Instance)
        }
        _ => None,
    }
}

/// Whether `expr` reads storage that keeps owning the value, rather than
/// producing a reference of its own. A read through a *fresh* base (an
/// attribute of a call's result, an element of a new list) is not
/// borrowed: the base is released once the read is made, so the read takes
/// its own reference first (`emit_expr`'s `AttrGet`/`Subscript` arms).
///
/// A *pinned* operand (`operand_pin`) is not borrowed: `emit_expr` took a
/// reference to it right after reading it.
pub(crate) fn is_borrowed_read(expr: &MirExpr) -> bool {
    !crate::operand_pin::is_pinned(expr) && reads_storage(expr)
}

/// [`is_borrowed_read`] before `expr`'s own pin: whether evaluating it
/// yields a value its storage keeps owning.
fn reads_storage(expr: &MirExpr) -> bool {
    match expr {
        MirExpr::Sequence { value, .. } => is_borrowed_read(value),
        MirExpr::Name { .. } | MirExpr::NamedExpr { .. } | MirExpr::ExceptionMessage(_) => true,
        MirExpr::AttrGet { base, .. } | MirExpr::Subscript { base, .. } => is_borrowed_read(base),
        _ => false,
    }
}

/// Takes the reference a pinned operand holds (`operand_pin`): called by
/// `emit_expr` once `expr` is evaluated and its exception guard passed.
/// A pinned read whose base was itself pinned is already fresh (the read
/// took its own reference before releasing the base), so it takes none.
pub(crate) fn retain_if_pinned<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    rt: &RtFns<'ctx>,
    expr: &MirExpr,
    scalar: &Scalar<'ctx>,
) {
    if crate::operand_pin::is_pinned(expr)
        && reads_storage(expr)
        && let Some((kind, ptr)) = counted(rt, &expr.ty(), scalar)
    {
        emit_incref(context, builder, rt, kind, ptr);
    }
}

/// The counted kind and pointer of `scalar`, the value of an expression of
/// type `ty`.
fn counted<'ctx>(
    rt: &RtFns<'ctx>,
    ty: &Ty,
    scalar: &Scalar<'ctx>,
) -> Option<(HeapKind, PointerValue<'ctx>)> {
    match (heap_kind(rt, ty)?, scalar) {
        (HeapKind::Str, Scalar::Str(ptr)) => Some((HeapKind::Str, *ptr)),
        (HeapKind::List, Scalar::List(ptr)) => Some((HeapKind::List, *ptr)),
        (HeapKind::Instance, Scalar::Instance(ptr)) => Some((HeapKind::Instance, *ptr)),
        _ => None,
    }
}

fn current_function<'ctx>(builder: &inkwell::builder::Builder<'ctx>) -> FunctionValue<'ctx> {
    builder
        .get_insert_block()
        .and_then(|block| block.get_parent())
        .expect("reference counting is emitted inside a function body")
}

/// Branches to a new block when `ptr` is non-null and returns it with the
/// join block both paths reach.
fn non_null<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    ptr: PointerValue<'ctx>,
    name: &str,
) -> (BasicBlock<'ctx>, BasicBlock<'ctx>) {
    let function = current_function(builder);
    let live = context.append_basic_block(function, &format!("{name}_live"));
    let done = context.append_basic_block(function, &format!("{name}_done"));
    let is_live = builder
        .build_is_not_null(ptr, &format!("{name}_nonnull"))
        .expect("build_is_not_null should not fail for a pointer");
    crate::int_fast::likely_branch(context, builder, is_live, live, done);
    builder.position_at_end(live);
    (live, done)
}

/// The count word of the list or instance at `ptr`.
fn count_ptr<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    ptr: PointerValue<'ctx>,
) -> PointerValue<'ctx> {
    // SAFETY (LLVM): the count sits inside every list and instance.
    unsafe {
        builder
            .build_in_bounds_gep(
                context.i8_type(),
                ptr,
                &[context.i64_type().const_int(RC_OFFSET, false)],
                "rc_ptr",
            )
            .expect("build_in_bounds_gep should not fail for a constant offset")
    }
}

fn tagged_load<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    ptr: PointerValue<'ctx>,
) -> IntValue<'ctx> {
    let value = builder
        .build_load(context.i64_type(), ptr, "rc")
        .expect("build_load should not fail for the count word")
        .into_int_value();
    crate::tbaa::tag(
        context,
        value.as_instruction().expect("a load is an instruction"),
        RC_TBAA,
    );
    value
}

fn tagged_store<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    ptr: PointerValue<'ctx>,
    value: IntValue<'ctx>,
) {
    let store = builder
        .build_store(ptr, value)
        .expect("build_store should not fail for the count word");
    crate::tbaa::tag(context, store, RC_TBAA);
}

/// Takes one more reference to the `kind` value at `ptr` (null is a no-op).
pub(crate) fn emit_incref<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    rt: &RtFns<'ctx>,
    kind: HeapKind,
    ptr: PointerValue<'ctx>,
) {
    if kind == HeapKind::Str {
        builder
            .build_call(rt.str_incref, &[ptr.into()], "str_incref")
            .expect("build_call should not fail for a well-formed incref");
        return;
    }
    let (_, done) = non_null(context, builder, ptr, "rc_inc");
    bump(context, builder, ptr);
    builder
        .build_unconditional_branch(done)
        .expect("build_unconditional_branch should not fail");
    builder.position_at_end(done);
}

fn bump<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    ptr: PointerValue<'ctx>,
) {
    let cell = count_ptr(context, builder, ptr);
    let count = tagged_load(context, builder, cell);
    let bumped = builder
        .build_int_add(count, context.i64_type().const_int(1, false), "rc_inc")
        .expect("build_int_add should not fail for two i64 operands");
    tagged_store(context, builder, cell, bumped);
}

/// Drops one reference to the `kind` value at `ptr`, freeing it at zero
/// (null is a no-op).
pub(crate) fn emit_decref<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    rt: &RtFns<'ctx>,
    kind: HeapKind,
    ptr: PointerValue<'ctx>,
) {
    let free = match kind {
        HeapKind::Str => {
            builder
                .build_call(rt.str_decref, &[ptr.into()], "str_decref")
                .expect("build_call should not fail for a well-formed decref");
            return;
        }
        HeapKind::List => rt.list_free,
        HeapKind::Instance => rt.instance_free,
    };
    let (_, done) = non_null(context, builder, ptr, "rc_dec");
    let cell = count_ptr(context, builder, ptr);
    let count = tagged_load(context, builder, cell);
    let dropped = builder
        .build_int_sub(count, context.i64_type().const_int(1, false), "rc_dec")
        .expect("build_int_sub should not fail for two i64 operands");
    tagged_store(context, builder, cell, dropped);
    let alive = builder
        .build_int_compare(
            inkwell::IntPredicate::NE,
            dropped,
            context.i64_type().const_zero(),
            "rc_alive",
        )
        .expect("build_int_compare should not fail for two i64 operands");
    let free_bb = context.append_basic_block(current_function(builder), "rc_free");
    crate::int_fast::likely_branch(context, builder, alive, done, free_bb);
    builder.position_at_end(free_bb);
    builder
        .build_call(free, &[ptr.into()], "rc_free")
        .expect("build_call should not fail for a free");
    builder
        .build_unconditional_branch(done)
        .expect("build_unconditional_branch should not fail");
    builder.position_at_end(done);
}

/// Takes a reference to `scalar` (the value of `source`) when `source` is
/// a borrowed read of a counted value, so the caller can store it as an
/// owned reference. Fresh values pass through: their birth reference is
/// the one being stored.
pub(crate) fn retain_if_heap_duplicate<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    rt: &RtFns<'ctx>,
    source: &MirExpr,
    scalar: Scalar<'ctx>,
) -> Scalar<'ctx> {
    if is_borrowed_read(source)
        && let Some((kind, ptr)) = counted(rt, &source.ty(), &scalar)
    {
        // Null-checked even for an element or slot read: a value that
        // reached here through a raising path must never be dereferenced.
        emit_incref(context, builder, rt, kind, ptr);
    }
    scalar
}

/// Takes a reference to `scalar`, a value of type `ty` read out of a list
/// element or an instance slot, unconditionally: a counted value is
/// incremented (null-checked) and a heap bigint retained.
pub(crate) fn retain_scalar<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    rt: &RtFns<'ctx>,
    ty: &Ty,
    scalar: &Scalar<'ctx>,
) {
    if let Some((kind, ptr)) = counted(rt, ty, scalar) {
        emit_incref(context, builder, rt, kind, ptr);
    } else if let (Ty::Int, Scalar::Int(word)) = (ty, scalar) {
        crate::bigint_rc::emit_bigint_refcount_call(
            context,
            builder,
            rt,
            *word,
            crate::bigint_rc::BigIntRefcount::Retain,
        );
    }
}

/// Releases `scalar` (the value of `source`) when it is a fresh reference
/// the consumer at hand does not keep: an `int` temporary (a heap bigint,
/// D-181) or a counted value that is not a borrowed read. Every other kind
/// owns no reference here and is left alone.
pub(super) fn release_scalar_if_temporary<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    rt: &RtFns<'ctx>,
    source: &MirExpr,
    scalar: &Scalar<'ctx>,
) {
    if let Scalar::Int(_) = scalar {
        release_scalar_if_int_temporary(context, builder, rt, source, scalar);
    } else if !is_borrowed_read(source)
        && let Some((kind, ptr)) = counted(rt, &source.ty(), scalar)
    {
        emit_decref(context, builder, rt, kind, ptr);
    }
}

/// Releases the counted value a slot of type `ty` at `slot_ptr` holds (a
/// no-op for an uncounted type, and for a null slot).
pub(crate) fn release_slot<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    rt: &RtFns<'ctx>,
    ty: &Ty,
    slot_ptr: PointerValue<'ctx>,
) {
    let Some(kind) = heap_kind(rt, ty) else {
        return;
    };
    let old = builder
        .build_load(
            context.ptr_type(inkwell::AddressSpace::default()),
            slot_ptr,
            "old_heap",
        )
        .expect("build_load should not fail for a slot this function allocated")
        .into_pointer_value();
    emit_decref(context, builder, rt, kind, old);
}

/// Releases the value an instance slot of the counted kind of `ty` holds
/// before a store overwrites it. An unassigned slot holds the `0` word,
/// which releases nothing.
pub(crate) fn release_attr_slot<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    rt: &RtFns<'ctx>,
    ty: &Ty,
    base: PointerValue<'ctx>,
    slot: u64,
) {
    let Some(kind) = heap_kind(rt, ty) else {
        return;
    };
    let raw = attr_slot::emit_slot_load(context, builder, base, slot);
    let old = builder
        .build_int_to_ptr(
            raw,
            context.ptr_type(inkwell::AddressSpace::default()),
            "old_attr",
        )
        .expect("build_int_to_ptr should not fail reinterpreting an i64 as a pointer");
    emit_decref(context, builder, rt, kind, old);
}

#[cfg(test)]
#[path = "heap_rc_tests.rs"]
mod tests;
