//! Owned buffer allocation, reallocation and slot frees.

use super::*;

/// An `--ext` module holding one parameterless function with `body`.
fn owned_buffer_fn_items(body: Vec<MirStmt>) -> Vec<MirItem> {
    vec![MirItem::Function {
        name: "allocate".to_string(),
        params: vec![],
        return_ty: Ty::None,
        body,
    }]
}

#[test]
fn a_buffer_allocation_untags_its_length_and_calls_the_runtime_allocator() {
    // The length reaches the runtime as a raw `i64`: the allocator takes an
    // element count, not a tagged `int`, and handing it the tagged form
    // would allocate twice the requested length.
    //
    // #1166 round 8: the decoder is `pycc_rt_buffer_alloc_untag_len`, *not*
    // the shared `pycc_rt_int_untag_checked` every other D-141 operand uses.
    // That one `panic!`s on a bigint, and the panic becomes a process abort
    // at its own `extern "C"` boundary -- the host CPython interpreter's,
    // for a D-244 `ext` artifact. Asserted two-directionally: the
    // non-aborting decoder is present *and* the aborting one is absent, so a
    // half-applied edit that emitted both cannot pass.
    compile_ext_items_checking_ir(
        "buffer_allocation",
        owned_buffer_fn_items(vec![buffer_alloc_a()]),
        |ir| {
            assert!(
                ir.contains("call i64 @pycc_rt_buffer_alloc_untag_len"),
                "{ir}"
            );
            // A `declare` for every runtime function is emitted whether or
            // not it is called, so the negative is on the *call*.
            assert!(!ir.contains("call i64 @pycc_rt_int_untag_checked"), "{ir}");
            assert!(ir.contains("call ptr @pycc_rt_buffer_f64_alloc"), "{ir}");
        },
    );
}

#[test]
fn a_buffer_allocations_length_decoder_can_branch_away_before_it_allocates() {
    // #1166 round 8, the P1 this arm exists to close. The decoder's own
    // bigint refusal raises `OverflowError` (D-173) and returns the
    // type-valid sentinel `0`, and `0` is a *valid* length -- so if the
    // allocator call were reachable from the decode without an intervening
    // branch, the refusal would allocate a zero-length view that
    // `MirStmt::Assign` never stores, leaking it.
    //
    // `guard_statement_effects` repositions the builder into a fresh
    // continuation block, so the property is a *block* separation, not mere
    // text order: pinned here as a conditional branch plus a new label
    // between the decode and the allocator call. A presence-only assertion
    // on the guard passes with or without that separation.
    compile_ext_items_checking_ir(
        "buffer_alloc_decoder_branch",
        owned_buffer_fn_items(vec![buffer_alloc_a()]),
        |ir| {
            let decode = ir
                .find("call i64 @pycc_rt_buffer_alloc_untag_len")
                .unwrap_or_else(|| panic!("the length decode should be emitted: {ir}"));
            let alloc = ir[decode..]
                .find("@pycc_rt_buffer_f64_alloc")
                .map(|offset| decode + offset)
                .unwrap_or_else(|| panic!("the allocator call should follow the decode: {ir}"));
            let between = &ir[decode..alloc];
            assert!(between.contains("@pycc_rt_exception_active"), "{ir}");
            assert!(between.contains("br i1 "), "{ir}");
            assert!(between.contains("effect_exc_cont"), "{ir}");
        },
    );
}

#[test]
fn an_allocating_function_frees_its_buffer_slot_in_the_epilogue() {
    // The whole reason the producer is admitted in exactly one position: the
    // value cannot escape the frame, so the frame can free it
    // unconditionally on the way out. Without the epilogue this is one
    // leaked allocation per call in a long-lived host process, which is the
    // D-107 list/dict/set precedent this issue explicitly does not follow.
    compile_ext_items_checking_ir(
        "buffer_epilogue_free",
        owned_buffer_fn_items(vec![buffer_alloc_a(), MirStmt::Return(None)]),
        |ir| {
            assert!(ir.contains("call void @pycc_rt_buffer_f64_free"), "{ir}");
        },
    );
}

#[test]
fn a_reallocating_function_frees_the_old_view_before_it_stores_the_new_one() {
    // D-074's free-before-overwrite, mirrored from `decref_str_slot_before_store`:
    // a second `a = ndarray(4)` overwrites the slot, so the view the slot
    // already holds has to be released first -- the epilogue only ever sees
    // the last one. Two frees is the signature of the fix: one before the
    // second store, one in the epilogue.
    compile_ext_items_checking_ir(
        "buffer_realloc_free",
        owned_buffer_fn_items(vec![buffer_alloc_a(), buffer_alloc_a()]),
        |ir| {
            assert!(
                ir.matches("call void @pycc_rt_buffer_f64_free").count() >= 2,
                "{ir}"
            );
        },
    );
}

#[test]
fn a_refused_reallocation_never_reaches_the_free_before_its_own_store() {
    // #1166 round 9 review, the reassign-then-refused-allocation ordering.
    // `a = ndarray(4); a = ndarray(-1)` is the one shape neither neighbour
    // covers: `a_reallocating_function_frees_the_old_view_before_it_stores_the_new_one`
    // reallocates *successfully*, and the hosted `alloc_then_raise` refuses
    // on a *different* name, where the slot being overwritten is null anyway.
    // Here the slot holds a live view when the second allocation is refused,
    // so freeing it before the refusal branched away would release a view the
    // exception-exit epilogue then frees again -- a double free.
    //
    // What makes that unreachable is block separation, not text order:
    // `expression_can_set_exception` answers `true` for `MirExpr::BufferAlloc`,
    // so `emit_expr`'s post-call `guard_statement_effects` repositions the
    // builder into a fresh continuation block before `MirStmt::Assign` reaches
    // `free_buffer_slot_before_store`. Pinned the way the round-8 decoder test
    // pins its own separation: the window between the second allocator call
    // and the next free must carry the pending-state read, a conditional
    // branch, and the continuation label. A bare "no free here" assertion
    // would be unkillable -- the mutation that matters moves the free
    // *upstream*, out of any such window -- and "a free exists afterwards"
    // is confounded by the epilogue's own free.
    compile_ext_items_checking_ir(
        "buffer_refused_realloc_order",
        owned_buffer_fn_items(vec![buffer_alloc_a(), buffer_alloc_a()]),
        |ir| {
            let first = ir
                .find("@pycc_rt_buffer_f64_alloc")
                .unwrap_or_else(|| panic!("the first allocator call should be emitted: {ir}"));
            let second = ir[first + 1..]
                .find("@pycc_rt_buffer_f64_alloc")
                .map(|offset| first + 1 + offset)
                .unwrap_or_else(|| panic!("the second allocator call should be emitted: {ir}"));
            let free = ir[second..]
                .find("call void @pycc_rt_buffer_f64_free")
                .map(|offset| second + offset)
                .unwrap_or_else(|| panic!("a free should follow the second allocation: {ir}"));
            let between = &ir[second..free];
            assert!(between.contains("@pycc_rt_exception_active"), "{ir}");
            assert!(between.contains("br i1 "), "{ir}");
            // Substring, not an exact label: LLVM uniquifies the name once
            // two statements each emit a continuation block.
            assert!(between.contains("effect_exc_cont"), "{ir}");
            // The window above pins the separation but not *which* free it
            // landed on: hoisting `free_buffer_slot_before_store` above
            // `emit_expr` in the `MirStmt::Assign` arm would move the second
            // statement's free upstream of the whole window, leaving the
            // epilogue's free to satisfy it. Counting what precedes the
            // second allocation closes that: exactly one, statement 1's
            // free-before-store over a slot the entry block null-initialized.
            assert_eq!(
                ir[..second]
                    .matches("call void @pycc_rt_buffer_f64_free")
                    .count(),
                1,
                "{ir}"
            );
        },
    );
}

/// An `--ext` module holding one `n: int` function with `body`, for the
/// arms that need a *parameter* to build a borrowed length out of.
fn owned_buffer_fn_items_taking_n(body: Vec<MirStmt>) -> Vec<MirItem> {
    vec![MirItem::Function {
        name: "allocate".to_string(),
        params: vec![("n".to_string(), Ty::Int)],
        return_ty: Ty::None,
        body,
    }]
}

/// `a = ndarray(<len>)` over an arbitrary length expression.
fn buffer_alloc_a_of(len: MirExpr) -> MirStmt {
    MirStmt::Assign {
        target: "a".to_string(),
        value: MirExpr::BufferAlloc { len: Box::new(len) },
    }
}

#[test]
fn a_buffer_allocations_length_temporary_is_released_before_the_guard() {
    // #1166 round 11, finding 1. `a = ndarray(n + 1)` with `n == 2 ** 62 - 1`
    // evaluates the length to a freshly owned `BigIntObj`, and the decoder
    // below refuses it with a *catchable* `OverflowError`. Every exception
    // edge out of this arm therefore abandons a birth reference unless the
    // arm retires it first, and repeatedly catching that exception leaked one
    // bigint per call in the host process.
    //
    // Asserted as an ordering rather than as presence, for the reason the
    // round-8 decoder arm above states: a release emitted *after*
    // `guard_statement_effects` is skipped by exactly the edges that leak,
    // and a presence-only assertion passes either way. The window is
    // therefore the decode through the pre-allocator guard's own pending
    // read -- which is also what pins the release ahead of the stale-pending
    // short circuit, not merely ahead of the allocator.
    compile_ext_items_checking_ir(
        "buffer_alloc_len_release",
        owned_buffer_fn_items_taking_n(vec![buffer_alloc_a_of(MirExpr::BinOp {
            op: BinOpKind::Add,
            left: Box::new(MirExpr::Name {
                name: "n".to_string(),
                ty: Ty::Int,
            }),
            right: Box::new(MirExpr::IntLiteral(1)),
            ty: Ty::Int,
        })]),
        |ir| {
            let decode = ir
                .find("call i64 @pycc_rt_buffer_alloc_untag_len")
                .unwrap_or_else(|| panic!("the length decode should be emitted: {ir}"));
            let guard = ir[decode..]
                .find("@pycc_rt_exception_active")
                .map(|offset| decode + offset)
                .unwrap_or_else(|| {
                    panic!("the pre-allocator guard should follow the decode: {ir}")
                });
            assert!(
                ir[decode..guard].contains("@pycc_rt_bigint_release"),
                "{ir}"
            );
        },
    );
}

#[test]
fn a_borrowed_buffer_length_is_not_released_by_the_allocation() {
    // The other direction of the arm above, and the one that makes the
    // release a classification rather than an unconditional emission: in
    // `a = ndarray(n)` the length is a plain `Name` read, which owns no
    // reference of its own -- `n`'s own storage still refers to the word.
    // Releasing it here would be a double release, not a leak, so this is
    // pinned in the same window the positive arm asserts over.
    compile_ext_items_checking_ir(
        "buffer_alloc_borrowed_len",
        owned_buffer_fn_items_taking_n(vec![buffer_alloc_a_of(MirExpr::Name {
            name: "n".to_string(),
            ty: Ty::Int,
        })]),
        |ir| {
            let decode = ir
                .find("call i64 @pycc_rt_buffer_alloc_untag_len")
                .unwrap_or_else(|| panic!("the length decode should be emitted: {ir}"));
            let alloc = ir[decode..]
                .find("@pycc_rt_buffer_f64_alloc")
                .map(|offset| decode + offset)
                .unwrap_or_else(|| panic!("the allocator call should follow the decode: {ir}"));
            assert!(
                !ir[decode..alloc].contains("@pycc_rt_bigint_release"),
                "{ir}"
            );
        },
    );
}

#[test]
fn a_buffer_allocation_checks_the_pending_state_before_it_allocates() {
    // #1166 review finding F5. `emit_expr` guards *after*
    // `emit_expr_unchecked` returns, so an exception already pending when
    // this node is reached -- set by an operation
    // `expression_can_set_exception` classifies `false` and therefore leaves
    // unguarded, such as the `ListPop` in `a = ndarray(xs.pop())` -- let the
    // allocation succeed and only then branched to the handler, before
    // `MirStmt::Assign` stored the pointer into the frame's owned slot. The
    // slot kept its entry null, so nothing freed the view: one leaked buffer
    // per call.
    //
    // Asserted as an ordering *between* the length untag and the allocator
    // call rather than as the mere presence of a guard: the post-call guard
    // `expression_can_set_exception` already emitted makes a presence-only
    // assertion pass with or without the fix.
    compile_ext_items_checking_ir(
        "buffer_alloc_pending_pre_guard",
        owned_buffer_fn_items(vec![buffer_alloc_a()]),
        |ir| {
            let untag = ir
                .find("buffer_untag_alloc_len")
                .unwrap_or_else(|| panic!("the length untag should be emitted: {ir}"));
            let alloc = ir[untag..]
                .find("@pycc_rt_buffer_f64_alloc")
                .map(|offset| untag + offset)
                .unwrap_or_else(|| panic!("the allocator call should follow the untag: {ir}"));
            assert!(
                ir[untag..alloc].contains("@pycc_rt_exception_active"),
                "{ir}"
            );
        },
    );
}

#[test]
fn a_buffer_parameter_is_never_pushed_into_the_owned_slot_frame() {
    // The load-bearing negative. A `memoryview` *parameter* is storage the
    // host lent for exactly one call, so freeing it in the epilogue would
    // hand the host's allocator a pointer it still owns. The `str` precedent
    // this frame is modelled on *does* push parameter slots, so the omission
    // is deliberate rather than incidental and is pinned here.
    compile_ext_items_checking_ir(
        "buffer_param_not_owned",
        buffer_fn_items(vec![MirStmt::Return(Some(buffer_get_b_i()))], Ty::Float),
        |ir| {
            assert!(!ir.contains("call void @pycc_rt_buffer_f64_free"), "{ir}");
        },
    );
}

#[test]
#[should_panic(expected = "has a non-buffer storage slot")]
fn freeing_a_non_buffer_slot_before_a_store_is_an_internal_error() {
    // `free_buffer_slot_before_store` is reached only from the
    // `Ty::MemoryView` arm of `MirStmt::Assign`, so a slot of any other type
    // is a lowering defect. Pinned directly, on the same convention as the
    // defensive `Scalar::MemoryView` tests above: an honest panic in the
    // function that owns the invariant beats a silent free of an `i64` slot.
    let context = Context::create();
    let (module, rt) = list_scalar_panic_fixture(&context);
    let builder = context.create_builder();
    let function = module.add_function(
        "free_wrong_slot",
        context.void_type().fn_type(&[], false),
        None,
    );
    builder.position_at_end(context.append_basic_block(function, "entry"));
    let ptr = builder
        .build_alloca(context.i64_type(), "a")
        .expect("build_alloca should not fail for a fresh block");
    let locals = HashMap::from([(
        "a".to_string(),
        StorageSlot {
            ptr,
            ty: Ty::Int,
            initialized: None,
        },
    )]);
    free_buffer_slot_before_store(&context, &builder, &rt, &locals, "a");
}

/// A native program checks the pending-exception state with one inline load
/// of the process-wide flag; an `ext` module, which CPython may enter from
/// several threads, calls the thread-local accessor instead.
#[test]
fn only_a_native_module_loads_the_process_wide_pending_flag() {
    let raising = || {
        vec![MirItem::TopLevelStmt(MirStmt::ExprStmt(MirExpr::BinOp {
            op: pycc_mir::BinOpKind::FloorDiv,
            left: Box::new(MirExpr::IntLiteral(1)),
            right: Box::new(MirExpr::IntLiteral(0)),
            ty: Ty::Int,
        }))]
    };
    compile_items_checking_ir("exc_check_native", raising(), |ir| {
        assert!(ir.contains("load i8, ptr @pycc_rt_exc_pending"), "{ir}");
        assert!(!ir.contains("call i8 @pycc_rt_exception_active()"), "{ir}");
    });
    compile_ext_items_checking_ir("exc_check_ext", raising(), |ir| {
        assert!(ir.contains("call i8 @pycc_rt_exception_active()"), "{ir}");
        assert!(!ir.contains("load i8, ptr @pycc_rt_exc_pending"), "{ir}");
    });
}
