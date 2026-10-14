//! Reference counting of heap values: `str`, `list[T]`, `dict[str, int]`,
//! `set[T]`/`frozenset[T]`, instances of the module's classes, and CPython
//! `object` values (`PyObject *`).
//!
//! **The ownership model** (the same for all of them, and the one `str` has
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
//! A `str`, a dict and a set count through their runtime `_incref`/
//! `_decref` (a freed dict releases its keys, a freed set of counted
//! instances its elements). A list or an instance keeps its count at
//! `pycc_rt::RC_OFFSET` in both objects, so the increment and the decrement
//! are inlined here; only a decrement that reaches zero calls the runtime,
//! on a cold path, to free the object and release what it holds. Instances
//! of an exception class are not counted (they are the runtime's leak-only
//! exception objects). An `object` counts through the shim's `Py_XINCREF`/
//! `Py_XDECREF` (`pycc_ext_obj_retain`/`pycc_ext_obj_release`); the GIL is
//! held wherever compiled code runs with CPython present.
//!
//! **`object` temporaries** are the one split: their *storage* follows this
//! module exactly (slots, parameters, returns, attributes, arguments,
//! pins), but a consumer that does not keep an `object` operand releases it
//! through `object_release` (whose `is_produced` is this module's "not a
//! borrowed read"), not through [`release_scalar_if_temporary`] or
//! [`hold_temporary`], which skip `object` values so that no operand is
//! released twice.
//!
//! Two refinements keep borrowed reads safe and temporaries balanced:
//! an operand a later sibling could free is *pinned* (`operand_pin`), and
//! a fresh temporary live while something can raise is held on the
//! pending-release stack ([`hold_temporary`]) that every exception edge
//! releases.

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
    Dict,
    Set,
    Object,
    /// A generator object, counted through `pycc_rt_gen_incref`/`_decref`
    /// (whose last release closes an unfinished generator).
    Generator,
    /// A function value, counted through `pycc_rt_closure_incref`/`_decref`
    /// (whose last release releases its captured values).
    Closure,
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
        Ty::Dict(_) => Some(HeapKind::Dict),
        Ty::Set(_) | Ty::FrozenSet(_) => Some(HeapKind::Set),
        Ty::Object => Some(HeapKind::Object),
        Ty::Generator(_) => Some(HeapKind::Generator),
        Ty::Callable(_) => Some(HeapKind::Closure),
        // A boxed tuple is a `PyListObj` (`containers.rs`).
        Ty::Tuple(elements) if pycc_hir::tuple_is_boxed(elements) => Some(HeapKind::List),
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
        (HeapKind::Dict, Scalar::Dict(ptr)) => Some((HeapKind::Dict, *ptr)),
        (HeapKind::Set, Scalar::Set(ptr)) => Some((HeapKind::Set, *ptr)),
        (HeapKind::Object, Scalar::Object(ptr)) => Some((HeapKind::Object, *ptr)),
        (HeapKind::Generator, Scalar::Generator(ptr)) => Some((HeapKind::Generator, *ptr)),
        (HeapKind::Closure, Scalar::Closure(ptr)) => Some((HeapKind::Closure, *ptr)),
        _ => None,
    }
}

/// [`counted`] for a consumer's temporary: an `object` operand is released
/// by `object_release` instead (see the module doc).
fn counted_temporary<'ctx>(
    rt: &RtFns<'ctx>,
    ty: &Ty,
    scalar: &Scalar<'ctx>,
) -> Option<(HeapKind, PointerValue<'ctx>)> {
    counted(rt, ty, scalar).filter(|(kind, _)| *kind != HeapKind::Object)
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
    let out_of_line = match kind {
        HeapKind::Str => Some(rt.str_incref),
        HeapKind::Dict => Some(rt.dict_incref),
        HeapKind::Set => Some(rt.set_incref),
        HeapKind::Object => Some(rt.obj_retain),
        HeapKind::Generator => Some(rt.gens.incref),
        HeapKind::Closure => Some(rt.closures.incref),
        HeapKind::List | HeapKind::Instance => None,
    };
    if let Some(incref) = out_of_line {
        builder
            .build_call(incref, &[ptr.into()], "rc_incref")
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
    let out_of_line = match kind {
        HeapKind::Str => rt.str_decref,
        HeapKind::Dict => rt.dict_decref,
        HeapKind::Set => rt.set_decref,
        HeapKind::List => rt.list_free,
        HeapKind::Instance => rt.instance_free,
        HeapKind::Object => rt.obj_release,
        HeapKind::Generator => rt.gens.decref,
        HeapKind::Closure => rt.closures.decref,
    };
    if !matches!(kind, HeapKind::List | HeapKind::Instance) {
        builder
            .build_call(out_of_line, &[ptr.into()], "rc_decref")
            .expect("build_call should not fail for a well-formed decref");
        return;
    }
    let free = out_of_line;
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
        && let Some((kind, ptr)) = counted_temporary(rt, &source.ty(), scalar)
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

/// The counted value a slot of type `ty` at `slot_ptr` holds, loaded
/// before a store overwrites it, for [`release_old`] once the new value is
/// in place. Storing first and releasing after (CPython's `Py_XSETREF`
/// order) matters for an `object`: its release can run a finalizer that
/// reads the slot, which must see the new value, never a freed one.
pub(crate) fn take_slot_old<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    rt: &RtFns<'ctx>,
    ty: &Ty,
    slot_ptr: PointerValue<'ctx>,
) -> Option<(HeapKind, PointerValue<'ctx>)> {
    let kind = heap_kind(rt, ty)?;
    let old = builder
        .build_load(
            context.ptr_type(inkwell::AddressSpace::default()),
            slot_ptr,
            "old_heap",
        )
        .expect("build_load should not fail for a slot this function allocated")
        .into_pointer_value();
    Some((kind, old))
}

/// The counted value an instance slot holds, loaded before a store
/// overwrites it (see [`take_slot_old`]). An unassigned slot holds the `0`
/// word, which releases nothing.
pub(crate) fn take_attr_slot_old<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    rt: &RtFns<'ctx>,
    ty: &Ty,
    base: PointerValue<'ctx>,
    slot: u64,
) -> Option<(HeapKind, PointerValue<'ctx>)> {
    let kind = heap_kind(rt, ty)?;
    let raw = attr_slot::emit_slot_load(context, builder, base, slot);
    let old = builder
        .build_int_to_ptr(
            raw,
            context.ptr_type(inkwell::AddressSpace::default()),
            "old_attr",
        )
        .expect("build_int_to_ptr should not fail reinterpreting an i64 as a pointer");
    Some((kind, old))
}

/// Releases a value [`take_slot_old`] or [`take_attr_slot_old`] loaded.
pub(crate) fn release_old<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    rt: &RtFns<'ctx>,
    old: Option<(HeapKind, PointerValue<'ctx>)>,
) {
    if let Some((kind, ptr)) = old {
        emit_decref(context, builder, rt, kind, ptr);
    }
}

/// The depth of the pending-release stack, for a later
/// [`truncate_pending`]: a consumer holds its operands with
/// [`hold_temporary`] while later siblings or its own operation may raise,
/// then truncates back to the mark on the fallthrough, where it releases
/// (or stores) them itself. Nested consumers truncate only their own
/// entries.
pub(crate) fn pending_mark(rt: &RtFns<'_>) -> usize {
    rt.exceptions.pending_heap_releases.borrow().len()
}

/// Drops the entries pushed since `mark` without releasing them.
pub(crate) fn truncate_pending(rt: &RtFns<'_>, mark: usize) {
    rt.exceptions
        .pending_heap_releases
        .borrow_mut()
        .truncate(mark);
}

/// Holds `scalar` (the value of `source`) on the pending-release stack when
/// it is a fresh counted reference, so an exception edge emitted before the
/// matching [`truncate_pending`] releases it instead of leaking it.
pub(crate) fn hold_temporary<'ctx>(rt: &RtFns<'ctx>, source: &MirExpr, scalar: &Scalar<'ctx>) {
    if !is_borrowed_read(source)
        && let Some(entry) = counted_temporary(rt, &source.ty(), scalar)
    {
        rt.exceptions.pending_heap_releases.borrow_mut().push(entry);
    }
}

/// [`hold_temporary`] answering the entry it pushed, for a consumer whose
/// holds do not nest as a contiguous run above one [`pending_mark`]
/// (`native_operand`): it retires the entry itself with [`unhold`].
pub(crate) fn hold_temporary_entry<'ctx>(
    rt: &RtFns<'ctx>,
    source: &MirExpr,
    scalar: &Scalar<'ctx>,
) -> Option<(HeapKind, PointerValue<'ctx>)> {
    if is_borrowed_read(source) {
        return None;
    }
    let entry = counted_temporary(rt, &source.ty(), scalar)?;
    rt.exceptions.pending_heap_releases.borrow_mut().push(entry);
    Some(entry)
}

/// Removes the entry [`hold_temporary_entry`] pushed, without releasing it
/// (the fallthrough releases it itself). The topmost equal entry is the
/// one: a pointer is held at most once by one consumer at a time.
pub(crate) fn unhold<'ctx>(rt: &RtFns<'ctx>, entry: (HeapKind, PointerValue<'ctx>)) {
    let mut stack = rt.exceptions.pending_heap_releases.borrow_mut();
    let position = stack.iter().rposition(|held| *held == entry).expect(
        "pycc_codegen: internal error: a held native temporary is missing from \
         pending_heap_releases",
    );
    stack.remove(position);
}

/// Holds `scalar`, an owned reference of type `ty` (a fresh value, or a
/// borrowed one just retained), when its type is counted.
pub(crate) fn hold_owned_value<'ctx>(rt: &RtFns<'ctx>, ty: &Ty, scalar: &Scalar<'ctx>) {
    if let Some(entry) = counted(rt, ty, scalar) {
        rt.exceptions.pending_heap_releases.borrow_mut().push(entry);
    }
}

/// Holds an owned reference of `kind` at `ptr` (an object being built).
pub(crate) fn hold_owned<'ctx>(rt: &RtFns<'ctx>, kind: HeapKind, ptr: PointerValue<'ctx>) {
    rt.exceptions
        .pending_heap_releases
        .borrow_mut()
        .push((kind, ptr));
}

/// Releases every held temporary: emitted on an exception edge
/// (`exception::jump_to_exception_target`), which leaves the statement.
pub(crate) fn release_pending<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    rt: &RtFns<'ctx>,
) {
    let snapshot = rt.exceptions.pending_heap_releases.borrow().clone();
    for (kind, ptr) in snapshot.into_iter().rev() {
        emit_decref(context, builder, rt, kind, ptr);
    }
}

#[cfg(test)]
#[path = "heap_rc_tests.rs"]
mod tests;
