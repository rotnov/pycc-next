//! Exception-specific code-generation state and expression guards (D-173).

use super::exception_value::emit_str_bytes_constant;
use super::*;
use std::cell::{Cell, RefCell};

mod try_star;
mod try_stmt;
pub(crate) use try_star::emit_try_star;
pub(crate) use try_stmt::emit_try;

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
    /// The counted (`heap_rc`) counterpart: fresh `str`/list/instance
    /// temporaries a consumer holds while a sibling or the consuming
    /// operation can still raise. Released by the same two unwinds, empty
    /// at every statement boundary; `heap_rc::hold_temporary` owns the
    /// push/pop discipline.
    pub(super) pending_heap_releases: RefCell<Vec<(crate::heap_rc::HeapKind, PointerValue<'ctx>)>>,
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
            pending_heap_releases: RefCell::new(Vec::new()),
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
        // `AttrGet` is *not* in this group although it raises
        // `AttributeError` for a slot read before its assignment (#1388):
        // `attr_slot::emit_slot_load_checked` emits the guard itself, on the
        // cold unassigned path only, so the common read pays no flag test.
        //
        // `ObjAttrGet` is in the `true` group (D-244, Part 2 of #1026):
        // unlike `AttrGet`'s compile-time-resolved slot, a foreign attribute
        // load is a real `PyObject_GetAttrString` call
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
        // A runtime builtin can raise (`int("x")`, `input()` at EOF, ...).
        | MirExpr::RtCall(_)
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
        | MirExpr::ObjectUnbox(_, _)
        // Non-strict mode (#3): CPython's operators raise (`TypeError`,
        // `ZeroDivisionError`, ...); `foreign_arith` owns the `NULL` check.
        | MirExpr::ObjBinOp { .. }
        | MirExpr::ObjDisplay { .. }
        | MirExpr::ObjUnaryOp { .. } => true,
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
        // `^` never raise. `**` raises for a negative exponent and for a
        // result too large to allocate. `int` `+`, `-` and `*` never raise
        // (arbitrary precision), but a `float` operation with an `int`
        // operand converts it, which raises `OverflowError` past `1e308`.
        // A `float` `+`, `-`, `*`, `/` or `**` (other than `int / int`,
        // computed exactly by `pycc_rt_int_truediv`) guards itself on the
        // cold paths `float_fast` emits -- an `int` operand's conversion,
        // a zero divisor, `pow` outside its exception-free domain -- so
        // `emit_expr` adds no guard after it.
        MirExpr::BinOp {
            op, left, right, ty, ..
        } if *ty == pycc_mir::Ty::Float
            && matches!(
                op,
                pycc_mir::BinOpKind::Add
                    | pycc_mir::BinOpKind::Sub
                    | pycc_mir::BinOpKind::Mul
                    | pycc_mir::BinOpKind::Div
                    | pycc_mir::BinOpKind::Pow
            )
            && !(*op == pycc_mir::BinOpKind::Div
                && matches!(left.ty(), pycc_mir::Ty::Int | pycc_mir::Ty::Bool)
                && matches!(right.ty(), pycc_mir::Ty::Int | pycc_mir::Ty::Bool)) =>
        {
            false
        }
        // An `int` `//`, `%`, `<<` or `>>` raises only on its runtime path,
        // which guards itself (`int_fast::emit_guarded`); `int ** int` is
        // always a runtime call and is guarded here.
        MirExpr::BinOp {
            op,
            ty: pycc_mir::Ty::Int,
            ..
        } => *op == pycc_mir::BinOpKind::Pow,
        MirExpr::BinOp {
            op, left, right, ty, ..
        } => {
            matches!(
                op,
                pycc_mir::BinOpKind::Div
                    | pycc_mir::BinOpKind::FloorDiv
                    | pycc_mir::BinOpKind::Mod
                    | pycc_mir::BinOpKind::Pow
                    | pycc_mir::BinOpKind::LShift
                    | pycc_mir::BinOpKind::RShift
            ) || (*ty == pycc_mir::Ty::Float
                && (left.ty() == pycc_mir::Ty::Int || right.ty() == pycc_mir::Ty::Int))
        }
        // `str()` of an `int` past CPython's 4300-digit limit raises
        // `ValueError`.
        MirExpr::FString(parts) => parts.iter().any(|part| {
            matches!(part, pycc_mir::MirFStringPart::Interpolation(e) if e.ty() == pycc_mir::Ty::Int)
        }),
        // A list index's `IndexError` leaves through the inlined bounds
        // check's own cold branch (`list::build_list_get`), so the read
        // needs no flag test; a tuple index is checked at compile time.
        // `xs.pop()` raises on an empty list or a bad index, and a slice on
        // a zero step.
        MirExpr::ListPop { .. } | MirExpr::Slice { .. } => true,
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
        // Guards itself on its cold path; see the comment at the top.
        | MirExpr::AttrGet { .. }
        | MirExpr::FloatLiteral(_)
        | MirExpr::BoolLiteral(_)
        | MirExpr::IntBoundary(_)
        | MirExpr::StringLiteral(_)
        | MirExpr::NoneLiteral
        | MirExpr::NotImplemented
        | MirExpr::Name { .. }
        | MirExpr::Compare { .. }
        | MirExpr::ListLiteral(_)
        | MirExpr::EmptyList(_)
        | MirExpr::EmptySet(_)
        | MirExpr::ListAppend { .. }
        | MirExpr::DictLiteral(_)
        | MirExpr::EmptyDict(_)
        | MirExpr::SetLiteral { .. }
        // Part 1 of #1319: `frozenset(x)` copies an already-validated
        // container, exactly as a set literal inserts already-validated
        // words -- the runtime's bigint raise is unreachable from a
        // `list[int]` codegen built, as `pycc_rt_int_set_from_list`'s
        // own doc comment records.
        | MirExpr::FrozenSetFrom { .. }
        // #1335: `hash(instance)`'s own work -- a pointer hash, a
        // `slot_tp_hash` reduction, an int birth -- cannot fail. A user
        // `__hash__` is the inner `Call`, which guards itself where
        // `emit_expr` evaluates it, before this node's work runs.
        | MirExpr::InstanceHash { .. }
        | MirExpr::TupleLiteral(_)
        | MirExpr::Subscript { .. }
        | MirExpr::ListInsert { .. }
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
        | MirExpr::BufferLen { .. }
        // The generator consumers guard each resume themselves
        // (`generator::resume_branch`, `pycc_rt_gen_next`'s guard).
        | MirExpr::GenNext { .. }
        | MirExpr::GenToList { .. }
        | MirExpr::GenAnyAll { .. } => false,
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
    let active = crate::int_fast::emit_exception_pending(context, builder, rt, "effect_exc_active");
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
        && rt.exceptions.pending_object_releases.borrow().is_empty()
        && rt.exceptions.pending_heap_releases.borrow().is_empty();
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
    crate::heap_rc::release_pending(context, builder, rt);
    builder
        .build_unconditional_branch(exception_target)
        .expect("build_unconditional_branch should not fail for a fresh unwind block");
}

#[cfg(test)]
mod tests;
