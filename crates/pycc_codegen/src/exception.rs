//! Exception-specific code-generation state and expression guards (D-173).

use super::exception_value::emit_str_bytes_constant;
use super::*;
use std::cell::{Cell, RefCell};

mod finally;
pub(crate) mod handled;
pub(crate) mod lazy_miss;
pub(crate) mod owned_slots;
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
    /// The exception slots live while a `try` clause runs, outermost first
    /// (`owned_slots`). Empty at every function boundary.
    pub(super) owned_slots: RefCell<Vec<PointerValue<'ctx>>>,
    /// The live slots of `owned_slots` that hold a CPython object rather
    /// than an exception, each with the shim function that releases it.
    pub(super) owned_object_slots: RefCell<Vec<(PointerValue<'ctx>, FunctionValue<'ctx>)>>,
    /// The `try`s whose `KeyError` handler takes lazy misses, innermost
    /// last (`lazy_miss`).
    pub(super) lazy_misses: RefCell<Vec<lazy_miss::LazyMiss<'ctx>>>,
    /// The handlers that publish the exception they handle whose bodies
    /// are being emitted, innermost last (`handled`).
    pub(super) handlers: RefCell<Vec<handled::Open<'ctx>>>,
    /// The slots a publishing handler's way out releases, and the releases
    /// emitted for them since the outermost such handler was entered
    /// (`handled::exits`).
    pub(super) handler_slots: RefCell<Vec<PointerValue<'ctx>>>,
    pub(super) handler_exits: RefCell<Vec<inkwell::values::InstructionValue<'ctx>>>,
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
            owned_slots: RefCell::new(Vec::new()),
            owned_object_slots: RefCell::new(Vec::new()),
            lazy_misses: RefCell::new(Vec::new()),
            handlers: RefCell::new(Vec::new()),
            handler_slots: RefCell::new(Vec::new()),
            handler_exits: RefCell::new(Vec::new()),
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
        // A native `str | None` local's tests (`pycc_mir::optional_str`)
        // read a pointer and compare bytes; none of them raises.
        MirExpr::RtCall(call) if call.sig.symbol.starts_with("pycc_rt_optional_str_") => false,
        // A region's flag is a load (`rt_call::emit_region_intact`).
        MirExpr::RtCall(call) if call.sig.symbol == crate::rt_call::REGION_INTACT_SYMBOL => false,
        // An operation on CPython objects raises nothing pycc's pending state
        // (D-173) holds once it is done: its helper reports a failure
        // CPython's way (a `NULL` result or a negative status, with
        // CPython's error indicator set), and its emitter routes that
        // failure itself, immediately, through `foreign_fail`'s edge -- in
        // a function body the shim's error bridge and a branch to the
        // innermost exception target, in the module body outside every
        // module-level `try` the import's `-1` return. A direct call
        // between compiled bodies (`direct_call`) tests the pending state
        // itself right after the body returns. So on the success path the
        // D-173 guard after the node could never fire: no pending
        // exception reaches it. It used to be emitted anyway, as
        // fail-closed conservatism, and it cost a load, a test and a
        // branch after every object operation, plus a cold block each:
        // a tenth of a generic body's LLVM instructions (pydantic's
        // `json_schema`). `tests/object_failure_edges.rs` checks that
        // every one of these operations still stops where CPython does
        // when it fails.
        //
        // The emitters, by node: `foreign_attr::emit` (an attribute
        // load); `foreign_call` (a method call, a call, its keyword form,
        // a subscript, a list, a display); `foreign_len` (a length, a
        // float-tuple unpack); `foreign_unpack::emit_unpack`;
        // `foreign_compare` (`isinstance`, `in`, a rich comparison; an
        // identity test cannot fail at all); `foreign_slice::emit_slice`;
        // `foreign_type::emit_type`;
        // `object_box::emit_boxed`; `object_unbox::emit_unboxed`;
        // `object_coerce` (a conversion, an index); `tuple_rebox::emit`;
        // `foreign_arith` (binary and unary operators).
        MirExpr::ObjAttrGet { .. }
        | MirExpr::ObjMethodCall { .. }
        | MirExpr::ObjCall { .. }
        | MirExpr::ObjKeywordCall(_)
        | MirExpr::ObjLen { .. }
        | MirExpr::ObjSubscript { .. }
        | MirExpr::ObjUnpackFloatTuple { .. }
        | MirExpr::ObjUnpack { .. }
        | MirExpr::ObjIsInstance { .. }
        | MirExpr::ObjContains { .. }
        | MirExpr::ObjSlice { .. }
        | MirExpr::ObjType { .. }
        | MirExpr::ObjList { .. }
        | MirExpr::ObjectBox(_)
        | MirExpr::ObjectUnbox(_, _)
        | MirExpr::ObjectCoerce(_, _)
        | MirExpr::TupleRebox(_, _)
        | MirExpr::ObjectIndex(..)
        | MirExpr::ObjBinOp { .. }
        | MirExpr::ObjDisplay { .. }
        | MirExpr::ObjUnaryOp { .. }
        | MirExpr::ObjCompare { .. } => false,
        // `AttrGet` is *not* in this group although it raises
        // `AttributeError` for a slot read before its assignment (#1388):
        // `attr_slot::emit_slot_load_checked` emits the guard itself, on the
        // cold unassigned path only, so the common read pays no flag test.
        MirExpr::Call { .. }
        | MirExpr::CallClosure { .. }
        // A runtime builtin can raise (`int("x")`, `input()` at EOF, ...).
        | MirExpr::RtCall(_)
        | MirExpr::DictGet { .. }
        | MirExpr::Instantiate(_)
        // Part 2 of #1027: `BufferGet` is fallible, and here the
        // dependency is live -- `pycc_rt_buffer_f64_get` sets pycc's own
        // D-173 pending state and returns a `0.0` sentinel for an index
        // outside `[0, len)`, so this guard is the whole of the failure
        // handling. A by-name entry rather than a widening of the
        // `Subscript` arm below, which asks its base's type: a `BufferGet`
        // is always fallible, with no base type to consult.
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
        | MirExpr::Comprehension(_) => true,
        // #1346: the node itself raises nothing. `sequence::emit_sequence`
        // emits both children through the guarded `emit_expr`, so each
        // child's own guard has already handled a raise by the time the
        // sequence yields.
        MirExpr::Sequence { .. } => false,
        // A typed array operation branches to the foreign failure edge
        // itself the moment a check or an index fails (`ndarray`), so it
        // never leaves a pending exception for the post-node guard. The one
        // exception is converting an `int` element too large for a
        // smallint, which allocates a bigint and cannot fail.
        MirExpr::Nd(_) => false,
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
                && matches!(left.ty(), pycc_mir::Ty::Int | pycc_mir::Ty::Bool | pycc_mir::Ty::Num)
                && matches!(right.ty(), pycc_mir::Ty::Int | pycc_mir::Ty::Bool | pycc_mir::Ty::Num))
            && !(*op == pycc_mir::BinOpKind::Div
                && (left.ty() == pycc_mir::Ty::Num || right.ty() == pycc_mir::Ty::Num)) =>
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
        // An `int | float` operation raises `ZeroDivisionError`, and
        // `OverflowError` converting a huge `int` operand to `float`
        // (`crate::num`).
        MirExpr::BinOp {
            ty: pycc_mir::Ty::Num,
            ..
        } => true,
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
            matches!(part, pycc_mir::MirFStringPart::Interpolation(e) if matches!(e.ty(), pycc_mir::Ty::Int | pycc_mir::Ty::Num))
        }),
        // A list index's `IndexError` leaves through the inlined bounds
        // check's own cold branch (`list::build_list_get`), so the read
        // needs no flag test; a tuple index is checked at compile time.
        // `xs.pop()` raises on an empty list or a bad index, and a slice on
        // a zero step.
        MirExpr::ListPop { .. } | MirExpr::Slice { .. } => true,
        // An ordering or a membership test of instances calls user methods
        // (each guarded), and `xs.sort()` raises when the list changed.
        MirExpr::InstanceOrder(_) | MirExpr::InstanceSetContains { .. } => true,
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
        | MirExpr::Ellipsis
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
        // Converting a `float` into an `int | float` word may box it, which
        // cannot fail.
        | MirExpr::NumWrap(_)
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
        | MirExpr::GenAnyAll { .. }
        | MirExpr::GenYield { .. }
        | MirExpr::GenYieldFrom { .. }
        | MirExpr::GenSend { .. }
        | MirExpr::GenThrow { .. }
        | MirExpr::GenClose { .. }
        | MirExpr::StopIterationValue { .. }
        // Building a function value calls nothing that raises.
        | MirExpr::MakeClosure { .. } => false,
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
    let has_exc =
        crate::int_fast::emit_exception_pending_test(context, builder, rt, "effect_has_exc");
    let function = builder.get_insert_block().unwrap().get_parent().unwrap();
    let continuation = context.append_basic_block(function, "effect_exc_cont");
    // Part 1 of #1092: a held object temporary needs the unwind block
    // exactly as a held bigint does; both empty keeps the two-block shape.
    let nothing_pending = rt.exceptions.pending_int_releases.borrow().is_empty()
        && rt.exceptions.pending_object_releases.borrow().is_empty()
        && rt.exceptions.pending_heap_releases.borrow().is_empty();
    if nothing_pending {
        let exception_target =
            crate::frames::exception_edge(context, builder, rt, exception_target);
        let branch = builder
            .build_conditional_branch(has_exc, exception_target, continuation)
            .expect("build_conditional_branch should guard a statement effect");
        crate::int_fast::weight_branch(context, branch, false);
        builder.position_at_end(continuation);
        return;
    }
    let unwind_bb = context.append_basic_block(function, "effect_exc_unwind");
    let branch = builder
        .build_conditional_branch(has_exc, unwind_bb, continuation)
        .expect("build_conditional_branch should guard a statement effect");
    crate::int_fast::weight_branch(context, branch, false);
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
    let exception_target = crate::frames::exception_edge(context, builder, rt, exception_target);
    builder
        .build_unconditional_branch(exception_target)
        .expect("build_unconditional_branch should not fail for a fresh unwind block");
}

#[cfg(test)]
mod tests;
