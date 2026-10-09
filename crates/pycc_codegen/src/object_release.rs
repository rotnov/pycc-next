//! The release protocol for CPython object temporaries (Part 1 of #1092).
//!
//! Every shim producer -- an attribute load, a call, a subscript, a rich
//! comparison, `type(o)`, a slice, a list display, a tuple unpack -- hands
//! compiled code a *new* `PyObject *` reference. Before this module every
//! one of them was leaked on purpose (the leak-only rule `docs/RUNTIME.md`
//! recorded), which made a foreign operation inside a loop leak one
//! reference per trip.
//!
//! This module releases the references that are **temporaries**: a
//! produced value that a borrowing operation consumes and that is bound
//! nowhere. That is the operand of another object operation (the base of
//! `o.a.b`, the argument of `o.m(p.q)`, the operand of `len(o.x)`), a
//! condition (`if o.ready():`), the operand of a conversion (`float(o.x)`),
//! and a discarded statement value (`o.update()`). Each such site is a
//! *consumer*: it evaluates the operand, calls [`hold`] on it, runs the
//! operation, and then calls [`Held::release`], which emits one call to the
//! shim's `pycc_ext_obj_release` (`Py_XDECREF`).
//!
//! **Ownership is decided by MIR shape, never by type.** [`is_produced`] is
//! an allowlist of the shim's new-reference producers, the one
//! classification `foreign_call::callee_is_produced` also answers through.
//! Everything else that evaluates to a `Scalar::Object` is a *borrow* --
//! a `Name` read of a module global or a slot, a function parameter, a
//! `for` target, a boxed value, a user function's `object` result -- and is
//! never released here: a function local carries a borrowed or moved
//! pointer with no reference-count traffic of its own, and a module
//! global's reference belongs to the global (`object_slot.rs`), so
//! releasing one would underflow a reference the slot still uses. An
//! unlisted node therefore defaults to the old leak, never to a
//! use-after-free.
//!
//! **The exception edge.** An operand that is held while a later sibling
//! or the consuming operation itself can fail sits on
//! `ExceptionCodegenState::pending_object_releases` for exactly that
//! window. Both ways out of a statement on a failure -- the post-node guard
//! (`exception::guard_statement_effects`, through
//! `exception::jump_to_exception_target`) and a foreign failure edge
//! (`foreign_fail::emit_failure`, including its direct
//! `EXT_MODULE_EXEC_FAILED` return) -- release a snapshot of that stack
//! before they leave, exactly as they already release
//! `pending_int_releases` (#638, D-208). The fallthrough pops the entry and
//! releases it once, after the operation, so no path releases twice.
//!
//! A reference the operation **consumes** -- the bound method
//! `pycc_ext_obj_call` releases, or a produced callee the consuming call
//! helper takes -- is held across the argument evaluations, where an
//! argument's failure would orphan it, and then [`Held::consumed`] pops it
//! without a release before the call that takes it over.
//!
//! An empty stack costs nothing: every guard site keeps its two-block
//! shape, so a function that performs no object operation (the D-084/D-140
//! nbody hot loop) emits exactly the code it always did.
//!
//! **Iteration and operand temporaries** (Part 3 of #1092). The iterator
//! `pycc_ext_obj_get_iter` returns for a comprehension over an object, and
//! that comprehension's result collection while it is being filled, are
//! held on the same pending stack for the comprehension's whole extent --
//! an expression, so no `try` can sit inside it -- and the iterator is
//! released at the loop's exit. A comprehension over an object is itself
//! a producer in [`is_produced`], so an unbound result (`len([...])`, a
//! discarded one) is released by the consumer that reads it.
//!
//! A `for x in <object>:` statement's iterator cannot use that stack: the
//! loop body is a statement suite, and a `try` inside it would release the
//! iterator on its own caught edge. [`enter_loop_iterator`] instead pushes
//! a *cleanup block* onto the exception-target stack -- release the
//! iterator, then branch to the target that enclosed the loop -- so every
//! exception edge out of the body runs it, and [`LoopIterator::exit`]
//! releases the iterator on the normal exit. The module-exec direct return
//! (`foreign_fail::emit_failure`) looks through those blocks with
//! [`loop_iterators_above_handler`] and releases their iterators itself,
//! so a failure in the loop still fails the import with CPython's own
//! exception set. `break` is not admitted yet (C0001), so the loop has no
//! third exit.
//!
//! The operand of `print`, of an f-string interpolation and of
//! `raise <object>` is a consumer like any other, and so is the condition
//! of a conditional expression and every operand of a truth-only `and`/`or`.
//! A value `and`/`or` holds its left operand across that operand's truth
//! test and releases it on the arm that discards it; the node is a
//! producer itself only when every arm it can select is owned (a produced
//! object or a boxed native value), and a conditional expression only when
//! both of its arms are produced objects.
//!
//! **Bound values** are not temporaries and are never released here. Since
//! Part 1 of #1499 a module-global `object` slot owns the reference it holds
//! and releases its previous value on rebind -- including a module-level
//! `for x in <object>:` target's per-trip item -- in `object_slot.rs`, not
//! through this stack. **Not yet released** (the later parts of #1499): a
//! produced value bound to a function local or a compiled-instance
//! attribute, passed to a user function, returned, or boxed; the per-trip
//! item of a comprehension over an object (an object `for` is admitted only
//! in a module body, whose target owns its item), which the loop variable's
//! frame slot holds without a reference
//! of its own -- a body can hand it to a user function that stores it, so
//! releasing it at the trip's end needs the frame-slot model (Part 2,
//! #1502); and the result of an `and`/`or` or a conditional expression
//! whose arms mix produced and borrowed objects.
//! `hash(<object>)` is not admitted yet (C0001), so it has no site. The
//! read of a narrowed `object` name (`MirExpr::ObjectUnbox`) needs no
//! release: its operand is always a borrowed slot.

use super::*;
use crate::ext::EXT_OBJ_RELEASE_SYMBOL;
use inkwell::basic_block::BasicBlock;
use inkwell::builder::Builder;

/// One held reference: the pointer and the release helper declared in the
/// module it was emitted into.
///
/// The helper travels with the pointer because the unwind
/// (`exception::jump_to_exception_target`) has no module in scope to
/// declare it from.
#[derive(Clone, Copy, PartialEq, Debug)]
pub(super) struct PendingObject<'ctx> {
    pointer: PointerValue<'ctx>,
    release: FunctionValue<'ctx>,
}

/// A produced operand on `pending_object_releases`, or nothing when the
/// operand is a borrow. Every `Held` is retired exactly once, by
/// [`Held::release`] or [`Held::consumed`], before the statement ends.
#[must_use = "a held operand must be released or consumed, or the pending stack drifts"]
pub(super) struct Held<'ctx>(Option<PendingObject<'ctx>>);

/// Whether `expr` evaluates to a new reference that nothing else holds.
///
/// An allowlist of the shim's own producers; see the module doc for why an
/// unlisted node is treated as a borrow. A comparison is a producer only
/// when it is a rich comparison: `is`/`is not` answers a native `bool`.
pub(super) fn is_produced(expr: &MirExpr) -> bool {
    match expr {
        MirExpr::ObjAttrGet { .. }
        | MirExpr::ObjMethodCall { .. }
        | MirExpr::ObjCall { .. }
        | MirExpr::ObjKeywordCall(_)
        | MirExpr::ObjSubscript { .. }
        | MirExpr::ObjType { .. }
        | MirExpr::ObjSlice { .. }
        | MirExpr::ObjList { .. }
        | MirExpr::ObjUnpack { .. } => true,
        MirExpr::ObjCompare { .. } => expr.ty() == pycc_mir::Ty::Object,
        // Part 3 of #1092: the fresh CPython `list`/`set` the comprehension
        // builds (`object_comprehension.rs`).
        MirExpr::Comprehension(comp) => matches!(comp.source, pycc_mir::CompSource::Object(_)),
        MirExpr::BoolOp {
            left,
            right,
            ty: pycc_mir::Ty::Object,
            truth_only: false,
            ..
        } => owned_arm(left) && owned_arm(right),
        MirExpr::IfExp {
            body,
            orelse,
            ty: pycc_mir::Ty::Object,
            ..
        } => is_produced_object(body) && is_produced_object(orelse),
        _ => false,
    }
}

/// Whether `operand`, an arm an `object`-valued `and`/`or` can select, is
/// a reference the node owns: a produced object, or a native value
/// `boolop.rs` boxes into a new reference (`boolop::needs_boxing`). An
/// arm of any other type is never treated as owned.
fn owned_arm(operand: &MirExpr) -> bool {
    match operand.ty() {
        pycc_mir::Ty::Bool | pycc_mir::Ty::Int | pycc_mir::Ty::Float | pycc_mir::Ty::Str => true,
        _ => is_produced_object(operand),
    }
}

/// Whether `operand` is an `object` arm evaluated to a produced reference.
fn is_produced_object(operand: &MirExpr) -> bool {
    operand.ty() == pycc_mir::Ty::Object && is_produced(operand_source(operand))
}

/// The node an operand's value really comes from:
/// `object_unbox::emit_pack_operand` evaluates the object inside an
/// `ObjectUnbox` rather than the unboxed read, so a held operand must be
/// classified the same way.
pub(super) fn operand_source(expr: &MirExpr) -> &MirExpr {
    match expr {
        MirExpr::ObjectUnbox(object, _) => object,
        other => other,
    }
}

/// Declares `void pycc_ext_obj_release(PyObject *)` once per module.
fn release_fn<'ctx>(
    context: &'ctx Context,
    module: &inkwell::module::Module<'ctx>,
) -> FunctionValue<'ctx> {
    crate::foreign_pack::shim_fn(
        module,
        EXT_OBJ_RELEASE_SYMBOL,
        context.void_type().fn_type(
            &[context.ptr_type(inkwell::AddressSpace::default()).into()],
            false,
        ),
    )
}

/// The pending entry for `scalar`, evaluated from `expr`, when it is a
/// produced object.
fn produced<'ctx>(
    context: &'ctx Context,
    module: &inkwell::module::Module<'ctx>,
    expr: &MirExpr,
    scalar: &Scalar<'ctx>,
) -> Option<PendingObject<'ctx>> {
    match scalar {
        Scalar::Object(pointer) if is_produced(operand_source(expr)) => Some(PendingObject {
            pointer: *pointer,
            release: release_fn(context, module),
        }),
        _ => None,
    }
}

fn emit_release<'ctx>(builder: &Builder<'ctx>, object: PendingObject<'ctx>) {
    builder
        .build_call(object.release, &[object.pointer.into()], "obj_release")
        .expect("build_call should not fail for pycc_ext_obj_release");
}

/// Holds `scalar`, the already-evaluated operand `expr`, on the pending
/// stack when it is a produced object, so that every failure edge emitted
/// until the matching [`Held`] is retired releases it.
pub(super) fn hold<'ctx>(
    context: &'ctx Context,
    module: &inkwell::module::Module<'ctx>,
    rt: &RtFns<'ctx>,
    expr: &MirExpr,
    scalar: &Scalar<'ctx>,
) -> Held<'ctx> {
    let pending = produced(context, module, expr, scalar);
    if let Some(object) = pending {
        rt.exceptions
            .pending_object_releases
            .borrow_mut()
            .push(object);
    }
    Held(pending)
}

/// Holds `pointer`, a reference the shim just produced (a bound method
/// from `foreign_call::emit_lookup`), without a MIR node to classify.
pub(super) fn hold_new_reference<'ctx>(
    context: &'ctx Context,
    module: &inkwell::module::Module<'ctx>,
    rt: &RtFns<'ctx>,
    pointer: PointerValue<'ctx>,
) -> Held<'ctx> {
    let object = PendingObject {
        pointer,
        release: release_fn(context, module),
    };
    rt.exceptions
        .pending_object_releases
        .borrow_mut()
        .push(object);
    Held(Some(object))
}

impl<'ctx> Held<'ctx> {
    /// Removes this entry from the stack. Usually the top one, but not
    /// always: a call's bound method is retired before the arguments held
    /// above it, which stay held across the call that consumes it.
    fn pop(&self, rt: &RtFns<'ctx>) {
        if let Some(object) = self.0 {
            let mut stack = rt.exceptions.pending_object_releases.borrow_mut();
            let position = stack.iter().rposition(|entry| *entry == object).expect(
                "pycc_codegen: internal error: a held object is missing from \
                 pending_object_releases",
            );
            stack.remove(position);
        }
    }

    /// Retires the hold on the fallthrough path and releases the operand,
    /// now that the operation borrowing it is done.
    pub(super) fn release(self, builder: &Builder<'ctx>, rt: &RtFns<'ctx>) {
        self.pop(rt);
        if let Some(object) = self.0 {
            emit_release(builder, object);
        }
    }

    /// Retires the hold without a release: the operation emitted next takes
    /// the reference over and releases it on every path itself.
    pub(super) fn consumed(self, rt: &RtFns<'ctx>) {
        self.pop(rt);
    }
}

/// Releases `scalar`, evaluated from `expr`, at once when it is a produced
/// object -- for a consumer whose own operation cannot fail after the value
/// exists (a discarded statement value), so no hold is needed.
pub(super) fn release_if_produced<'ctx>(
    context: &'ctx Context,
    builder: &Builder<'ctx>,
    module: &inkwell::module::Module<'ctx>,
    expr: &MirExpr,
    scalar: &Scalar<'ctx>,
) {
    if let Some(object) = produced(context, module, expr, scalar) {
        emit_release(builder, object);
    }
}

/// Emits a release of every currently held operand, innermost first, at
/// the builder's position -- the unwind half of the protocol, emitted on a
/// block that is about to leave the statement for an exception target or
/// the module-exec failure return. The stack itself is untouched: the
/// fallthrough still owns each entry's pop.
pub(super) fn release_pending<'ctx>(builder: &Builder<'ctx>, rt: &RtFns<'ctx>) {
    let snapshot: Vec<PendingObject<'ctx>> = rt.exceptions.pending_object_releases.borrow().clone();
    for object in snapshot.into_iter().rev() {
        emit_release(builder, object);
    }
}

/// One entry of `ExceptionCodegenState::loop_iterators`: an enclosing
/// `for x in <object>:` loop's cleanup target and the iterator it releases.
#[derive(Clone, Copy, PartialEq, Debug)]
pub(super) struct LoopCleanup<'ctx> {
    block: BasicBlock<'ctx>,
    iterator: PendingObject<'ctx>,
}

/// The iterator of a `for x in <object>:` statement, owned from the moment
/// `iter()` succeeds until the loop exits, on every path (Part 3 of #1092).
#[must_use = "a loop iterator must be exited, or the exception-target stack drifts"]
pub(super) struct LoopIterator<'ctx>(LoopCleanup<'ctx>);

/// Takes ownership of `iterator`, the new reference `iter()` just returned,
/// for the loop about to be emitted at the builder's position: appends a
/// `foreign_iter_cleanup` block that releases it and branches to the
/// exception target enclosing the loop, and installs that block as the
/// innermost target, so every exception edge out of the loop's header and
/// body -- the guard, a foreign failure's bridge, an explicit `raise`, a
/// handler's re-raise -- releases the iterator before it unwinds further.
pub(super) fn enter_loop_iterator<'ctx>(
    context: &'ctx Context,
    builder: &Builder<'ctx>,
    module: &inkwell::module::Module<'ctx>,
    rt: &RtFns<'ctx>,
    iterator: PointerValue<'ctx>,
) -> LoopIterator<'ctx> {
    let iterator = PendingObject {
        pointer: iterator,
        release: release_fn(context, module),
    };
    let enclosing = *rt
        .exceptions
        .targets
        .borrow()
        .last()
        .expect("a loop is always emitted inside an exception target");
    let current = builder
        .get_insert_block()
        .expect("the builder is positioned inside a block");
    let function = current
        .get_parent()
        .expect("every basic block belongs to a function");
    let block = context.append_basic_block(function, "foreign_iter_cleanup");
    builder.position_at_end(block);
    emit_release(builder, iterator);
    builder
        .build_unconditional_branch(enclosing)
        .expect("build_unconditional_branch should not fail for a fresh cleanup block");
    builder.position_at_end(current);
    let cleanup = LoopCleanup { block, iterator };
    rt.exceptions.targets.borrow_mut().push(block);
    rt.exceptions.loop_iterators.borrow_mut().push(cleanup);
    LoopIterator(cleanup)
}

impl<'ctx> LoopIterator<'ctx> {
    /// Uninstalls the loop's cleanup target once its body is emitted, and
    /// releases the iterator at the builder's position: the loop's normal
    /// exit, which the exhausted iterator branches to.
    pub(super) fn exit(self, builder: &Builder<'ctx>, rt: &RtFns<'ctx>) {
        let target = rt.exceptions.targets.borrow_mut().pop();
        let cleanup = rt.exceptions.loop_iterators.borrow_mut().pop();
        assert!(
            target == Some(self.0.block) && cleanup == Some(self.0),
            "pycc_codegen: internal error: a loop's cleanup target is not the innermost one \
             at its exit"
        );
        emit_release(builder, self.0.iterator);
    }
}

/// The innermost exception target once the cleanup blocks of the loops
/// directly above it are looked through, and those loops' iterators,
/// innermost first -- for `foreign_fail::emit_failure`, which keeps its
/// direct module-exec return when that target is the entry's own exit and
/// must then release the iterators the skipped blocks would have.
pub(super) fn loop_iterators_above_handler<'ctx>(
    rt: &RtFns<'ctx>,
) -> (Option<BasicBlock<'ctx>>, Vec<PendingObject<'ctx>>) {
    let targets = rt.exceptions.targets.borrow();
    let loops = rt.exceptions.loop_iterators.borrow();
    let mut iterators = Vec::new();
    let handler = targets.iter().rev().copied().find(|target| {
        let cleanup = loops.iter().find(|cleanup| cleanup.block == *target);
        iterators.extend(cleanup.map(|cleanup| cleanup.iterator));
        cleanup.is_none()
    });
    (handler, iterators)
}

/// Emits a release of each of `objects`, in order.
pub(super) fn release_all<'ctx>(builder: &Builder<'ctx>, objects: &[PendingObject<'ctx>]) {
    for object in objects {
        emit_release(builder, *object);
    }
}

#[cfg(test)]
#[path = "object_release_tests.rs"]
mod tests;
