//! memoryview/buffer element access and pending-return ownership.

use super::*;

// ---------------------------------------------------------------------------
// Part 2 of #1027 and #1116: the `Scalar::MemoryView` arms,
// `MirExpr::BufferGet` and `MirExpr::BufferLen`.
//
// A `memoryview` reaches codegen only as a parameter of a `pycc build --ext`
// export (D-244), and `reject_memoryview_read` refuses every *use* of such a
// name except the `b[i]` element load and the `len(b)` count these parts
// admit. Every consuming arm
// below is therefore defensive, and each is pinned directly with a
// hand-built `Scalar::MemoryView` carrying a null `PyccExtBufferView *` --
// the same convention the `Scalar::List` and `Scalar::Object` defensive
// tests above use, and for the same reason: it pins the panic to the
// function that owns the gap rather than to whichever caller reaches it
// first. The *reachable* paths -- `MirExpr::Name`'s pointer-slot load and
// the `BufferGet`/`BufferLen` emissions themselves -- are covered by real
// MIR at the end of this section.
// ---------------------------------------------------------------------------

/// A null `PyccExtBufferView *` as a [`Scalar::MemoryView`]. None of the
/// defensive tests below dereferences it -- each panics inside its
/// function's own `match` before any `build_*` call runs.
fn null_memoryview_scalar(context: &Context) -> Scalar<'_> {
    Scalar::MemoryView(
        context
            .ptr_type(inkwell::AddressSpace::default())
            .const_null(),
    )
}

#[test]
#[should_panic(expected = "expected an int-or-bool operand, got memoryview")]
fn to_numeric_encoded_int_rejects_a_memoryview_operand() {
    let context = Context::create();
    let builder = context.create_builder();
    to_numeric_encoded_int(&context, &builder, null_memoryview_scalar(&context));
}

#[test]
#[should_panic(expected = "expected a numeric operand, got memoryview")]
fn to_float_rejects_a_memoryview_operand() {
    // A buffer *element* is a `float`, but the buffer itself is not: there
    // is no `float(b)` in CPython either.
    let context = Context::create();
    let (_module, rt) = list_scalar_panic_fixture(&context);
    let builder = context.create_builder();
    to_float(&context, &builder, &rt, null_memoryview_scalar(&context));
}

#[test]
#[should_panic(expected = "string conversion of a memoryview value is not supported yet")]
fn to_str_rejects_a_memoryview_operand() {
    // `print(b)` and `f"{b}"` are both bare reads of the name, which
    // `reject_memoryview_read` refuses with `C0001` before codegen.
    let context = Context::create();
    let (_module, rt) = list_scalar_panic_fixture(&context);
    let builder = context.create_builder();
    to_str(&builder, &rt, null_memoryview_scalar(&context));
}

#[test]
#[should_panic(expected = "truthiness of a memoryview value is not supported yet")]
fn truthiness_of_a_memoryview_value_panics_honestly() {
    // `if b:` is a bare read of the name, so `reject_memoryview_read`
    // refuses it with `C0001`. CPython's own answer would be `len(b) != 0`,
    // which #1027 does not ship -- an honest panic naming the gap is the
    // correct behavior until it does.
    let context = Context::create();
    let (module, rt) = list_scalar_panic_fixture(&context);
    let builder = context.create_builder();
    truthy(
        &context,
        &builder,
        &module,
        &rt,
        null_memoryview_scalar(&context),
    );
}

/// `len(b)` as a `MirExpr`, the buffer shape #1116 adds.
fn buffer_len_b() -> MirExpr {
    MirExpr::BufferLen {
        base: Box::new(MirExpr::Name {
            name: "b".to_string(),
            ty: Ty::MemoryView,
        }),
    }
}

#[test]
fn a_buffer_element_load_untags_its_index_and_calls_the_runtime_helper() {
    // The whole admitted read surface, end to end through real MIR: the
    // `memoryview` parameter's pointer slot is loaded (`MirExpr::Name`'s
    // own `Ty::MemoryView` arm), the D-141-tagged index is decoded through
    // the shared checked untag, and the pair is handed to the runtime
    // helper that owns the bounds check. Nothing here dereferences the
    // buffer: `pycc_rt_buffer_f64_get` does, after it has compared the
    // index against the view's recorded length.
    compile_ext_items_checking_ir(
        "buffer_element_load",
        buffer_fn_items(vec![MirStmt::Return(Some(buffer_get_b_i()))], Ty::Float),
        |ir| {
            assert!(ir.contains("call i64 @pycc_rt_int_untag_checked"), "{ir}");
            assert!(ir.contains("call double @pycc_rt_buffer_f64_get"), "{ir}");
        },
    );
}

#[test]
fn a_buffer_length_read_calls_the_runtime_helper_and_tags_the_count() {
    // #1116, end to end through real MIR. Two properties, both of which a
    // `return len(b)` test alone would pass without: the raw `i64` count is
    // re-tagged into D-141's form before it becomes a user-visible `int`
    // (`list_tag_shl`/`list_tag_or`, the same pair `raw_i64_to_tagged_int`
    // emits for `pycc_rt_int_list_len`), and no untag of an index is
    // emitted, because there is no index. An untagged count would compile,
    // return plausibly, and then produce a wrong iteration count the moment
    // it is used as a `range` bound.
    compile_ext_items_checking_ir(
        "buffer_length_read",
        buffer_fn_items(vec![MirStmt::Return(Some(buffer_len_b()))], Ty::Int),
        |ir| {
            assert!(ir.contains("call i64 @pycc_rt_buffer_len"), "{ir}");
            assert!(ir.contains("list_tag_shl"), "{ir}");
            assert!(ir.contains("list_tag_or"), "{ir}");
        },
    );
}

#[test]
fn a_buffer_length_read_emits_no_pending_exception_check() {
    // `pycc_rt_buffer_len` cannot raise, so `expression_can_set_exception`
    // answers `false` for this node where it answers `true` for both its
    // siblings. Asserted as a pair so a regression in either direction is
    // caught: the same function shape emits D-173 guards when its body is
    // the fallible element load (one for the index decode, which raises for
    // a heap bigint, and one for the load), and none at all when it is the
    // length read. A `true` classification here would emit a never-taken
    // branch after every `len(b)`.
    let guard = "call i8 @pycc_rt_exception_active()";
    compile_ext_items_checking_ir(
        "buffer_length_no_guard",
        buffer_fn_items(vec![MirStmt::Return(Some(buffer_len_b()))], Ty::Int),
        |ir| {
            assert_eq!(ir.matches(guard).count(), 0, "{ir}");
        },
    );
    compile_ext_items_checking_ir(
        "buffer_load_has_guard",
        buffer_fn_items(vec![MirStmt::Return(Some(buffer_get_b_i()))], Ty::Float),
        |ir| {
            assert_eq!(ir.matches(guard).count(), 2, "{ir}");
        },
    );
}

#[test]
#[should_panic(expected = "a buffer length read's base did not evaluate to a memoryview")]
fn a_buffer_length_read_whose_base_is_not_a_memoryview_is_an_internal_error() {
    // The same argument the element load's own defensive arm records:
    // `MirExpr::BufferLen` is produced by exactly one lowering arm, which
    // keys on the lowered argument's `Ty::MemoryView`, so a base of any
    // other type means that dispatch regressed.
    compile_ext_items_checking_ir(
        "buffer_length_bad_base",
        buffer_fn_items(
            vec![MirStmt::Return(Some(MirExpr::BufferLen {
                base: Box::new(MirExpr::Name {
                    name: "i".to_string(),
                    ty: Ty::Int,
                }),
            }))],
            Ty::Int,
        ),
        |_| unreachable!("codegen should have panicked"),
    );
}

#[test]
#[should_panic(expected = "a buffer element store's base did not evaluate to a memoryview")]
fn a_buffer_element_store_whose_base_is_not_a_memoryview_is_an_internal_error() {
    // `MirStmt::BufferSet` is produced by exactly one lowering arm, which
    // keys on the target name's `Ty::MemoryView` (`pycc_mir`'s `DictSet`
    // arm), so a base of any other type means that dispatch regressed --
    // the same argument the load's and the length read's own defensive
    // arms record. The deliberately mistyped `int` base below is a shape
    // no type-checked program can produce.
    compile_ext_items_checking_ir(
        "buffer_element_store_bad_base",
        buffer_fn_items(
            vec![MirStmt::BufferSet {
                base: MirExpr::Name {
                    name: "i".to_string(),
                    ty: Ty::Int,
                },
                index: MirExpr::IntLiteral(0),
                value: MirExpr::FloatLiteral(1.0),
            }],
            Ty::None,
        ),
        |_| unreachable!("codegen should have panicked"),
    );
}

/// The store emits a D-173 guard after its setter (plus the index decode's
/// own), where the length read emits none: the void setter's exception path
/// is the `IndexError` the runtime helper raises, so dropping the guard
/// would let an out-of-range store run the rest of the body.
#[test]
fn a_buffer_element_store_emits_its_pending_exception_guards() {
    compile_ext_items_checking_ir(
        "buffer_store_has_guard",
        buffer_fn_items(
            vec![MirStmt::BufferSet {
                base: MirExpr::Name {
                    name: "b".to_string(),
                    ty: Ty::MemoryView,
                },
                index: MirExpr::Name {
                    name: "i".to_string(),
                    ty: Ty::Int,
                },
                value: MirExpr::FloatLiteral(1.0),
            }],
            Ty::None,
        ),
        |ir| {
            // One guard for the index decode (a heap bigint raises), one for
            // the store.
            assert_eq!(
                ir.matches("call i8 @pycc_rt_exception_active()").count(),
                2,
                "{ir}"
            );
            assert_eq!(ir.matches("@pycc_rt_buffer_f64_set").count(), 2, "{ir}");
        },
    );
}

/// CPython evaluates `b[i] = v`'s **right-hand side first**, then the
/// subscription target, then the index -- a `d[i()] = v()` probe on the
/// reference interpreter prints `v`'s effect before `i`'s. The emitted IR
/// must observe the same order, or a raising or side-effecting index
/// expression would suppress a value expression CPython would already have
/// run.
///
/// The witness is a store whose value is itself a buffer load: with the
/// value emitted first, the index's own `pycc_rt_int_untag_checked` falls
/// *between* the load and the store. Emitting the index first would put
/// both untags ahead of the load and leave that span empty.
#[test]
fn a_buffer_element_store_emits_its_value_before_its_index() {
    compile_ext_items_checking_ir(
        "buffer_store_value_before_index",
        buffer_fn_items(
            vec![MirStmt::BufferSet {
                base: MirExpr::Name {
                    name: "b".to_string(),
                    ty: Ty::MemoryView,
                },
                index: MirExpr::Name {
                    name: "i".to_string(),
                    ty: Ty::Int,
                },
                value: buffer_get_b_i(),
            }],
            Ty::None,
        ),
        |ir| {
            // Anchored on the `call` sites, not on the bare symbol: the
            // module prints its `declare` lines ahead of every definition.
            let load_at = ir
                .find("call double @pycc_rt_buffer_f64_get")
                .unwrap_or_else(|| panic!("the value's own load is missing: {ir}"));
            let store_at = ir
                .find("call void @pycc_rt_buffer_f64_set")
                .unwrap_or_else(|| panic!("the store is missing: {ir}"));
            assert!(load_at < store_at, "{ir}");
            assert!(
                ir[load_at..store_at].contains("call i64 @pycc_rt_int_untag_checked"),
                "the index was decoded before the value was evaluated: {ir}"
            );
        },
    );
}

#[test]
#[should_panic(expected = "a buffer element load's base did not evaluate to a memoryview")]
fn a_buffer_element_load_whose_base_is_not_a_memoryview_is_an_internal_error() {
    // `MirExpr::BufferGet` is produced by exactly one lowering arm, which
    // keys on the lowered base's `Ty::MemoryView` (`pycc_mir`'s
    // `Subscript` arm), so a base of any other type means that dispatch
    // regressed. The deliberately mistyped `int` base below is a shape no
    // type-checked program can produce.
    compile_ext_items_checking_ir(
        "buffer_element_load_bad_base",
        buffer_fn_items(
            vec![MirStmt::Return(Some(MirExpr::BufferGet {
                base: Box::new(MirExpr::Name {
                    name: "i".to_string(),
                    ty: Ty::Int,
                }),
                index: Box::new(MirExpr::IntLiteral(0)),
            }))],
            Ty::Float,
        ),
        |_| unreachable!("codegen should have panicked"),
    );
}

#[test]
fn returning_owned_buffer_storage_transfers_it_in_the_epilogue_not_at_the_return() {
    // Part 2b of #1142 (#1164). Ownership of a returned buffer transfers at
    // the frame's owned-slot epilogue, which is the one point control
    // actually leaves the frame -- **not** at the `return` statement, which
    // only makes the value pending. Between the two, `finally_stack`
    // interposes every user finalizer.
    //
    // Both directions are pinned, because only the pair excludes the
    // regression: the returning block must *not* clear the slot (clearing it
    // there made a finalizer that reads the name dereference null and abort
    // the hosting interpreter), and the epilogue must decide by identity
    // against the value actually being returned (without that the epilogue
    // frees storage the host is still reading).
    compile_ext_items_checking_ir(
        "buffer_returned",
        vec![MirItem::Function {
            name: "allocate".to_string(),
            params: vec![],
            return_ty: Ty::MemoryView,
            body: vec![
                buffer_alloc_a(),
                MirStmt::Return(Some(MirExpr::Name {
                    name: "a".to_string(),
                    ty: Ty::MemoryView,
                })),
            ],
        }],
        |ir| {
            // The LLVM signature returns the carrier pointer, not `void`.
            assert!(ir.contains("define ptr @pyfn_allocate()"), "{ir}");
            let block = ir
                .split("global_ready:")
                .nth(1)
                .unwrap_or_else(|| panic!("no returning block: {ir}"));
            let load_at = block
                .find("%load = load ptr, ptr %a")
                .unwrap_or_else(|| panic!("no load out of the slot: {ir}"));
            // The regression direction. A `store ptr null, ptr %a` here is
            // exactly the defect: the slot stops naming live storage while
            // user finalizers are still running.
            assert!(
                !block.contains("store ptr null, ptr %a"),
                "the returning block cleared the slot: {ir}"
            );
            // What the statement does instead: record the pointer as
            // pending, so a finalizer that rebinds the name cannot release
            // it out from under the host.
            let pending_at = block
                .find("store ptr %load, ptr %pending_return_buffer_slot")
                .unwrap_or_else(|| panic!("the pending return was not recorded: {ir}"));
            let handoff_at = block
                .find("store ptr %load, ptr %str_epilogue_ret_slot")
                .unwrap_or_else(|| panic!("the loaded view was not returned: {ir}"));
            assert!(load_at < pending_at, "{ir}");
            assert!(pending_at < handoff_at, "{ir}");
            // ...and the epilogue releases the slot only when it does not
            // hold the value being returned. The `select` is the transfer:
            // `pycc_rt_buffer_f64_free`'s documented null no-op is what the
            // returning path gets instead of a free.
            let epilogue = ir
                .split("pycc_str_epilogue:")
                .nth(1)
                .unwrap_or_else(|| panic!("no owned-slot epilogue: {ir}"));
            assert!(
                epilogue
                    .contains("%buffer_epilogue_returned = load ptr, ptr %str_epilogue_ret_slot"),
                "{ir}"
            );
            assert!(
                epilogue.contains(
                    "%buffer_epilogue_transferred = icmp eq ptr %buffer_epilogue_live, \
                     %buffer_epilogue_returned"
                ),
                "{ir}"
            );
            assert!(
                epilogue.contains(
                    "%buffer_epilogue_release = select i1 %buffer_epilogue_transferred, ptr null, \
                     ptr %buffer_epilogue_live"
                ),
                "{ir}"
            );
            assert!(
                epilogue
                    .contains("call void @pycc_rt_buffer_f64_free(ptr %buffer_epilogue_release)"),
                "{ir}"
            );
        },
    );
}

#[test]
fn an_abandoned_pending_return_is_released_only_once_nothing_else_names_it() {
    // Part 2b of #1142 (#1164), the arm the per-slot loop cannot reach. A
    // `finally` that rebinds the returned name runs
    // `free_buffer_slot_before_store` *after* the `return` statement made
    // that pointer pending, so the store site must keep its hands off it --
    // and the pending record then becomes the pointer's sole owner, which
    // the epilogue has to release when a raising finalizer abandoned the
    // return.
    //
    // The conjunction is what makes both safe. `old == pending` alone is
    // true for the *first* allocation into a never-assigned slot, where both
    // are the entry null: that raised the orphan flag on a frame that never
    // rebound anything, and the epilogue then released a pointer the slot
    // loop had already released -- a double free on every exception exit.
    //
    // Two `ndarray` allocations rather than one because the second is what
    // reaches the store site's release path at all; the shape is
    // `a_reallocating_function_frees_the_old_view_before_it_stores_the_new_one`'s,
    // lifted to a `memoryview`-returning frame.
    compile_ext_items_checking_ir(
        "buffer_returned_rebound",
        vec![MirItem::Function {
            name: "allocate".to_string(),
            params: vec![],
            return_ty: Ty::MemoryView,
            body: vec![
                buffer_alloc_a(),
                buffer_alloc_a(),
                MirStmt::Return(Some(MirExpr::Name {
                    name: "a".to_string(),
                    ty: Ty::MemoryView,
                })),
            ],
        }],
        |ir| {
            // The store site's guard, both halves.
            assert!(
                ir.contains("%buffer_store_old_is_live = icmp ne ptr %old_buffer, null"),
                "the null guard is missing: {ir}"
            );
            assert!(
                ir.contains(
                    "%buffer_store_matches_pending = icmp eq ptr %old_buffer, \
                     %pending_return_buffer"
                ),
                "{ir}"
            );
            assert!(
                ir.contains(
                    "%buffer_store_is_pending = and i1 %buffer_store_old_is_live, \
                     %buffer_store_matches_pending"
                ),
                "the guard is not a conjunction: {ir}"
            );
            assert!(
                ir.contains(
                    "%buffer_free_old_release = select i1 %buffer_store_is_pending, ptr null, \
                     ptr %old_buffer"
                ),
                "{ir}"
            );
            // ...and the orphan flag it raises, which is the only thing that
            // lets the epilogue distinguish "still in a slot" from "rebound
            // away".
            assert!(
                ir.contains(
                    "%pending_return_orphaned = select i1 %buffer_store_is_pending, i8 1, \
                     i8 %pending_return_was_orphaned"
                ),
                "{ir}"
            );
            // The epilogue's trailing release, gated on that flag *and* on
            // the pending pointer not being the returned one.
            let epilogue = ir
                .split("pycc_str_epilogue:")
                .nth(1)
                .unwrap_or_else(|| panic!("no owned-slot epilogue: {ir}"));
            assert!(
                epilogue.contains(
                    "%buffer_epilogue_was_orphaned = icmp ne i8 %buffer_epilogue_orphaned, 0"
                ),
                "{ir}"
            );
            assert!(
                epilogue.contains(
                    "%buffer_epilogue_pending_keep = select i1 \
                     %buffer_epilogue_pending_transferred, ptr null, ptr %buffer_epilogue_pending"
                ),
                "{ir}"
            );
            assert!(
                epilogue.contains(
                    "%buffer_epilogue_pending_release = select i1 %buffer_epilogue_was_orphaned, \
                     ptr %buffer_epilogue_pending_keep, ptr null"
                ),
                "the trailing release is not gated on the orphan flag: {ir}"
            );
            assert!(
                epilogue.contains(
                    "call void @pycc_rt_buffer_f64_free(ptr %buffer_epilogue_pending_release)"
                ),
                "{ir}"
            );
        },
    );
}

#[test]
fn a_superseding_return_releases_the_orphaned_record_it_replaces_and_clears_the_flag() {
    // Part 2b of #1142 (#1164), round 2: the pending record's own state
    // machine rather than the control-flow shapes that reach it.
    //
    // `(pending, orphaned)` is one two-field state whose invariant is
    // "`orphaned != 0` means the record is the pointer's sole owner". It has
    // exactly two mutators. `free_buffer_slot_before_store` is the one the
    // test above pins; this is the other. A `return` that supersedes an
    // earlier one -- which `finally: while True: return a` reaches, because
    // the HIR resets its `in_finally` state on loop entry and the `L0001`
    // refusal of a bare `return` in `finally` therefore does not cover it --
    // used to store the new pointer raw, which broke the invariant twice
    // over: the superseded pointer's sole record was overwritten and its
    // storage leaked, and the surviving flag then described a pointer that
    // was no longer pending, so the epilogue's trailing release freed the
    // *new* pointer that the owned-slot loop had already released.
    //
    // The transition is pinned rather than the shape: with one
    // invariant-preserving mutator, a third and a fourth `return` are the
    // same transition applied again, so no enumeration of syntactic paths
    // carries the correctness argument. A single-`return` frame emits the
    // same instructions (`stale` is statically false there at runtime, not
    // in the IR), which is why this fixture needs no `finally` to pin them.
    compile_ext_items_checking_ir(
        "buffer_return_supersedes",
        vec![MirItem::Function {
            name: "allocate".to_string(),
            params: vec![],
            return_ty: Ty::MemoryView,
            body: vec![
                buffer_alloc_a(),
                MirStmt::Return(Some(MirExpr::Name {
                    name: "a".to_string(),
                    ty: Ty::MemoryView,
                })),
            ],
        }],
        |ir| {
            let block = ir
                .split("global_ready:")
                .nth(1)
                .unwrap_or_else(|| panic!("no returning block: {ir}"));
            // The predecessor is read back before it can be overwritten.
            let read_at = block
                .find("%superseded_return_buffer = load ptr, ptr %pending_return_buffer_slot")
                .unwrap_or_else(|| panic!("the superseded record is not read: {ir}"));
            assert!(
                block.contains(
                    "%superseded_return_orphaned = load i8, ptr %pending_return_orphaned_slot"
                ),
                "{ir}"
            );
            // Both halves of the release guard. Gating on the flag is what
            // keeps an *unorphaned* predecessor -- still held by its own
            // slot, which the epilogue's loop releases -- from being freed
            // twice; `try: return a` with `finally: while True: return b` is
            // that shape. The inequality makes the symmetric state
            // unreachable rather than merely improbable.
            assert!(
                block.contains(
                    "%superseded_return_was_orphaned = icmp ne i8 %superseded_return_orphaned, 0"
                ),
                "the release is not gated on the orphan flag: {ir}"
            );
            assert!(
                block.contains(
                    "%superseded_return_differs = icmp ne ptr %superseded_return_buffer, %load"
                ),
                "the release does not exclude the value being returned: {ir}"
            );
            assert!(
                block.contains(
                    "%superseded_return_is_stale = and i1 %superseded_return_was_orphaned, \
                     %superseded_return_differs"
                ),
                "the guard is not a conjunction: {ir}"
            );
            assert!(
                block.contains(
                    "%superseded_return_release = select i1 %superseded_return_is_stale, \
                     ptr %superseded_return_buffer, ptr null"
                ),
                "{ir}"
            );
            let free_at = block
                .find("call void @pycc_rt_buffer_f64_free(ptr %superseded_return_release)")
                .unwrap_or_else(|| panic!("the superseded record is not released: {ir}"));
            // ...and the transition completes: the new pointer is recorded
            // and the flag it inherited is cleared. Leaving the flag raised
            // is the double-free half of the defect, so the clear is pinned
            // separately from the release and ordered after it.
            let store_at = block
                .find("store ptr %load, ptr %pending_return_buffer_slot")
                .unwrap_or_else(|| panic!("the pending return was not recorded: {ir}"));
            let clear_at = block
                .find("store i8 0, ptr %pending_return_orphaned_slot")
                .unwrap_or_else(|| panic!("the inherited orphan flag was not cleared: {ir}"));
            assert!(read_at < free_at, "{ir}");
            assert!(free_at < store_at, "{ir}");
            assert!(store_at < clear_at, "{ir}");
        },
    );
}

#[test]
fn returning_a_buffer_parameter_leaves_its_slot_alone() {
    // The other provenance. A `memoryview` **parameter**'s storage belongs
    // to the host's exporter: the slot is not in the frame's release list,
    // so the frame must neither clear it nor free it -- an edit that treated
    // every buffer slot as artifact-owned would either lose the host's
    // pointer or hand its exporter a pointer this frame had freed.
    //
    // `pycc_types` refuses this shape at the source level
    // (`reject_memoryview_read`'s parameter arm), so the MIR below is hand
    // built: the assertion guards the codegen arm, not a reachable program.
    //
    // Part 2b of #1142 (#1164) narrows what this can assert. The pending
    // record and the epilogue both exist only for a frame that owns at least
    // one buffer *local*; this frame owns none, so the assertions are that
    // no owned-slot machinery appears at all around a parameter.
    compile_ext_items_checking_ir(
        "buffer_parameter_returned",
        buffer_fn_items(
            vec![MirStmt::Return(Some(MirExpr::Name {
                name: "b".to_string(),
                ty: Ty::MemoryView,
            }))],
            Ty::MemoryView,
        ),
        |ir| {
            assert!(ir.contains("define ptr @pyfn_element("), "{ir}");
            // The slot is null-initialized at entry like every other one, so
            // the assertion is that *no second* null store appears -- and
            // that the one that does precedes the load feeding the return.
            assert_eq!(
                ir.matches("store ptr null, ptr %b").count(),
                1,
                "the host's buffer pointer was cleared: {ir}"
            );
            let null_at = ir
                .find("store ptr null, ptr %b")
                .unwrap_or_else(|| panic!("no entry initialization: {ir}"));
            let load_at = ir
                .find("%load = load ptr, ptr %b")
                .unwrap_or_else(|| panic!("no load out of the slot: {ir}"));
            assert!(null_at < load_at, "{ir}");
            // Nothing frees the host's buffer, and the frame allocates no
            // pending-return record for a parameter it does not own.
            assert!(
                !ir.contains("call void @pycc_rt_buffer_f64_free"),
                "the host's buffer was released by the callee: {ir}"
            );
            assert!(
                !ir.contains("pending_return_buffer_slot"),
                "a frame that owns no buffer local got a pending-return record: {ir}"
            );
        },
    );
}

#[test]
#[should_panic(expected = "a memoryview argument is not supported yet")]
fn passing_a_memoryview_as_a_call_argument_is_an_internal_error() {
    // Reached through real MIR rather than a direct call, because this arm
    // lives inside `build_call_to_with_leading_args`' per-argument loop and
    // only a `MirExpr` that *evaluates* to `Scalar::MemoryView` selects it.
    // A `memoryview` parameter exists only on an `--ext` export, and
    // passing the name on is a bare read refused with `C0001`, so the
    // deliberately mistyped `int` parameter below is unreachable from
    // source.
    let mut items = buffer_fn_items(
        vec![MirStmt::ExprStmt(MirExpr::Call {
            callee: "takes_int".to_string(),
            args: vec![MirExpr::Name {
                name: "b".to_string(),
                ty: Ty::MemoryView,
            }],
            ty: Ty::None,
        })],
        Ty::None,
    );
    items.push(MirItem::Function {
        name: "takes_int".to_string(),
        params: vec![("n".to_string(), Ty::Int)],
        return_ty: Ty::None,
        body: vec![MirStmt::Return(None)],
    });
    compile_ext_items_checking_ir("buffer_call_argument", items, |_| {
        unreachable!("codegen should have panicked")
    });
}
