//! Entry-block storage slots, assignment stores and buffer-slot frees.

use super::*;

/// #382 (PR-22 Part 2): If the current block's terminator is an
/// `unreachable` (placed by `MirStmt::Raise`/`RaiseFrom`/`Reraise`),
/// erase it so the caller can append an exception check (conditional
/// branch) in this block. Returns `true` if an `unreachable` was erased
/// (meaning a raise happened and the exception is active), `false`
/// otherwise. After erasing, the block has no terminator and the caller
/// can build its exception-check conditional branch normally.
pub(crate) fn erase_unreachable_if_present<'ctx>(
    builder: &inkwell::builder::Builder<'ctx>,
) -> bool {
    if let Some(term) = builder.get_insert_block().unwrap().get_terminator()
        && term.get_opcode() == inkwell::values::InstructionOpcode::Unreachable
    {
        term.erase_from_basic_block();
        return true;
    }
    false
}

/// Runs `build` with the builder positioned at the *top* of `function`'s
/// entry block, then puts the builder back at the end of the block it was
/// emitting into, and returns whatever `build` produced.
///
/// This is the one place an expression that needs its own stack slot hoists
/// the `alloca`: an `alloca` is reclaimed only when its function returns, so
/// one emitted inside a loop grows the stack without bound, and LLVM's own
/// convention is that every `alloca` belongs in the entry block. The top of
/// the block, not its end, because by the time an expression is emitted the
/// entry block may already have been terminated -- a comprehension after an
/// `if`, a `while` or a `try` (#1254) is emitted into a later block, and
/// appending after the entry block's branch would be invalid IR. An entry
/// block with no instructions yet is appended to instead.
///
/// `foreign_call.rs`'s `alloca_in_entry_block` and `foreign_len.rs`'s
/// `out_slot_in_entry_block` build on this for their argument arrays and
/// out-slots; `comprehension.rs` uses it for an expression comprehension's
/// loop-variable slot.
pub(crate) fn build_at_entry_block<'ctx, T>(
    builder: &inkwell::builder::Builder<'ctx>,
    function: inkwell::values::FunctionValue<'ctx>,
    build: impl FnOnce(&inkwell::builder::Builder<'ctx>) -> T,
) -> T {
    let resume_at = builder
        .get_insert_block()
        .expect("the builder is positioned inside a block");
    let entry_block = function
        .get_first_basic_block()
        .expect("a function being emitted into has an entry block");
    match entry_block.get_first_instruction() {
        Some(first) => builder.position_before(&first),
        None => builder.position_at_end(entry_block),
    }
    let value = build(builder);
    builder.position_at_end(resume_at);
    value
}

/// An uninitialized `alloca` of `ty` in the entry block of the function the
/// builder is emitting into, leaving the builder where it was. Every fixed-size
/// slot belongs there: an `alloca` in a block a loop re-enters grows the stack
/// on every iteration (the frame is reclaimed only at return), and a slot
/// allocated in one branch does not dominate a read in another.
pub(crate) fn alloca_at_entry<'ctx>(
    builder: &inkwell::builder::Builder<'ctx>,
    ty: impl BasicType<'ctx>,
    name: &str,
) -> PointerValue<'ctx> {
    let function = builder
        .get_insert_block()
        .and_then(|block| block.get_parent())
        .expect("an alloca is always emitted inside a function");
    build_at_entry_block(builder, function, |b| {
        b.build_alloca(ty, name)
            .expect("build_alloca should not fail for a fixed-size slot")
    })
}

/// Allocates a local slot in the current function's entry block, which
/// dominates every branch, loop, and merge that may read it. String slots are
/// initialized to null and `int` slots to the word `0` (#146 Part 1), so the
/// first emitted decref/release is a safe no-op rather than a read of
/// uninitialized stack. `0` is deliberately *not* a valid encoded int -- it
/// is `classify_encoded_int`'s fail-closed pattern -- which is exactly why
/// `pycc_rt_bigint_release` returns on it before classifying. A guarded
/// non-parameter slot also receives an `i8` initialized flag, because #118's
/// static definite-assignment joins have not landed yet and a syntactically
/// present assignment may not execute at runtime.
pub(crate) fn storage_slot_at_entry<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    ty: pycc_mir::Ty,
    name: &str,
    guard_reads: bool,
) -> StorageSlot<'ctx> {
    let current_block = builder.get_insert_block().expect(
        "builder is always positioned inside some block while a statement is being emitted",
    );
    let function = current_block
        .get_parent()
        .expect("the block builder is currently positioned in always belongs to a function");
    let entry_block = function.get_first_basic_block().expect(
        "compile_to_object always appends a function's entry block before emitting its body",
    );
    builder.position_at_end(entry_block);
    let ptr = builder
        .build_alloca(ty_to_basic_type(context, ty.clone()), name)
        .expect("build_alloca should not fail for a supported local type");
    if matches!(
        ty,
        pycc_mir::Ty::Str
            | pycc_mir::Ty::MemoryView
            | pycc_mir::Ty::List(_)
            | pycc_mir::Ty::VarTuple(_)
            | pycc_mir::Ty::Instance(_)
            | pycc_mir::Ty::Dict(_)
            | pycc_mir::Ty::Set(_)
            | pycc_mir::Ty::FrozenSet(_)
            | pycc_mir::Ty::Object
            | pycc_mir::Ty::Generator(_)
            | pycc_mir::Ty::Callable(_)
    ) || matches!(&ty, pycc_mir::Ty::Tuple(elements) if pycc_hir::tuple_is_boxed(elements))
    {
        // Lists, instances, dicts, sets, boxed tuples and CPython objects are
        // counted (`heap_rc`) and released the same way, so their slots start
        // null too.
        // Part 2a of #1142 (#1165) joins `Ty::Str` on this arm rather than
        // adding one of its own: both are owning pointer slots released on
        // the way out, and both runtime releases are documented no-ops on
        // null. Without the null-init a `MemoryView` slot holds garbage, and
        // the epilogue's free and the first free-before-overwrite store
        // would each read uninitialized alloca memory. Gating those on the
        // `initialized` flag below is *not* an alternative: that flag guards
        // reads, and the very first store happens before any read.
        builder
            .build_store(
                ptr,
                context
                    .ptr_type(inkwell::AddressSpace::default())
                    .const_null(),
            )
            .expect("build_store should not fail immediately after this function's own alloca");
    } else if ty == pycc_mir::Ty::Int {
        builder
            .build_store(ptr, context.i64_type().const_zero())
            .expect("build_store should not fail immediately after this function's own alloca");
    } else if matches!(&ty, pycc_mir::Ty::Optional(inner) if **inner == pycc_mir::Ty::Int) {
        // #770 review: mirrors the `Ty::Int` zero-init immediately above,
        // but with the same valid-payload placeholder
        // `default_value_for_type`'s own `Ty::Optional(_)` arm builds
        // (`{ tag_smallint_const(0), 0 }`, never a raw `{0, 0}` struct) --
        // required so `release_optional_int_slot_before_store` can release
        // this slot's payload unconditionally, including at the very first
        // store, without ever reading uninitialized alloca memory or
        // tripping `classify_encoded_int`'s fail-closed panic on a raw
        // zero word.
        let placeholder = default_value_for_type(context, ty.clone());
        builder
            .build_store(ptr, placeholder)
            .expect("build_store should not fail immediately after this function's own alloca");
    }
    let initialized = if guard_reads {
        let initialized_ptr = builder
            .build_alloca(context.i8_type(), &format!("{name}_initialized"))
            .expect("build_alloca should not fail for an i8 initialization flag");
        builder
            .build_store(initialized_ptr, context.i8_type().const_zero())
            .expect("build_store should not fail for a fresh initialization flag");
        Some(initialized_ptr)
    } else {
        None
    };
    builder.position_at_end(current_block);
    StorageSlot {
        ptr,
        ty,
        initialized,
    }
}

/// Reuses the predeclared slot backing `target`. The slot's established type
/// wins over the current expression type, including the accepted
/// `bool`-to-`int` assignment that must store a D-141 encoded i64 rather than raw i8.
///
/// #146 Part 1: an `Ty::Int` slot releases its previous word here, before
/// the store, for *every* caller rather than at each call site
/// individually. `str`'s own release deliberately stays at `MirStmt::
/// Assign` (`decref_str_slot_before_store`): only a small subset of this
/// function's callers bind a `str`, whereas thirteen of them bind an `int`
/// (the four `range`-induction binds, the eight container-element binds,
/// and ordinary assignment), and a per-site release is exactly the shape
/// where one missed site ships a silent leak that no value assertion can
/// observe. Centralizing it here makes "released before every int store" a
/// property of the function rather than of an audit.
///
/// The release is gated on the *slot's* declared type, never on the value's:
/// `x: int = True` reaches here as a `Scalar::Bool` that
/// `coerce_scalar_to_type` (below) turns into a D-141 encoded word, and
/// value-type gating would skip the release for exactly that case.
// PEP 572 (#774): loosened from `&mut HashMap` to `&HashMap`. The body only
// ever reads the map (`locals.get(target)`), never inserts or removes a key
// -- the actual mutation this function performs is a `build_store` into the
// slot's own already-allocated pointer, not a change to the map of slots
// itself. Every pre-existing call site sits inside `emit_stmt`, which holds
// `locals: &mut HashMap<...>`; passing that through to a `&HashMap`
// parameter is an ordinary reborrow coercion, so this loosening is
// behavior-preserving for all of them. It is required, not merely tidier,
// for `MirExpr::NamedExpr`'s own codegen (`emit_expr_unchecked`'s new arm
// below): that function only ever holds a read-only `locals: &HashMap<...>`
// (expression lowering never needs to declare a *new* slot, only read
// existing ones), yet the walrus's "evaluate value, store to the bound
// local" behavior needs exactly this store logic to run mid-expression.
pub(crate) fn emit_assign<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    rt: &RtFns<'ctx>,
    locals: &HashMap<String, StorageSlot<'ctx>>,
    target: &str,
    value: Scalar<'ctx>,
) {
    let slot = locals
        .get(target)
        .cloned()
        .expect("every assignment target must have a predeclared storage slot");
    if slot.ty == pycc_mir::Ty::Int {
        // A local whose every value is a smallint holds nothing to release
        // (`int_range`).
        if !rt.int_ranges.borrow().is_small_local(target) {
            release_int_slot_before_store(context, builder, rt, &slot);
        }
    } else if matches!(&slot.ty, pycc_mir::Ty::Optional(inner) if **inner == pycc_mir::Ty::Int) {
        // #770 review: the release-side half of the retain
        // `MirExpr::OptionalWrap`'s codegen now performs on a borrowed
        // `int` payload -- without this, that retain would be a permanent
        // per-reassignment leak of the slot's previous payload instead of
        // a balanced reference (D-197's only supported `Optional[T]`
        // payload is `int`, so this is the only `Optional` shape that can
        // ever hold a bigint word to release).
        release_optional_int_slot_before_store(context, builder, rt, &slot);
    }
    let value = coerce_scalar_to_type(context, builder, value, slot.ty);
    let basic_value: inkwell::values::BasicValueEnum = match value {
        Scalar::Generator(v) | Scalar::Closure(v) => v.into(),
        Scalar::Int(v) => v.into(),
        Scalar::Bool(v) => v.into(),
        Scalar::Float(v) => v.into(),
        Scalar::Str(v) => v.into(),
        // Pass-through, identical to `Str`'s arm directly above: the caller
        // already took the reference the slot owns and released the one it
        // held (`heap_rc`).
        Scalar::List(v) => v.into(),
        // Pass-through, identical to `List`'s arm directly above: storing
        // a `dict[K, V]` value is storing one opaque pointer into a slot
        // `ty_to_basic_type` already allocated as a pointer. No refcount
        // traffic accompanies it -- D-124 keeps `dict[K, V]` leak-only for
        // v0.2 (extending D-107's exact reasoning), so unlike `Str` there
        // is deliberately no incref here and no
        // `decref_str_slot_before_store` counterpart.
        Scalar::Dict(v) => v.into(),
        // Pass-through, identical to `List`'s/`Dict`'s arms directly
        // above: storing a `set[T]` value is storing one opaque pointer
        // into a slot `ty_to_basic_type` already allocated as a pointer.
        // No refcount traffic accompanies it -- D-124 keeps `set[T]`
        // leak-only for v0.2 (extending D-107's exact reasoning),
        // identically to `List`/`Dict`.
        Scalar::Set(v) => v.into(),
        // Pass-through like the three arms above, but storing a whole
        // struct by value rather than one opaque pointer (D-115) -- into a
        // slot `ty_to_basic_type` already allocated at exactly that struct
        // type, so the store is type-correct without any tuple-specific
        // code at the `build_store` call below. The store itself emits no
        // refcount traffic, but that is no longer because a tuple holds
        // nothing worth counting: under D-182 a tuple field can hold an
        // ingress-retained reference to a borrowed `int` element, so
        // overwriting this slot drops the struct without releasing those
        // fields. That is the accepted, documented leak -- a slot-death
        // release for tuple fields is deferred to D-124's container
        // refcounting, not simply absent because there is nothing to free.
        Scalar::Tuple(v) => v.into(),
        // Pass-through like `List`'s arm above: an instance is reference
        // counted, and the caller handled the slot's references (`heap_rc`).
        Scalar::Instance(v) => v.into(),
        // Pass-through by VALUE, identical in kind to `Tuple`'s arm above
        // (D-197, #763, Part 1 of #747): `coerce_scalar_to_type` above
        // already built the correctly-typed `{ int, i8 }` struct against
        // `slot.ty`, so this arm needs only the store, exactly like
        // `Tuple`'s own struct-by-value store above -- no refcount traffic
        // accompanies it either, for the identical D-182-acknowledged
        // reason `Tuple`'s own comment already gives.
        Scalar::Optional(v) => v.into(),
        // Storing one `PyObject *` into a slot `ty_to_basic_type` already
        // allocated as a pointer. The value is already an owned reference
        // and the caller releases the replaced one (`heap_rc`).
        Scalar::Object(v) => v.into(),
        // A pass-through since Part 2a of #1142 (#1165), where it was a
        // panic (as `Object`'s arm above was until #1325): storing one
        // `PyccExtBufferView *` into a slot `ty_to_basic_type` already
        // allocated as a pointer.
        //
        // The value is always artifact-owned storage, never the host's. The
        // two shapes that would put a *borrowed* view here are both refused
        // before codegen: `c = b` is a bare read of a buffer parameter
        // (`reject_memoryview_read`'s `C0001`), and `b = ndarray(n)` on a
        // parameter name is `buffer_parameter_rebinding`'s. What makes the
        // store safe rather than a leak is the pair around it --
        // `free_buffer_slot_before_store` releases whatever the slot already
        // held (D-074), and the frame's owned-slot epilogue releases this one
        // on every return path -- so unlike `List`/`Dict`/`Set` above, this
        // arm is deliberately *not* on the D-107 leak-only precedent.
        Scalar::MemoryView(v) => v.into(),
    };
    builder
        .build_store(slot.ptr, basic_value)
        .expect("build_store should not fail for a slot this function itself allocated");
    if let Some(initialized_ptr) = slot.initialized {
        builder
            .build_store(initialized_ptr, context.i8_type().const_int(1, false))
            .expect("build_store should not fail for a declared global flag");
    }
}

/// Stores a smallint `word` into an `int` slot whose previous value the
/// caller has already released (`for_range_fast`'s hoisted release): the
/// store half of [`emit_assign`] without its release.
pub(crate) fn store_unowned_int<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    slot: &StorageSlot<'ctx>,
    word: inkwell::values::IntValue<'ctx>,
) {
    builder
        .build_store(slot.ptr, word)
        .expect("build_store should not fail for a slot this function itself allocated");
    if let Some(initialized_ptr) = slot.initialized {
        builder
            .build_store(initialized_ptr, context.i8_type().const_int(1, false))
            .expect("build_store should not fail for a declared flag");
    }
}

/// Mirror of [`decref_str_slot_before_store`] for an artifact-owned buffer
/// slot (Part 2a of #1142, #1165): loads the target's predeclared slot and
/// frees its current storage before the new view pointer overwrites it.
///
/// D-074's rule, applied to the second owning slot type this crate has.
/// Without it `a = ndarray(3); a = ndarray(4)` leaks the first allocation
/// and `for i in range(n): a = ndarray(3)` leaks `n - 1` of them -- an
/// unbounded leak inside someone else's process, which is why the D-107
/// leak precedent for `list`/`dict`/`set` is deliberately *not* followed
/// here. Buffer slots are null-initialized by `storage_slot_at_entry`, and
/// `pycc_rt_buffer_f64_free` is a documented no-op on null, so the same path
/// is correct for the first assignment and for every later one.
pub(crate) fn free_buffer_slot_before_store<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    rt: &RtFns<'ctx>,
    locals: &HashMap<String, StorageSlot<'ctx>>,
    target: &str,
) {
    let slot = &locals[target];
    if slot.ty != pycc_mir::Ty::MemoryView {
        panic!(
            "pycc_codegen: internal error: buffer assignment target `{target}` has a non-buffer storage slot"
        );
    }
    let old = builder
        .build_load(
            context.ptr_type(inkwell::AddressSpace::default()),
            slot.ptr,
            "old_buffer",
        )
        .expect("build_load should not fail for this function's own alloca")
        .into_pointer_value();
    // Part 2b of #1142 (#1164): a rebinding that happens while a `return` of
    // this same pointer is pending must not release it -- the value is on
    // its way to the host. That is reachable: `try: return a` followed by
    // `finally: a = ndarray(n)` runs this store *after* the return statement
    // made `a`'s pointer pending, so an unguarded free here would hand the
    // host a view over storage this frame had already released.
    //
    // Skipping the free makes the pending slot the pointer's sole owner, so
    // the orphan flag is raised in the same breath; the frame's owned-slot
    // epilogue reads it to decide whether it must release the pending
    // pointer itself (when the return was abandoned by a raising finalizer)
    // or leave it to the host (when the return proceeded). Branch-free on
    // the `select` model the epilogue uses, so the emitted block keeps the
    // straight-line shape #1165 gave it.
    let release = match locals.get(PENDING_RETURN_BUFFER_KEY) {
        None => old,
        Some(pending) => {
            let pending_ptr = builder
                .build_load(
                    context.ptr_type(inkwell::AddressSpace::default()),
                    pending.ptr,
                    "pending_return_buffer",
                )
                .expect("build_load should not fail for the pending-return slot")
                .into_pointer_value();
            // Both halves are load-bearing. The equality alone is true for
            // the *first* allocation into a never-assigned slot, where `old`
            // and the pending record are both the entry null: that would
            // raise the orphan flag on a frame that never rebound anything,
            // and the epilogue would then release a pointer its own slot
            // loop had already released -- a double free on every
            // exception exit, which is what an unguarded equality actually
            // produced here before this conjunction was added.
            let is_live = builder
                .build_int_compare(
                    inkwell::IntPredicate::NE,
                    old,
                    context
                        .ptr_type(inkwell::AddressSpace::default())
                        .const_null(),
                    "buffer_store_old_is_live",
                )
                .expect("build_int_compare should not fail for two pointers");
            let is_same = builder
                .build_int_compare(
                    inkwell::IntPredicate::EQ,
                    old,
                    pending_ptr,
                    "buffer_store_matches_pending",
                )
                .expect("build_int_compare should not fail for two pointers");
            let is_pending = builder
                .build_and(is_live, is_same, "buffer_store_is_pending")
                .expect("build_and should not fail for two i1 values");
            let orphaned = &locals[PENDING_RETURN_ORPHANED_KEY];
            let previously = builder
                .build_load(
                    context.i8_type(),
                    orphaned.ptr,
                    "pending_return_was_orphaned",
                )
                .expect("build_load should not fail for the pending-return orphan flag")
                .into_int_value();
            let raised = builder
                .build_select(
                    is_pending,
                    context.i8_type().const_int(1, false),
                    previously,
                    "pending_return_orphaned",
                )
                .expect("build_select should not fail for two i8 flags");
            builder
                .build_store(orphaned.ptr, raised)
                .expect("build_store should not fail for the pending-return orphan flag");
            builder
                .build_select(
                    is_pending,
                    context
                        .ptr_type(inkwell::AddressSpace::default())
                        .const_null(),
                    old,
                    "buffer_free_old_release",
                )
                .expect("build_select should not fail for two pointers")
                .into_pointer_value()
        }
    };
    builder
        .build_call(rt.buffer_f64_free, &[release.into()], "buffer_free_old")
        .expect("build_call should not fail for a well-formed buffer free");
}
