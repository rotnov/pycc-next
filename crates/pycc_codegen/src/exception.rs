//! Exception-specific code-generation state and expression guards (D-173).

use super::exception_value::emit_str_bytes_constant;
use super::*;
use std::cell::{Cell, RefCell};

pub(super) struct ExceptionCodegenState<'ctx> {
    pub(super) reraise_values: RefCell<Vec<PointerValue<'ctx>>>,
    pub(super) targets: RefCell<Vec<inkwell::basic_block::BasicBlock<'ctx>>>,
    /// Stack of already-evaluated, not-yet-released `Ty::Int` birth
    /// references that a sibling expression's evaluation might orphan by
    /// raising before the owning node's own release call is reached
    /// (#638, D-208). `guard_statement_effects` walks a snapshot of this
    /// stack when it routes an exception edge away from the normal
    /// fallthrough, releasing each entry exactly once. Every push has a
    /// matching pop on the fallthrough path (see each call site's own
    /// comment); the stack is otherwise empty, in particular at every
    /// statement boundary and at every guard site the D-084/D-140 nbody
    /// hot loop reaches, which is what keeps that loop's codegen
    /// unchanged (see `guard_statement_effects`'s own doc comment).
    pub(super) pending_int_releases: RefCell<Vec<IntValue<'ctx>>>,
    /// The CPython-object counterpart of `pending_int_releases` (Part 1 of
    /// #1092): produced object temporaries a consumer holds while a sibling
    /// or the consuming operation itself can still fail. The same two
    /// unwinds release a snapshot of it, and it is empty at every statement
    /// boundary; `object_release.rs` owns the push/pop discipline.
    pub(super) pending_object_releases: RefCell<Vec<crate::object_release::PendingObject<'ctx>>>,
    /// The iterator of every enclosing `for x in <object>:` loop, paired
    /// with the cleanup block it pushed onto `targets` (Part 3 of #1092).
    /// Unlike `pending_object_releases` it lives across whole statements:
    /// the cleanup block releases the iterator on every exception edge out
    /// of the loop, and `foreign_fail::emit_failure` reads this stack to
    /// look through those blocks for its direct module-exec return.
    /// `object_release::LoopIterator` owns the push/pop discipline.
    pub(super) loop_iterators: RefCell<Vec<crate::object_release::LoopCleanup<'ctx>>>,
    /// The `ext` module entry's own `top_exception_exit` while its body is
    /// emitted, and `None` otherwise (Part 1 of #1096). When it is also the
    /// innermost entry of `targets` -- looking through `loop_iterators`'
    /// cleanup blocks (Part 3 of #1092) -- no module-level `try` encloses the
    /// statement being emitted, and that target means "fail the import
    /// directly": `foreign_fail::emit_failure` then keeps the direct
    /// `EXT_MODULE_EXEC_FAILED` return instead of bridging a foreign failure.
    pub(super) module_exec_exit: Cell<Option<inkwell::basic_block::BasicBlock<'ctx>>>,
}

impl ExceptionCodegenState<'_> {
    pub(super) fn new() -> Self {
        Self {
            reraise_values: RefCell::new(Vec::new()),
            targets: RefCell::new(Vec::new()),
            pending_int_releases: RefCell::new(Vec::new()),
            pending_object_releases: RefCell::new(Vec::new()),
            loop_iterators: RefCell::new(Vec::new()),
            module_exec_exit: Cell::new(None),
        }
    }
}

/// Returns whether evaluating this node's own operation can set D-173's
/// pending-exception state. Child expressions classify and guard themselves,
/// so this deliberately does not recurse. Keeping infallible arithmetic off
/// the guarded path preserves the compiler's performance contract while the
/// operations converted to catchable Python exceptions remain fail-closed.
pub(super) fn expression_can_set_exception(expr: &MirExpr) -> bool {
    match expr {
        // `AttrGet` joins the `true` group with #1388, and the dependency is
        // live: `pycc_rt_instance_get_slot_checked` sets pycc's own D-173
        // pending state (`AttributeError`) and returns a `0` word for a slot
        // read before its assignment, so this guard is the whole of the
        // failure handling.
        //
        // `ObjAttrGet` joins the `true` group too (D-244, Part 2 of #1026):
        // like `AttrGet`, but unlike its compile-time-resolved slot, a
        // foreign attribute load is a real `PyObject_GetAttrString` call
        // that returns `NULL` with a CPython exception set whenever the
        // attribute is missing or its descriptor raises.
        //
        // The guard this answer emits is not what *handles* that failure:
        // it reads pycc's own pending state (D-173), which a CPython-set
        // exception leaves untouched, so `foreign_attr::emit` emits its own
        // `NULL` check ahead of it (PR 2a of #1081 review finding 1).
        //
        // `ObjMethodCall` (PR 2b of #1081) joins it for exactly the same
        // reason: a missing method or a method that raises makes
        // `pycc_ext_obj_call` return `NULL` with CPython's error indicator
        // set, and `foreign_call::emit` owns the `NULL` check that actually
        // stops the module body.
        //
        // So for both nodes the `true` answer is fail-closed conservatism
        // rather than a live dependency: the emitter's own check is
        // immediate. In the module body outside every module-level `try` it
        // returns the `-1` status; anywhere else (#1316, Part 1 of #1096) it
        // calls `pycc_ext_obj_error_bridge`,
        // which moves CPython's exception into pycc's pending state, and
        // branches straight to the innermost exception target
        // (`foreign_fail.rs`). Either way the D-173 guard emitted after the
        // node sees no pending exception on the path it guards, and this
        // classification stays correct should a future emitter defer its
        // branch to that guard instead.
        MirExpr::Call { .. }
        | MirExpr::AttrGet { .. }
        | MirExpr::DictGet { .. }
        | MirExpr::Instantiate(_)
        | MirExpr::ObjAttrGet { .. }
        | MirExpr::ObjMethodCall { .. }
        // #1313: a direct call joins `ObjMethodCall` on the identical
        // argument -- a non-callable object or a raising call makes
        // `pycc_ext_obj_call_borrowed` return `NULL`.
        | MirExpr::ObjCall { .. }
        // Part 8 of #1371: the keyword form, on the same argument (an
        // unexpected keyword is CPython's own `TypeError`).
        | MirExpr::ObjKeywordCall(_)
        // PR 3a of #1082: `ObjLen` joins them on the identical argument --
        // `PyObject_Size` raises `TypeError` for an operand with no length,
        // and `foreign_len::emit_len` owns the `-1` check that actually
        // stops the module body.
        | MirExpr::ObjLen { .. }
        // PR 3b of #1082: `ObjSubscript` joins them on the identical
        // argument -- `PyObject_GetItem` raises `KeyError`, `IndexError` or
        // `TypeError`, and `foreign_call::emit_subscript` owns the `NULL`
        // check that actually stops the module body.
        | MirExpr::ObjSubscript { .. }
        // PR 4c of #1083: `ObjUnpackFloatTuple` joins them on the identical
        // argument -- the shim raises `TypeError` for a non-tuple operand
        // and for a wrong arity, and propagates whatever `PyNumber_Float`
        // raises for an item it cannot convert;
        // `foreign_len::emit_unpack_float_tuple` owns the `-1` check that
        // actually stops the module body.
        | MirExpr::ObjUnpackFloatTuple { .. }
        // Part 1 of #891: `ObjUnpack` joins them -- the shim raises
        // CPython's own `TypeError` for a non-iterable and `ValueError` for
        // a wrong item count, and propagates a raising `__iter__` or
        // `__next__`; `foreign_unpack::emit_unpack` owns the `NULL` check.
        | MirExpr::ObjUnpack { .. }
        // Part 2 of #1027: `BufferGet` joins them, and here the dependency
        // *is* live rather than conservative -- `pycc_rt_buffer_f64_get`
        // sets pycc's own D-173 pending state and returns a `0.0` sentinel
        // for an index outside `[0, len)`, so this guard is the whole of
        // the failure handling. A by-name entry rather than a widening of
        // the `Subscript` arm below, which asks its base's type: a
        // `BufferGet` is always fallible, with no base type to consult.
        | MirExpr::BufferGet { .. }
        // #1165: `BufferAlloc` joins `BufferGet` rather than `BufferLen`,
        // and here the dependency is live for the identical reason --
        // `pycc_rt_buffer_f64_alloc` sets pycc's own D-173 pending state and
        // returns a *null* view for a negative length, so this guard is the
        // whole of the failure handling. The null is what makes the guard
        // sufficient rather than merely conservative: the epilogue's free is
        // a documented no-op on it, so a refused allocation leaves nothing
        // for the unwind path to release.
        | MirExpr::BufferAlloc { .. }
        // #1254 (D-250): a comprehension expression runs a whole loop whose
        // iterable, condition and elements may each raise -- a `range` step
        // of `0` or a `1 // 0` element -- so it is always fallible, exactly
        // like the statement form the loop body already guards.
        | MirExpr::Comprehension(_)
        // Part 1 of #1371: `PyObject_IsInstance` raises for a class
        // argument that is not a class, and propagates a raising
        // `__instancecheck__`; `foreign_compare::emit_isinstance` owns the
        // `-1` check.
        | MirExpr::ObjIsInstance { .. }
        // Part 2b of #1371: `PySequence_Contains` runs `__contains__` (or
        // iteration), and `PyObject_GetItem` on a slice runs `__getitem__`;
        // either may raise. `foreign_compare::emit_contains` owns the `-1`
        // check and `foreign_slice::emit_slice` the `NULL` check.
        | MirExpr::ObjContains { .. }
        | MirExpr::ObjSlice { .. }
        // Part 11 of #1371: `pycc_ext_obj_type` answers `NULL` for a `NULL`
        // operand (its defence-in-depth guard), and
        // `foreign_type::emit_type` owns that check.
        | MirExpr::ObjType { .. }
        // Part 2d of #1371: boxing an element or allocating the CPython
        // `list` may fail, and `foreign_call::emit_list` owns the `NULL`
        // check.
        | MirExpr::ObjList { .. }
        // Part 2 of #1387: boxing a native value into an `object` slot may
        // fail (a bigint outside the inline range is `OverflowError`), and
        // `object_box::emit_boxed` owns the `NULL` check.
        | MirExpr::ObjectBox(_)
        // #1476: a narrowed `object` read may fail (an `int` outside the
        // inline range is `OverflowError`), and `object_unbox::emit_unboxed`
        // owns the status check.
        | MirExpr::ObjectUnbox(_, _) => true,
        // Part 1 of #1371: a rich comparison runs the operands' own
        // `__eq__`/`__lt__`/..., which may raise, and
        // `foreign_compare::emit_compare` owns the `NULL` check; an
        // identity test is a pointer compare and cannot raise.
        MirExpr::ObjCompare { op, .. } => !matches!(
            op,
            pycc_mir::CmpOpKind::Is | pycc_mir::CmpOpKind::IsNot
        ),
        // #1346: the node itself raises nothing. `sequence::emit_sequence`
        // emits both children through the guarded `emit_expr`, so each
        // child's own guard has already handled a raise by the time the
        // sequence yields.
        MirExpr::Sequence { .. } => false,
        // #1210: a shift raises `ValueError` for a negative count and
        // `OverflowError` for a result too large to allocate; `&`, `|` and
        // `^` never raise.
        MirExpr::BinOp { op, .. } => matches!(
            op,
            pycc_mir::BinOpKind::Div
                | pycc_mir::BinOpKind::FloorDiv
                | pycc_mir::BinOpKind::Mod
                | pycc_mir::BinOpKind::LShift
                | pycc_mir::BinOpKind::RShift
        ),
        MirExpr::Subscript { base, .. } => matches!(base.ty(), pycc_mir::Ty::List(_)),
        // #1343: an instance set insertion that calls a user `__hash__` or
        // `__eq__` guards each call itself, but the raise can still be
        // pending when the statement ends. An `int` or identity-only
        // insertion falls through to the non-raising list below.
        MirExpr::SetLiteral { ops: Some(ops), .. } | MirExpr::SetAdd { ops: Some(ops), .. }
            if ops.calls_user_code() =>
        {
            true
        }
        MirExpr::IntLiteral(_)
        | MirExpr::FloatLiteral(_)
        | MirExpr::BoolLiteral(_)
        | MirExpr::IntBoundary(_)
        | MirExpr::StringLiteral(_)
        | MirExpr::NoneLiteral
        | MirExpr::NotImplemented
        | MirExpr::Name { .. }
        | MirExpr::Compare { .. }
        | MirExpr::FString(_)
        | MirExpr::ListLiteral(_)
        | MirExpr::EmptyList(_)
        | MirExpr::ListAppend { .. }
        | MirExpr::DictLiteral(_)
        | MirExpr::EmptyDict(_)
        | MirExpr::SetLiteral { .. }
        // Part 1 of #1319: `frozenset(x)` copies an already-validated
        // container, exactly as a set literal inserts already-validated
        // words -- the runtime's bigint raise is unreachable from a
        // `list[int]` codegen built, as `pycc_rt_int_set_from_int_list`'s
        // own doc comment records.
        | MirExpr::FrozenSetFrom { .. }
        // #1335: `hash(instance)`'s own work -- a pointer hash, a
        // `slot_tp_hash` reduction, an int birth -- cannot fail. A user
        // `__hash__` is the inner `Call`, which guards itself where
        // `emit_expr` evaluates it, before this node's work runs.
        | MirExpr::InstanceHash { .. }
        | MirExpr::TupleLiteral(_)
        | MirExpr::Slice { .. }
        | MirExpr::ListPop { .. }
        | MirExpr::DictGetOrDefault { .. }
        | MirExpr::SetAdd { .. }
        | MirExpr::NullInstance { .. }
        // Part 3A of #541 (#736): reading an already-caught exception's
        // message pointer off its object is a plain field load -- it cannot
        // itself allocate,
        // divide, index, or otherwise fail, so it cannot set D-173's
        // pending-exception state. The wrapped sub-expression is not
        // re-inspected here either, matching every other arm's own
        // "classify only this node's operation" rule.
        | MirExpr::ExceptionMessage(_)
        // #1337 (WI-6a): reading a caught exception's type tag is a plain
        // field load, like `ExceptionMessage` above.
        | MirExpr::ExceptionTypeTest { .. }
        // `OptionalWrap` (D-197, #763) only re-tags a value that has
        // already been evaluated for `.ty()`'s benefit; like
        // `IntBoundary` immediately above, the struct-building work in
        // `coerce_scalar_to_type` it drives is infallible, so it cannot
        // itself set D-173's pending-exception state. The wrapped
        // sub-expression is not re-inspected here, mirroring every other
        // arm in this match, which classifies only the node's own
        // operation and relies on child expressions to guard themselves.
        | MirExpr::OptionalWrap(_, _)
        // PEP 572 (#774): `target := value`. Storing an already-evaluated
        // value into a predeclared slot is a plain store, exactly as
        // infallible as `ExceptionMessage`'s field load or `OptionalWrap`'s
        // re-tag above -- it cannot itself allocate, divide, index, or otherwise
        // fail. The wrapped `value` sub-expression is not re-inspected
        // here either, matching this function's own "classify only this
        // node's operation, let child expressions guard themselves" rule;
        // `value`'s own guard (if any) is applied where `value` is itself
        // evaluated, before this node's store ever runs.
        | MirExpr::NamedExpr { .. }
        // Issue #769 (Part 2 of #747): `OptionalUnwrap` only extracts a
        // field out of a struct value that has already been evaluated --
        // like `OptionalWrap` immediately above, a plain `build_extract_value`
        // is infallible, so it cannot itself set D-173's pending-exception
        // state. The wrapped sub-expression is not re-inspected here either,
        // mirroring every other arm in this match.
        | MirExpr::OptionalUnwrap(_, _)
        // #604 (Part 3 of #573): `not x` computes the same `truthy` check
        // an `if`/`while` condition already evaluates (see `pycc_codegen`'s
        // `truthy` helper), then inverts the resulting `i1` -- a compare
        // and an xor, both infallible. The wrapped operand is not
        // re-inspected here either, matching every other arm's own
        // "classify only this node's operation" rule. (Part 10 of #1371:
        // an `object` operand's truth test raises through
        // `foreign_len::emit_truthy`'s own failure edge, which branches
        // immediately, as for `and`/`or` and `if` below.)
        | MirExpr::Not(_)
        // #1211 (Part 3 of #1018): `and`/`or` is a truth test, a branch
        // and a join, all infallible. Each operand is emitted through
        // `emit_expr` inside its own arm, which guards that operand where
        // it is evaluated, so this node adds no edge of its own. (Part 6 of
        // #1371: an `object` operand's truth test raises through
        // `foreign_len::emit_truthy`'s own failure edge, and boxing a native
        // operand into an `object` node routes a packer `NULL` through
        // `foreign_fail::route_null`, both branching immediately.)
        | MirExpr::BoolOp { .. }
        // #1395: a conditional expression is a truth test, a branch and a
        // join, all infallible; `test` and each branch are guarded by their
        // own `emit_expr` where they are evaluated. (An `object` condition's
        // truth test raises through `foreign_len::emit_truthy`'s own
        // failure edge, as in an `if` statement.)
        | MirExpr::IfExp { .. }
        // #1212 (Part 4 of #1018): a chained comparison is compares,
        // branches and a join. Each operand is guarded by its own
        // `emit_expr`, and each link that can leave an exception pending
        // (an `int` operand, a dataclass `__eq__` call) is guarded inside
        // `compare_chain.rs` before its branch, so the node adds no edge of
        // its own.
        | MirExpr::CompareChain { .. }
        // #1116: `BufferLen` deliberately diverges from *both* its siblings
        // `ObjLen` and `BufferGet`, which are `true` above. A buffer's
        // length read is a plain load of the `len` word the `--ext` wrapper
        // already filled in: there is no index to range-check, no
        // `PyObject_Size` to call, and `pycc_rt_buffer_len` has no path that
        // sets D-173's pending state. Classifying it `true` would emit a
        // never-taken exception-check branch after every `len(b)`.
        | MirExpr::BufferLen { .. } => false,
    }
}

/// Stops expression/statement evaluation before another effect can be
/// committed when a Python exception is pending.
///
/// #638 (D-208): when `rt.exceptions.pending_int_releases` is non-empty, an
/// enclosing node (never this call site's own operand -- see each push
/// site's own comment) has already evaluated a fresh `Ty::Int` birth
/// reference that a sibling's evaluation, reached from here, might orphan by
/// raising before the enclosing node's own release call is textually
/// reached. In that case the exception edge is routed through an
/// intermediate `effect_exc_unwind` block that releases a *snapshot* of the
/// stack before branching to `exception_target`, instead of branching there
/// directly.
///
/// The `is_empty()` check below is a codegen-time (Rust-side) read of the
/// `RefCell`'s current length while walking the MIR tree -- it costs nothing
/// in the emitted binary either way, and it is what keeps this change from
/// affecting the D-084/D-140 nbody hot loop's own codegen: that loop's
/// arithmetic is entirely smallint, so it never pushes onto this stack, and
/// every guard site it reaches takes the empty-stack branch below, emitting
/// exactly the same two-block shape this function has always emitted. Do
/// not simplify this to unconditionally emit `effect_exc_unwind`: doing so
/// would add an extra empty block and an extra unconditional branch to
/// every guard site in the program, including every one in that hot loop,
/// which is precisely the throughput regression `bigint_rc.rs`'s own guard
/// design exists to avoid.
pub(super) fn guard_statement_effects<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    rt: &RtFns<'ctx>,
) {
    let exception_target = rt
        .exceptions
        .targets
        .borrow()
        .last()
        .copied()
        .expect("expression emission always has an installed exception target");
    let active = builder
        .build_call(rt.exception_active, &[], "effect_exc_active")
        .expect("build_call should not fail for exception_active")
        .try_as_basic_value()
        .expect_basic("pycc_rt_exception_active returns i8")
        .into_int_value();
    let has_exc = builder
        .build_int_compare(
            inkwell::IntPredicate::NE,
            active,
            context.i8_type().const_zero(),
            "effect_has_exc",
        )
        .expect("build_int_compare should not fail");
    let function = builder.get_insert_block().unwrap().get_parent().unwrap();
    let continuation = context.append_basic_block(function, "effect_exc_cont");
    // Part 1 of #1092: a held object temporary needs the unwind block
    // exactly as a held bigint does; both empty keeps the two-block shape.
    let nothing_pending = rt.exceptions.pending_int_releases.borrow().is_empty()
        && rt.exceptions.pending_object_releases.borrow().is_empty();
    if nothing_pending {
        builder
            .build_conditional_branch(has_exc, exception_target, continuation)
            .expect("build_conditional_branch should guard a statement effect");
        builder.position_at_end(continuation);
        return;
    }
    let unwind_bb = context.append_basic_block(function, "effect_exc_unwind");
    builder
        .build_conditional_branch(has_exc, unwind_bb, continuation)
        .expect("build_conditional_branch should guard a statement effect");
    builder.position_at_end(unwind_bb);
    jump_to_exception_target(context, builder, rt);
    builder.position_at_end(continuation);
}

/// Terminates the current block with an unconditional branch to the
/// innermost exception target, first releasing a snapshot of
/// `rt.exceptions.pending_int_releases` (#638, D-208) and of
/// `rt.exceptions.pending_object_releases` (Part 1 of #1092).
///
/// The one definition of that unwind: [`guard_statement_effects`] emits it
/// into its `effect_exc_unwind` block, and `foreign_fail.rs` emits it on a
/// failed foreign operation inside a function body (#1316), after the shim
/// has bridged CPython's exception into pycc's pending state. With an empty
/// stack it is the bare branch.
pub(super) fn jump_to_exception_target<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    rt: &RtFns<'ctx>,
) {
    let exception_target = rt
        .exceptions
        .targets
        .borrow()
        .last()
        .copied()
        .expect("expression emission always has an installed exception target");
    // Snapshot rather than borrow across block emission: nothing below
    // mutates the stack, but cloning up front keeps the borrow scoped to
    // this one line, matching this file's existing style for `targets`/
    // `reraise_values` snapshots elsewhere.
    let snapshot: Vec<IntValue<'ctx>> = rt.exceptions.pending_int_releases.borrow().clone();
    for word in snapshot {
        emit_bigint_refcount_call(context, builder, rt, word, BigIntRefcount::Release);
    }
    crate::object_release::release_pending(builder, rt);
    builder
        .build_unconditional_branch(exception_target)
        .expect("build_unconditional_branch should not fail for a fresh unwind block");
}

#[allow(clippy::too_many_arguments)]
pub(super) fn emit_try<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    module: &inkwell::module::Module<'ctx>,
    rt: &RtFns<'ctx>,
    user_functions: &HashMap<&str, UserFunction<'ctx>>,
    locals: &mut HashMap<String, StorageSlot<'ctx>>,
    body: &[MirStmt],
    handlers: &[pycc_mir::MirExceptHandler],
    orelse: &[MirStmt],
    finalbody: &[MirStmt],
    expected_return_ty: pycc_mir::Ty,
    finally_stack: &mut Vec<FinallyTarget<'ctx>>,
) -> Result<(), String> {
    let function = builder.get_insert_block().unwrap().get_parent().unwrap();

    // Allocate blocks for the try body, handler dispatch, else,
    // finally, and merge (after-try).
    let try_body_bb = context.append_basic_block(function, "try_body");
    let handler_dispatch_bb = context.append_basic_block(function, "try_handler_dispatch");
    let else_bb = context.append_basic_block(function, "try_else");
    let finally_bb = context.append_basic_block(function, "try_finally");
    let after_bb = context.append_basic_block(function, "try_after");

    // #382 (PR-22 Part 2): If there is a finally body, set up a
    // `FinallyTarget` so `return` statements inside the try body,
    // handlers, and else body route through the finally block
    // before completing. The `is_returning` flag distinguishes a
    // return (which should emit `ret` after finally) from normal
    // completion (which branches to `after_bb`).
    let has_finally = !finalbody.is_empty();
    // Saved fields from the FinallyTarget, needed after the finally
    // body runs (the target itself is popped before emitting the
    // finally body to prevent self-interception).
    let saved_is_returning: Option<PointerValue<'ctx>>;
    let saved_ret_slot: Option<PointerValue<'ctx>>;
    if has_finally {
        let is_returning = builder
            .build_alloca(context.i8_type(), "try_is_returning")
            .expect("build_alloca should not fail for is_returning flag");
        builder
            .build_store(is_returning, context.i8_type().const_zero())
            .expect("build_store should not fail for is_returning init");
        let ret_slot = if expected_return_ty == pycc_mir::Ty::None {
            None
        } else {
            let slot = builder
                .build_alloca(
                    ty_to_basic_type(context, expected_return_ty.clone()),
                    "try_ret_slot",
                )
                .expect("build_alloca should not fail for ret_slot");
            Some(slot)
        };
        saved_is_returning = Some(is_returning);
        saved_ret_slot = ret_slot;
        finally_stack.push(FinallyTarget {
            finally_bb,
            ret_slot,
            is_returning,
        });
    } else {
        saved_is_returning = None;
        saved_ret_slot = None;
    }

    // Enter the try body.
    builder
        .build_unconditional_branch(try_body_bb)
        .expect("build_unconditional_branch should not fail entering try body");
    builder.position_at_end(try_body_bb);

    // Emit the try body. After it completes, check for an active
    // exception and branch to handler dispatch or else.
    rt.exceptions.targets.borrow_mut().push(handler_dispatch_bb);
    emit_body(
        context,
        builder,
        module,
        rt,
        user_functions,
        locals,
        body,
        expected_return_ty.clone(),
        finally_stack,
    )?;
    rt.exceptions.targets.borrow_mut().pop();
    // #382: A `raise` inside the body terminates the block with
    // `unreachable`. Erase it so the exception check can run here.
    erase_unreachable_if_present(builder);
    let body_falls_through = builder
        .get_insert_block()
        .unwrap()
        .get_terminator()
        .is_none();
    if body_falls_through {
        let active = builder
            .build_call(rt.exception_active, &[], "try_body_exc_active")
            .expect("build_call should not fail for exception_active")
            .try_as_basic_value()
            .expect_basic("pycc_rt_exception_active returns i8")
            .into_int_value();
        let has_exc = builder
            .build_int_compare(
                inkwell::IntPredicate::NE,
                active,
                context.i8_type().const_zero(),
                "try_body_has_exc",
            )
            .expect("build_int_compare should not fail");
        builder
            .build_conditional_branch(has_exc, handler_dispatch_bb, else_bb)
            .expect("build_conditional_branch should not fail");
    }

    // Handler dispatch: check each handler in order.
    builder.position_at_end(handler_dispatch_bb);
    if handlers.is_empty() {
        // No handlers — branch to finally (exception remains active,
        // which will cause the finally to re-check and propagate).
        builder
            .build_unconditional_branch(finally_bb)
            .expect("build_unconditional_branch should not fail");
    } else {
        // Build separate dispatch blocks and handler body blocks.
        // The dispatch chain checks type_matches in order; each
        // dispatch block branches to its handler body (match) or
        // the next dispatch block (no match).
        let exc_val = builder
            .build_call(rt.exception_value, &[], "try_exc_val")
            .expect("build_call should not fail for exception_value")
            .try_as_basic_value()
            .expect_basic("pycc_rt_exception_value returns a pointer")
            .into_pointer_value();
        let mut dispatch_bbs: Vec<inkwell::basic_block::BasicBlock> = Vec::new();
        let mut handler_body_bbs: Vec<inkwell::basic_block::BasicBlock> = Vec::new();
        for i in 0..handlers.len() {
            dispatch_bbs.push(context.append_basic_block(function, &format!("try_dispatch_{i}")));
            handler_body_bbs
                .push(context.append_basic_block(function, &format!("try_handler_{i}")));
        }
        let no_match_bb = context.append_basic_block(function, "try_no_match");

        // Entry into dispatch chain from handler_dispatch_bb.
        builder
            .build_unconditional_branch(dispatch_bbs[0])
            .expect("build_unconditional_branch should not fail");

        // Dispatch chain: for each handler, check type_matches.
        for (i, handler) in handlers.iter().enumerate() {
            let next_bb = if i + 1 < handlers.len() {
                dispatch_bbs[i + 1]
            } else {
                no_match_bb
            };
            builder.position_at_end(dispatch_bbs[i]);
            let matches = if let Some(tags) = handler.exc_type_tag.as_deref() {
                // Part 2 of #541 (D-189): a handler naming a class accepts
                // that class and every user-defined subclass of it, each of
                // which carries its own tag, so the test is an OR over the
                // whole set. A single-tag handler -- every builtin one --
                // emits exactly the call it emitted before, with no `or`.
                let mut accumulated: Option<inkwell::values::IntValue<'ctx>> = None;
                for tag in tags {
                    let tag_val = context.i8_type().const_int(u64::from(*tag), false);
                    let one = builder
                        .build_call(
                            rt.exception_type_matches,
                            &[exc_val.into(), tag_val.into()],
                            "exc_matches",
                        )
                        .expect("build_call should not fail for exception_type_matches")
                        .try_as_basic_value()
                        .expect_basic("pycc_rt_exception_type_matches returns i8")
                        .into_int_value();
                    accumulated = Some(match accumulated {
                        Some(previous) => builder
                            .build_or(previous, one, "exc_matches_any")
                            .expect("build_or should not fail"),
                        None => one,
                    });
                }
                accumulated.expect("pycc_mir never emits an empty handler tag set")
            } else {
                // Bare `except:` — always matches.
                context.i8_type().const_int(1, false)
            };
            let is_match = builder
                .build_int_compare(
                    inkwell::IntPredicate::NE,
                    matches,
                    context.i8_type().const_zero(),
                    "is_match",
                )
                .expect("build_int_compare should not fail");
            builder
                .build_conditional_branch(is_match, handler_body_bbs[i], next_bb)
                .expect("build_conditional_branch should not fail");
        }

        // No handler matched — branch to finally (exception stays
        // active, will propagate after finally).
        builder.position_at_end(no_match_bb);
        builder
            .build_unconditional_branch(finally_bb)
            .expect("build_unconditional_branch should not fail");

        // Emit each handler body. Save the current exception value
        // to a handler-local slot, then clear the exception state so the
        // handler body runs with no active exception. A bare `raise`
        // (Reraise) inside the handler loads the saved value and
        // re-raises it. After the handler body, if it completes
        // normally (no active exception), just branch to finally.
        for (i, handler) in handlers.iter().enumerate() {
            builder.position_at_end(handler_body_bbs[i]);
            // Save the current exception value and clear the state.
            let exc_val = builder
                .build_call(rt.exception_value, &[], "handler_exc_val")
                .expect("build_call should not fail for exception_value")
                .try_as_basic_value()
                .expect_basic("pycc_rt_exception_value returns a pointer")
                .into_pointer_value();
            let saved_exc = builder
                .build_alloca(
                    context.ptr_type(inkwell::AddressSpace::default()),
                    "handler_saved_exc",
                )
                .expect("build_alloca should not fail for a handler exception slot");
            builder
                .build_store(saved_exc, exc_val)
                .expect("build_store should not fail for a handler exception slot");
            builder
                .build_call(rt.exception_clear, &[], "")
                .expect("build_call should not fail for exception_clear");
            // #382: If the handler has an `as` binding (e.g.
            // `except ValueError as e:`), allocate a local slot
            // for it and store the exception value there so the
            // handler body can reference `e`. The slot is a
            // pointer-typed alloca (all exception objects are
            // pointer-represented, like class instances).
            if let Some(binding_name) = &handler.binding_name {
                let exc_slot = builder
                    .build_alloca(
                        context.ptr_type(inkwell::AddressSpace::default()),
                        binding_name,
                    )
                    .expect("build_alloca should not fail for an exception binding slot");
                builder
                    .build_store(exc_slot, exc_val)
                    .expect("build_store should not fail for an exception binding");
                locals.insert(
                    binding_name.clone(),
                    StorageSlot {
                        ptr: exc_slot,
                        ty: handler
                            .binding_ty
                            .clone()
                            .expect("an exception binding always carries its static type"),
                        initialized: None,
                    },
                );
            }
            rt.exceptions.reraise_values.borrow_mut().push(saved_exc);
            rt.exceptions.targets.borrow_mut().push(finally_bb);
            emit_body(
                context,
                builder,
                module,
                rt,
                user_functions,
                locals,
                &handler.body,
                expected_return_ty.clone(),
                finally_stack,
            )?;
            rt.exceptions.targets.borrow_mut().pop();
            rt.exceptions.reraise_values.borrow_mut().pop();
            // #382: A `raise`/`reraise` inside the handler body
            // terminates the block with `unreachable`. Erase it so
            // the handler-completion check can run here.
            erase_unreachable_if_present(builder);
            let handler_falls_through = builder
                .get_insert_block()
                .unwrap()
                .get_terminator()
                .is_none();
            if handler_falls_through {
                // Handler completed normally — exception was already
                // cleared before the handler body. Just branch to
                // finally.
                builder
                    .build_unconditional_branch(finally_bb)
                    .expect("build_unconditional_branch should not fail");
            }
        }
    }

    // Else body: runs only if no exception was raised.
    builder.position_at_end(else_bb);
    if orelse.is_empty() {
        builder
            .build_unconditional_branch(finally_bb)
            .expect("build_unconditional_branch should not fail");
    } else {
        rt.exceptions.targets.borrow_mut().push(finally_bb);
        emit_body(
            context,
            builder,
            module,
            rt,
            user_functions,
            locals,
            orelse,
            expected_return_ty.clone(),
            finally_stack,
        )?;
        rt.exceptions.targets.borrow_mut().pop();
        // #382: A `raise` inside the else body terminates the
        // block with `unreachable`. Erase it so the
        // else-completion check can run here.
        erase_unreachable_if_present(builder);
        let else_falls_through = builder
            .get_insert_block()
            .unwrap()
            .get_terminator()
            .is_none();
        if else_falls_through {
            builder
                .build_unconditional_branch(finally_bb)
                .expect("build_unconditional_branch should not fail");
        }
    }

    // Pop the FinallyTarget before emitting the finally body —
    // a `return` inside the finally body should NOT be intercepted
    // by this same finally (it would loop).
    if has_finally {
        finally_stack.pop();
    }

    // Finally body: always runs. After finally, check if a return
    // was intercepted (is_returning flag) or branch to after_bb
    // for normal completion. The structured-statement guard at
    // after_bb propagates any pending exception.
    builder.position_at_end(finally_bb);
    let finally_exception_bb = context.append_basic_block(function, "try_finally_exception");
    let pending_exception = if finalbody.is_empty() {
        None
    } else {
        // A pending exception must not prevent `finally` from
        // executing. Preserve it in SSA values and clear the
        // pending state while the final body runs. Normal
        // completion restores it; a new exception or a return from
        // `finally` deliberately replaces/suppresses it.
        let active = builder
            .build_call(rt.exception_active, &[], "finally_pending_active")
            .expect("build_call should not fail for exception_active")
            .try_as_basic_value()
            .expect_basic("pycc_rt_exception_active returns i8")
            .into_int_value();
        let value = builder
            .build_call(rt.exception_value, &[], "finally_pending_value")
            .expect("build_call should not fail for exception_value")
            .try_as_basic_value()
            .expect_basic("pycc_rt_exception_value returns a pointer")
            .into_pointer_value();
        builder
            .build_call(rt.exception_clear, &[], "")
            .expect("build_call should not fail for exception_clear");
        Some((active, value))
    };
    if finalbody.is_empty() {
        // No finally body — just check is_returning / branch.
    } else {
        rt.exceptions
            .targets
            .borrow_mut()
            .push(finally_exception_bb);
        emit_body(
            context,
            builder,
            module,
            rt,
            user_functions,
            locals,
            finalbody,
            expected_return_ty.clone(),
            finally_stack,
        )?;
        rt.exceptions.targets.borrow_mut().pop();
    }
    // #382: A `raise` inside the finally body terminates the
    // block with `unreachable`. Erase it so the
    // finally-completion check can run here.
    erase_unreachable_if_present(builder);
    let finally_falls_through = builder
        .get_insert_block()
        .unwrap()
        .get_terminator()
        .is_none();
    if finally_falls_through {
        if let Some((pending_active, pending_value)) = pending_exception {
            let had_pending = builder
                .build_int_compare(
                    inkwell::IntPredicate::NE,
                    pending_active,
                    context.i8_type().const_zero(),
                    "finally_had_pending",
                )
                .expect("build_int_compare should not fail");
            let restore_bb = context.append_basic_block(function, "try_finally_restore_exception");
            let restored_bb =
                context.append_basic_block(function, "try_finally_exception_restored");
            builder
                .build_conditional_branch(had_pending, restore_bb, restored_bb)
                .expect("build_conditional_branch should not fail");
            builder.position_at_end(restore_bb);
            builder
                .build_call(rt.exception_raise, &[pending_value.into()], "")
                .expect("build_call should not fail for exception_raise");
            builder
                .build_unconditional_branch(restored_bb)
                .expect("build_unconditional_branch should not fail");
            builder.position_at_end(restored_bb);
        }
        if has_finally {
            // Check if a return was intercepted: if so, load the
            // return value and emit `ret` (or propagate to an
            // enclosing finally). Otherwise, branch to after_bb.
            let is_returning = saved_is_returning.unwrap();
            let ret_slot = saved_ret_slot;
            let flag_val = builder
                .build_load(context.i8_type(), is_returning, "finally_ret_flag")
                .expect("build_load should not fail for is_returning flag")
                .into_int_value();
            let is_ret = builder
                .build_int_compare(
                    inkwell::IntPredicate::NE,
                    flag_val,
                    context.i8_type().const_zero(),
                    "finally_is_ret",
                )
                .expect("build_int_compare should not fail");
            let ret_bb = context.append_basic_block(function, "try_finally_ret");
            builder
                .build_conditional_branch(is_ret, ret_bb, after_bb)
                .expect("build_conditional_branch should not fail");
            // Return block: load the saved return value and emit
            // `ret`, or propagate to an enclosing finally.
            builder.position_at_end(ret_bb);
            if let Some(slot) = ret_slot {
                let ret_val = builder
                    .build_load(
                        ty_to_basic_type(context, expected_return_ty.clone()),
                        slot,
                        "finally_ret_val",
                    )
                    .expect("build_load should not fail for ret_slot");
                if let Some(outer) = finally_stack.last_mut() {
                    // Propagate to the enclosing finally: store the
                    // return value to the outer's ret_slot, set its
                    // is_returning flag, and branch to its finally_bb.
                    let outer_slot = outer
                        .ret_slot
                        .expect("nested finally in the same non-None function has a return slot");
                    builder
                        .build_store(outer_slot, ret_val)
                        .expect("build_store should not fail for outer ret_slot");
                    builder
                        .build_store(outer.is_returning, context.i8_type().const_int(1, false))
                        .expect("build_store should not fail for outer is_returning");
                    builder
                        .build_unconditional_branch(outer.finally_bb)
                        .expect("build_unconditional_branch should not fail");
                } else {
                    builder
                        .build_return(Some(&ret_val))
                        .expect("build_return should not fail for a finally-routed return");
                }
            } else {
                // Void function (Ty::None) or top-level code (main
                // returns i64). `ret_slot` is None because
                // `expected_return_ty` is `Ty::None`. For a void
                // function, emit `ret void`. For top-level `main`
                // (which returns i64), this block is dead code (no
                // `return` is allowed at top level), so emit
                // `unreachable` to satisfy the verifier.
                if let Some(outer) = finally_stack.last_mut() {
                    builder
                        .build_store(outer.is_returning, context.i8_type().const_int(1, false))
                        .expect("build_store should not fail for outer is_returning");
                    builder
                        .build_unconditional_branch(outer.finally_bb)
                        .expect("build_unconditional_branch should not fail");
                } else if function.get_type().get_return_type().is_none() {
                    // LLVM void function — emit `ret void`.
                    builder
                        .build_return(None)
                        .expect("build_return should not fail for a finally-routed void return");
                } else {
                    // Non-void LLVM function (e.g. `main` returns
                    // i64) with no `ret_slot` — dead code (no
                    // `return` was intercepted). Emit
                    // `unreachable` to satisfy the verifier.
                    builder
                        .build_unreachable()
                        .expect("build_unreachable should not fail for dead ret_bb");
                }
            }
        } else {
            // No finally body — branch to after_bb unconditionally.
            // The structured-statement guard at after_bb propagates any
            // pending exception before the enclosing suite can continue.
            builder
                .build_unconditional_branch(after_bb)
                .expect("build_unconditional_branch should not fail");
        }
    }

    // An exception raised by `finally` overrides an intercepted
    // return. It keeps the runtime exception active while skipping
    // the return-flag dispatch.
    builder.position_at_end(finally_exception_bb);
    builder
        .build_unconditional_branch(after_bb)
        .expect("build_unconditional_branch should propagate a finally exception");

    builder.position_at_end(after_bb);
    // A locally unmatched exception (or one restored after `finally`) must
    // leave this structured statement immediately. Expression nodes already
    // guard their own raising operations; this is the corresponding boundary
    // for a complete try statement.
    guard_statement_effects(context, builder, rt);
    Ok(())
}

// The fixed builtin runtime type tag for the `ExceptionGroup` class (Part 3
// of #382, #542, PEP 654, D-202) is `pycc_hir::exception::
// EXCEPTION_GROUP_TYPE_TAG`, re-exported here through `pycc_mir` and
// *derived* from `ExceptionGroup`'s position in `BUILTIN_EXCEPTION_CLASSES`
// rather than hand-maintained as a separate literal (D-194's derivation
// discipline). `emit_try_star` passes this tag (and the class name below) to
// `pycc_rt_exception_group_partition` whenever it needs to build a fresh
// reconstructed subgroup, regardless of the original raised object's own
// dynamic class -- a deliberate D-202 simplification: codegen does not track
// a raised group's polymorphic subclass identity through partitioning, so
// every subgroup an `except*` clause binds is reported as a plain
// `ExceptionGroup`, never the original (possibly user-defined)
// `BaseExceptionGroup` subclass name.

/// Emits `try`/`except*`/`else`/`finally` (Part 3 of #382, #542, PEP 654).
///
/// Structurally this mirrors [`emit_try`] almost exactly -- the same five
/// basic blocks, the same `FinallyTarget`/`finally`/`else`/return-routing
/// machinery, reused verbatim below. The two differ only in the handler
/// dispatch section: `Try`'s handlers are mutually exclusive (first
/// `pycc_rt_exception_type_matches` match wins, the rest are skipped
/// entirely), whereas PEP 654 requires every `except*` clause to run against
/// whatever the raised exception's *previous* clauses left unmatched, since
/// more than one clause may claim a member out of the same raised group. So
/// instead of a boolean `type_matches` test, each clause here calls
/// `pycc_rt_exception_group_partition` against a `current_group` value
/// threaded from one clause to the next (the runtime's own "remaining
/// members" output), runs its body on whatever it matched (if anything), and
/// after the last clause reraises whatever is still unmatched -- PEP 654
/// propagates a leftover remainder rather than silently discarding it.
#[allow(clippy::too_many_arguments)]
pub(super) fn emit_try_star<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    module: &inkwell::module::Module<'ctx>,
    rt: &RtFns<'ctx>,
    user_functions: &HashMap<&str, UserFunction<'ctx>>,
    locals: &mut HashMap<String, StorageSlot<'ctx>>,
    body: &[MirStmt],
    handlers: &[pycc_mir::MirExceptHandler],
    orelse: &[MirStmt],
    finalbody: &[MirStmt],
    expected_return_ty: pycc_mir::Ty,
    finally_stack: &mut Vec<FinallyTarget<'ctx>>,
) -> Result<(), String> {
    let function = builder.get_insert_block().unwrap().get_parent().unwrap();
    let ptr_type = context.ptr_type(inkwell::AddressSpace::default());

    let try_body_bb = context.append_basic_block(function, "trystar_body");
    let handler_dispatch_bb = context.append_basic_block(function, "trystar_handler_dispatch");
    let else_bb = context.append_basic_block(function, "trystar_else");
    let finally_bb = context.append_basic_block(function, "trystar_finally");
    let after_bb = context.append_basic_block(function, "trystar_after");

    let has_finally = !finalbody.is_empty();
    let saved_is_returning: Option<PointerValue<'ctx>>;
    let saved_ret_slot: Option<PointerValue<'ctx>>;
    if has_finally {
        let is_returning = builder
            .build_alloca(context.i8_type(), "trystar_is_returning")
            .expect("build_alloca should not fail for is_returning flag");
        builder
            .build_store(is_returning, context.i8_type().const_zero())
            .expect("build_store should not fail for is_returning init");
        let ret_slot = if expected_return_ty == pycc_mir::Ty::None {
            None
        } else {
            let slot = builder
                .build_alloca(
                    ty_to_basic_type(context, expected_return_ty.clone()),
                    "trystar_ret_slot",
                )
                .expect("build_alloca should not fail for ret_slot");
            Some(slot)
        };
        saved_is_returning = Some(is_returning);
        saved_ret_slot = ret_slot;
        finally_stack.push(FinallyTarget {
            finally_bb,
            ret_slot,
            is_returning,
        });
    } else {
        saved_is_returning = None;
        saved_ret_slot = None;
    }

    builder
        .build_unconditional_branch(try_body_bb)
        .expect("build_unconditional_branch should not fail entering try* body");
    builder.position_at_end(try_body_bb);

    rt.exceptions.targets.borrow_mut().push(handler_dispatch_bb);
    emit_body(
        context,
        builder,
        module,
        rt,
        user_functions,
        locals,
        body,
        expected_return_ty.clone(),
        finally_stack,
    )?;
    rt.exceptions.targets.borrow_mut().pop();
    erase_unreachable_if_present(builder);
    let body_falls_through = builder
        .get_insert_block()
        .unwrap()
        .get_terminator()
        .is_none();
    if body_falls_through {
        let active = builder
            .build_call(rt.exception_active, &[], "trystar_body_exc_active")
            .expect("build_call should not fail for exception_active")
            .try_as_basic_value()
            .expect_basic("pycc_rt_exception_active returns i8")
            .into_int_value();
        let has_exc = builder
            .build_int_compare(
                inkwell::IntPredicate::NE,
                active,
                context.i8_type().const_zero(),
                "trystar_body_has_exc",
            )
            .expect("build_int_compare should not fail");
        builder
            .build_conditional_branch(has_exc, handler_dispatch_bb, else_bb)
            .expect("build_conditional_branch should not fail");
    }

    // Handler dispatch: `except*` partitions the raised group across every
    // clause in source order (PEP 654), unlike `Try`'s mutually exclusive
    // first-match-wins chain. `pycc_hir::stmt::lower_stmt`'s own `Stmt::Try`
    // arm never lowers a bare `try`/`finally` (no `except*` clauses at all)
    // to `HirStmt::TryStar` -- only a real `except*` clause sets `is_star` --
    // so unlike `Try`, which codegens a real `handlers.is_empty()` branch for
    // exactly that case, there is no reachable empty-`handlers` case to
    // branch on here.
    builder.position_at_end(handler_dispatch_bb);
    let current_group_slot = builder
        .build_alloca(ptr_type, "trystar_current_group")
        .expect("build_alloca should not fail for the trystar current-group slot");
    {
        let exc_val = builder
            .build_call(rt.exception_value, &[], "trystar_exc_val")
            .expect("build_call should not fail for exception_value")
            .try_as_basic_value()
            .expect_basic("pycc_rt_exception_value returns a pointer")
            .into_pointer_value();
        // The raised value becomes this dispatch chain's own to manage --
        // clear the runtime's pending state immediately, exactly once,
        // before any clause's body can run. No clause below re-sets the
        // pending state itself; only a clause body that raises a brand-new
        // exception, or the final unmatched remainder reraised after the
        // last clause, does.
        builder
            .build_call(rt.exception_clear, &[], "")
            .expect("build_call should not fail for exception_clear");
        builder
            .build_store(current_group_slot, exc_val)
            .expect("build_store should not fail for the trystar current-group slot");
    }

    let mut dispatch_bbs: Vec<inkwell::basic_block::BasicBlock> = Vec::new();
    let mut handler_body_bbs: Vec<inkwell::basic_block::BasicBlock> = Vec::new();
    for i in 0..handlers.len() {
        dispatch_bbs.push(context.append_basic_block(function, &format!("trystar_dispatch_{i}")));
        handler_body_bbs
            .push(context.append_basic_block(function, &format!("trystar_handler_{i}")));
    }
    let reraise_remainder_bb = context.append_basic_block(function, "trystar_reraise_remainder");

    builder
        .build_unconditional_branch(dispatch_bbs[0])
        .expect("build_unconditional_branch should not fail");

    let (group_name_ptr, group_name_len) =
        emit_str_bytes_constant(context, module, "ExceptionGroup", "exc_class_name");
    let group_type_tag = context
        .i8_type()
        .const_int(u64::from(EXCEPTION_GROUP_TYPE_TAG), false);

    for (i, handler) in handlers.iter().enumerate() {
        let next_bb = if i + 1 < handlers.len() {
            dispatch_bbs[i + 1]
        } else {
            reraise_remainder_bb
        };

        builder.position_at_end(dispatch_bbs[i]);
        let group_ptr = builder
            .build_load(ptr_type, current_group_slot, "trystar_group")
            .expect("build_load should not fail for the trystar current-group slot")
            .into_pointer_value();

        // An earlier, broader clause (e.g. one matching the universal
        // `EXCEPTION_TYPE_EXCEPTION` tag) can already have consumed every
        // remaining member, in which case `rest_ptr` from that clause's own
        // partition call -- stored into `current_group_slot` below -- is
        // null (`build_group_or_null` returns null for an empty half).
        // `pycc_rt_exception_group_partition`'s safety contract requires a
        // non-null `group`, so this clause must not be dispatched at all
        // when nothing is left to test; skip straight to the next clause
        // (or `reraise_remainder_bb`) instead of calling the runtime
        // function with a null pointer.
        if i > 0 {
            let group_is_null = builder
                .build_is_null(group_ptr, "trystar_group_is_null")
                .expect("build_is_null should not fail for a pointer comparison");
            let dispatch_body_bb =
                context.append_basic_block(function, &format!("trystar_dispatch_body_{i}"));
            builder
                .build_conditional_branch(group_is_null, next_bb, dispatch_body_bb)
                .expect("build_conditional_branch should not fail");
            builder.position_at_end(dispatch_body_bb);
        }

        // `pycc_mir` always resolves an `except*` clause's tag set --
        // `pycc_hir::stmt::lower_stmt`'s own `Stmt::Try` arm rejects a
        // typeless `except*:` at parse time before lowering ever runs (see
        // that module's `lower_try_star_bare_except_star_is_rejected_at_
        // parse_time` test) -- so this is never `Try`'s own bare-`except:`
        // catch-all `None` case.
        let tags = handler
            .exc_type_tag
            .as_deref()
            .expect("an except* handler always carries a resolved tag set");
        let tags_array_ty = context.i8_type().array_type(tags.len() as u32);
        let tags_alloca = builder
            .build_alloca(tags_array_ty, &format!("trystar_tags_{i}"))
            .expect("build_alloca should not fail for a trystar tag array");
        for (j, tag) in tags.iter().enumerate() {
            let slot = unsafe {
                builder
                    .build_gep(
                        context.i8_type(),
                        tags_alloca,
                        &[context.i64_type().const_int(j as u64, false)],
                        &format!("trystar_tag_{i}_{j}"),
                    )
                    .expect("build_gep should not fail for a trystar tag slot")
            };
            builder
                .build_store(slot, context.i8_type().const_int(u64::from(*tag), false))
                .expect("build_store should not fail for a trystar tag slot");
        }
        let tags_len = context.i64_type().const_int(tags.len() as u64, false);

        let matched_slot = builder
            .build_alloca(ptr_type, &format!("trystar_matched_{i}"))
            .expect("build_alloca should not fail for a trystar matched-out slot");
        let rest_slot = builder
            .build_alloca(ptr_type, &format!("trystar_rest_{i}"))
            .expect("build_alloca should not fail for a trystar rest-out slot");

        builder
            .build_call(
                rt.exception_group_partition,
                &[
                    group_ptr.into(),
                    tags_alloca.into(),
                    tags_len.into(),
                    group_type_tag.into(),
                    group_name_ptr.into(),
                    group_name_len.into(),
                    matched_slot.into(),
                    rest_slot.into(),
                ],
                "",
            )
            .expect("build_call should not fail for exception_group_partition");
        let matched_ptr = builder
            .build_load(ptr_type, matched_slot, "trystar_matched_ptr")
            .expect("build_load should not fail for a trystar matched-out slot")
            .into_pointer_value();
        let rest_ptr = builder
            .build_load(ptr_type, rest_slot, "trystar_rest_ptr")
            .expect("build_load should not fail for a trystar rest-out slot")
            .into_pointer_value();
        builder
            .build_store(current_group_slot, rest_ptr)
            .expect("build_store should not fail for the trystar current-group slot");

        let no_match = builder
            .build_is_null(matched_ptr, "trystar_no_match")
            .expect("build_is_null should not fail for a pointer comparison");
        builder
            .build_conditional_branch(no_match, next_bb, handler_body_bbs[i])
            .expect("build_conditional_branch should not fail");

        builder.position_at_end(handler_body_bbs[i]);
        if let Some(binding_name) = &handler.binding_name {
            let exc_slot = builder
                .build_alloca(ptr_type, binding_name)
                .expect("build_alloca should not fail for an except* binding slot");
            builder
                .build_store(exc_slot, matched_ptr)
                .expect("build_store should not fail for an except* binding");
            locals.insert(
                binding_name.clone(),
                StorageSlot {
                    ptr: exc_slot,
                    ty: handler
                        .binding_ty
                        .clone()
                        .expect("an except* binding always carries its static type"),
                    initialized: None,
                },
            );
        }
        // A bare `raise` inside an `except*` body reraises the exact
        // matched subgroup this clause was handed, mirroring `Try`'s own
        // `reraise_values` mechanism.
        let saved_exc = builder
            .build_alloca(ptr_type, &format!("trystar_saved_exc_{i}"))
            .expect("build_alloca should not fail for a trystar reraise slot");
        builder
            .build_store(saved_exc, matched_ptr)
            .expect("build_store should not fail for a trystar reraise slot");
        rt.exceptions.reraise_values.borrow_mut().push(saved_exc);
        // D-202 simplification: a *new* exception raised from inside an
        // `except*` body (as opposed to a bare `raise` of the matched
        // subgroup above) is not folded back into this statement's
        // still-unhandled remainder the way CPython's own PEP 654 "derived
        // exception group" chaining would -- it propagates directly to
        // `finally_bb`, exactly like `Try`'s own handler bodies,
        // abandoning any later `except*` clauses and any remaining
        // unmatched members. This keeps `except*` codegen's control flow a
        // straight-line extension of `Try`'s existing check-and-branch
        // model rather than requiring a second exception-group merge step
        // with no `Try` precedent.
        rt.exceptions.targets.borrow_mut().push(finally_bb);
        emit_body(
            context,
            builder,
            module,
            rt,
            user_functions,
            locals,
            &handler.body,
            expected_return_ty.clone(),
            finally_stack,
        )?;
        rt.exceptions.targets.borrow_mut().pop();
        rt.exceptions.reraise_values.borrow_mut().pop();
        erase_unreachable_if_present(builder);
        let handler_falls_through = builder
            .get_insert_block()
            .unwrap()
            .get_terminator()
            .is_none();
        if handler_falls_through {
            builder
                .build_unconditional_branch(next_bb)
                .expect("build_unconditional_branch should not fail");
        }
    }

    // Every clause has now run (or been skipped). Reraise whatever
    // remains unmatched, if anything -- PEP 654 propagates a leftover
    // remainder rather than silently discarding it.
    builder.position_at_end(reraise_remainder_bb);
    {
        let remainder = builder
            .build_load(ptr_type, current_group_slot, "trystar_remainder")
            .expect("build_load should not fail for the trystar current-group slot")
            .into_pointer_value();
        let no_remainder = builder
            .build_is_null(remainder, "trystar_remainder_is_null")
            .expect("build_is_null should not fail for a pointer comparison");
        let reraise_bb = context.append_basic_block(function, "trystar_reraise");
        builder
            .build_conditional_branch(no_remainder, finally_bb, reraise_bb)
            .expect("build_conditional_branch should not fail");
        builder.position_at_end(reraise_bb);
        builder
            .build_call(rt.exception_raise, &[remainder.into()], "")
            .expect("build_call should not fail for exception_raise");
        builder
            .build_unconditional_branch(finally_bb)
            .expect("build_unconditional_branch should not fail");
    }

    // Else body: runs only if no exception was raised (identical to
    // `Try`'s own `else_bb`).
    builder.position_at_end(else_bb);
    if orelse.is_empty() {
        builder
            .build_unconditional_branch(finally_bb)
            .expect("build_unconditional_branch should not fail");
    } else {
        rt.exceptions.targets.borrow_mut().push(finally_bb);
        emit_body(
            context,
            builder,
            module,
            rt,
            user_functions,
            locals,
            orelse,
            expected_return_ty.clone(),
            finally_stack,
        )?;
        rt.exceptions.targets.borrow_mut().pop();
        erase_unreachable_if_present(builder);
        let else_falls_through = builder
            .get_insert_block()
            .unwrap()
            .get_terminator()
            .is_none();
        if else_falls_through {
            builder
                .build_unconditional_branch(finally_bb)
                .expect("build_unconditional_branch should not fail");
        }
    }

    if has_finally {
        finally_stack.pop();
    }

    builder.position_at_end(finally_bb);
    let finally_exception_bb = context.append_basic_block(function, "trystar_finally_exception");
    let pending_exception = if finalbody.is_empty() {
        None
    } else {
        let active = builder
            .build_call(rt.exception_active, &[], "trystar_finally_pending_active")
            .expect("build_call should not fail for exception_active")
            .try_as_basic_value()
            .expect_basic("pycc_rt_exception_active returns i8")
            .into_int_value();
        let value = builder
            .build_call(rt.exception_value, &[], "trystar_finally_pending_value")
            .expect("build_call should not fail for exception_value")
            .try_as_basic_value()
            .expect_basic("pycc_rt_exception_value returns a pointer")
            .into_pointer_value();
        builder
            .build_call(rt.exception_clear, &[], "")
            .expect("build_call should not fail for exception_clear");
        Some((active, value))
    };
    if finalbody.is_empty() {
        // No finally body — just check is_returning / branch.
    } else {
        rt.exceptions
            .targets
            .borrow_mut()
            .push(finally_exception_bb);
        emit_body(
            context,
            builder,
            module,
            rt,
            user_functions,
            locals,
            finalbody,
            expected_return_ty.clone(),
            finally_stack,
        )?;
        rt.exceptions.targets.borrow_mut().pop();
    }
    erase_unreachable_if_present(builder);
    let finally_falls_through = builder
        .get_insert_block()
        .unwrap()
        .get_terminator()
        .is_none();
    if finally_falls_through {
        if let Some((pending_active, pending_value)) = pending_exception {
            let had_pending = builder
                .build_int_compare(
                    inkwell::IntPredicate::NE,
                    pending_active,
                    context.i8_type().const_zero(),
                    "trystar_finally_had_pending",
                )
                .expect("build_int_compare should not fail");
            let restore_bb =
                context.append_basic_block(function, "trystar_finally_restore_exception");
            let restored_bb =
                context.append_basic_block(function, "trystar_finally_exception_restored");
            builder
                .build_conditional_branch(had_pending, restore_bb, restored_bb)
                .expect("build_conditional_branch should not fail");
            builder.position_at_end(restore_bb);
            builder
                .build_call(rt.exception_raise, &[pending_value.into()], "")
                .expect("build_call should not fail for exception_raise");
            builder
                .build_unconditional_branch(restored_bb)
                .expect("build_unconditional_branch should not fail");
            builder.position_at_end(restored_bb);
        }
        if has_finally {
            let is_returning = saved_is_returning.unwrap();
            let ret_slot = saved_ret_slot;
            let flag_val = builder
                .build_load(context.i8_type(), is_returning, "trystar_finally_ret_flag")
                .expect("build_load should not fail for is_returning flag")
                .into_int_value();
            let is_ret = builder
                .build_int_compare(
                    inkwell::IntPredicate::NE,
                    flag_val,
                    context.i8_type().const_zero(),
                    "trystar_finally_is_ret",
                )
                .expect("build_int_compare should not fail");
            let ret_bb = context.append_basic_block(function, "trystar_finally_ret");
            builder
                .build_conditional_branch(is_ret, ret_bb, after_bb)
                .expect("build_conditional_branch should not fail");
            builder.position_at_end(ret_bb);
            if let Some(slot) = ret_slot {
                let ret_val = builder
                    .build_load(
                        ty_to_basic_type(context, expected_return_ty.clone()),
                        slot,
                        "trystar_finally_ret_val",
                    )
                    .expect("build_load should not fail for ret_slot");
                if let Some(outer) = finally_stack.last_mut() {
                    let outer_slot = outer
                        .ret_slot
                        .expect("nested finally in the same non-None function has a return slot");
                    builder
                        .build_store(outer_slot, ret_val)
                        .expect("build_store should not fail for outer ret_slot");
                    builder
                        .build_store(outer.is_returning, context.i8_type().const_int(1, false))
                        .expect("build_store should not fail for outer is_returning");
                    builder
                        .build_unconditional_branch(outer.finally_bb)
                        .expect("build_unconditional_branch should not fail");
                } else {
                    builder
                        .build_return(Some(&ret_val))
                        .expect("build_return should not fail for a finally-routed return");
                }
            } else if let Some(outer) = finally_stack.last_mut() {
                builder
                    .build_store(outer.is_returning, context.i8_type().const_int(1, false))
                    .expect("build_store should not fail for outer is_returning");
                builder
                    .build_unconditional_branch(outer.finally_bb)
                    .expect("build_unconditional_branch should not fail");
            } else if function.get_type().get_return_type().is_none() {
                builder
                    .build_return(None)
                    .expect("build_return should not fail for a finally-routed void return");
            } else {
                builder
                    .build_unreachable()
                    .expect("build_unreachable should not fail for dead ret_bb");
            }
        } else {
            builder
                .build_unconditional_branch(after_bb)
                .expect("build_unconditional_branch should not fail");
        }
    }

    builder.position_at_end(finally_exception_bb);
    builder
        .build_unconditional_branch(after_bb)
        .expect("build_unconditional_branch should propagate a finally exception");

    builder.position_at_end(after_bb);
    guard_statement_effects(context, builder, rt);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn int_binop(op: pycc_mir::BinOpKind) -> MirExpr {
        MirExpr::BinOp {
            op,
            left: Box::new(MirExpr::IntLiteral(8)),
            right: Box::new(MirExpr::IntLiteral(2)),
            ty: pycc_mir::Ty::Int,
        }
    }

    #[test]
    fn exception_guard_classification_keeps_the_arithmetic_fast_path_clean() {
        for op in [
            pycc_mir::BinOpKind::Add,
            pycc_mir::BinOpKind::Sub,
            pycc_mir::BinOpKind::Mul,
            pycc_mir::BinOpKind::Pow,
            pycc_mir::BinOpKind::BitAnd,
            pycc_mir::BinOpKind::BitOr,
            pycc_mir::BinOpKind::BitXor,
        ] {
            assert!(!expression_can_set_exception(&int_binop(op)));
        }
        for op in [
            pycc_mir::BinOpKind::Div,
            pycc_mir::BinOpKind::FloorDiv,
            pycc_mir::BinOpKind::Mod,
            pycc_mir::BinOpKind::LShift,
            pycc_mir::BinOpKind::RShift,
        ] {
            assert!(expression_can_set_exception(&int_binop(op)));
        }

        assert!(expression_can_set_exception(&MirExpr::Call {
            callee: "f".to_string(),
            args: Vec::new(),
            ty: pycc_mir::Ty::Int,
        }));
        assert!(expression_can_set_exception(&MirExpr::DictGet {
            dict: Box::new(MirExpr::DictLiteral(vec![(
                MirExpr::StringLiteral("key".to_string()),
                MirExpr::IntLiteral(1),
            )])),
            key: Box::new(MirExpr::StringLiteral("key".to_string())),
        }));
        assert!(expression_can_set_exception(&MirExpr::Subscript {
            base: Box::new(MirExpr::ListLiteral(vec![MirExpr::IntLiteral(1)])),
            index: Box::new(MirExpr::IntLiteral(0)),
        }));
        assert!(!expression_can_set_exception(&MirExpr::Subscript {
            base: Box::new(MirExpr::TupleLiteral(vec![MirExpr::IntLiteral(1)])),
            index: Box::new(MirExpr::IntLiteral(0)),
        }));
        assert!(expression_can_set_exception(&MirExpr::Instantiate(
            Box::new(pycc_mir::InstantiateExpr {
                ctor: "C.__init__".to_string(),
                class_name: "C".to_string(),
                slot_names: vec![],
                args: Vec::new(),
                ty: pycc_mir::Ty::Instance(Box::new("C".to_string())),
            },)
        )));
        // Part 2 of #1027: a buffer element load is always raising -- its
        // base is a `memoryview`, so `pycc_rt_buffer_f64_get`'s bounds check
        // can always set an `IndexError`. Unlike `Subscript` above there is
        // no false direction to pin: no base shape makes it non-raising.
        // #1388: an instance attribute read is always raising too -- the
        // checked slot read sets `AttributeError` for a slot not yet
        // assigned, whatever the attribute's type.
        assert!(expression_can_set_exception(&MirExpr::AttrGet {
            base: Box::new(MirExpr::Name {
                name: "c".to_string(),
                ty: pycc_mir::Ty::Instance(Box::new("C".to_string())),
            }),
            slot: 0,
            ty: pycc_mir::Ty::Int,
        }));
        assert!(expression_can_set_exception(&MirExpr::BufferGet {
            base: Box::new(MirExpr::Name {
                name: "b".to_string(),
                ty: pycc_mir::Ty::MemoryView,
            }),
            index: Box::new(MirExpr::IntLiteral(0)),
        }));
        // Part 2d of #1371: a list display built as a CPython `list` always
        // may raise -- its elements are boxed and the list allocated --
        // even an empty one.
        assert!(expression_can_set_exception(&MirExpr::ObjList {
            elements: Vec::new(),
        }));
        assert!(!expression_can_set_exception(&MirExpr::IntLiteral(1)));
    }
}
