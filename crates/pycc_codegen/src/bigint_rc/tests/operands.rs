//! Refcount guards on binop/comparison/tuple operands and their exception edges.

use super::*;

// -----------------------------------------------------------------
// #638 (D-208) codegen-depth: releasing a bigint temporary on the
// exception-unwinding edge.
//
// `tests/issue_638_bigint_exception_release.rs` proves the net effect
// through the real CLI (`pycc run`'s value-correctness assertions and
// `pycc build`'s peak-RSS marginal comparison), but neither can see the
// structural property `guard_statement_effects`'s own `#638` doc
// comment describes: an `effect_exc_unwind` block, guarded exactly like
// every other `guard_statement_effects` site, releasing the pending
// stack's snapshot before branching to the installed exception target
// -- and, symmetrically, that this extra block is never emitted at all
// when nothing is pending (the codegen-time check that keeps the
// D-084/D-140 nbody hot loop's own guard sites at their original
// two-block shape). These follow the refcount-call cluster's own
// `compile_to_object_with_observer` convention immediately above.
// -----------------------------------------------------------------

/// Compiles `mir` and returns the emitted module's textual IR.
///
/// Skips `guarded_bigint_refcount_calls`'s own per-call guard-chain
/// proof (this cluster is checking for the presence/absence of an
/// entire extra block, not validating every call's guard) but keeps
/// its D-029 `LLVMString`-disposal discipline: route the IR through
/// `llvm_string_to_owned` rather than let `print_to_string`'s return
/// value drop on its own.
fn emitted_ir(label: &str, mir: &MirModule) -> String {
    let dir = pycc_scratch::ScratchDir::new(label).expect("failed to create scratch dir");
    let obj_path = dir.join(format!("{label}.o"));
    let mut ir = String::new();
    let mut observer = |module: &inkwell::module::Module<'_>, _| {
        ir = llvm_string_to_owned(module.print_to_string());
    };
    compile_to_object_with_observer(
        mir,
        &obj_path,
        &CompileOptions::default(),
        Some(&mut observer),
    )
    .expect("codegen should succeed");
    ir
}

#[test]
fn an_outer_binops_owned_left_operand_is_released_on_the_inner_binops_exception_edge() {
    // `(n + n) + (1 // n)`: the outer `+`'s left operand is a fresh,
    // owned `BinOp` result -- `int_temporary_word` classifies it as a
    // pending release candidate before the right operand (`1 // n`, a
    // `FloorDiv`, one of `expression_can_set_exception`'s three
    // exception-settable `BinOp`s) is evaluated. Reaching `1 // n`'s own
    // `guard_statement_effects` call with that word still pending must
    // emit an `effect_exc_unwind` block that releases it.
    let owned_sum = MirExpr::BinOp {
        op: pycc_mir::BinOpKind::Add,
        left: Box::new(int_name("n")),
        right: Box::new(int_name("n")),
        ty: Ty::Int,
    };
    let raising_floordiv = MirExpr::BinOp {
        op: pycc_mir::BinOpKind::FloorDiv,
        left: Box::new(MirExpr::IntLiteral(1)),
        right: Box::new(int_name("n")),
        ty: Ty::Int,
    };
    let mir = MirModule {
        items: vec![MirItem::Function {
            name: "outer_binop_exception_edge".to_string(),
            params: vec![("n".to_string(), Ty::Int)],
            return_ty: Ty::Int,
            body: vec![MirStmt::Return(Some(MirExpr::BinOp {
                op: pycc_mir::BinOpKind::Add,
                left: Box::new(owned_sum),
                right: Box::new(raising_floordiv),
                ty: Ty::Int,
            }))],
        }],
        class_defs: Vec::new(),
    };
    let ir = emitted_ir("bigint_rc_exc_unwind_present", &mir);
    assert!(
        ir.contains("effect_exc_unwind"),
        "a pending owned operand at the exception guard must emit the \
         #638 unwind block; got:\n{ir}"
    );
    // `contains()` above matches the branch instruction's own label
    // reference (`label %effect_exc_unwind`) as readily as the block's
    // definition, so isolate the block's own body by its label
    // *definition* line specifically (an unindented `effect_exc_unwind:`
    // line, per `guarded_bigint_refcount_calls_in_one_function`'s own
    // convention for recognizing a block label).
    //
    // This cannot split on the next `\neffect_exc_cont` occurrence: in
    // `guard_statement_effects`, `effect_exc_cont` is *created* before
    // `effect_exc_unwind` (so it is appended, and therefore emitted in
    // the IR text, first) whenever the pending-releases stack is
    // non-empty -- exactly this test's fixture. For a single guard site
    // that puts the only `effect_exc_cont` occurrence *before*
    // `effect_exc_unwind:` in the string, so splitting the
    // already-past-that-point substring on `\neffect_exc_cont` would
    // never find a second occurrence and `.next()` would silently hand
    // back the rest of the entire module -- including the fallthrough
    // path's own D-181 release calls -- rather than failing loudly.
    // Instead, treat `effect_exc_unwind` as the root of its own small
    // control-flow subgraph and walk it structurally: the block itself
    // ends in a *conditional* branch (`emit_bigint_refcount_call`'s own
    // heap-tag guard, per its doc comment on `guarded_bigint_refcount_calls`
    // above), whose taken arm (`bigint_rc_call*`) holds the actual
    // release and whose not-taken arm (`bigint_rc_cont*`) re-joins to
    // branch on to the exception target. Parse every basic block's body
    // and outgoing edges once, then collect the closure reachable from
    // `effect_exc_unwind` -- this follows exactly the blocks
    // `guard_statement_effects`'s own snapshot loop emits, regardless of
    // where in the text another guard site's blocks happen to land.
    let mut block_bodies: std::collections::HashMap<&str, Vec<&str>> =
        std::collections::HashMap::new();
    let mut block_order: Vec<&str> = Vec::new();
    let mut current_block: Option<&str> = None;
    for raw in ir.lines() {
        let line = raw.trim();
        if !raw.starts_with([' ', '@', '%', '}', '!']) {
            if let Some((label, _)) = line.split_once(':') {
                current_block = Some(label);
                block_order.push(label);
                block_bodies.entry(label).or_default();
            }
            continue;
        }
        if let Some(label) = current_block {
            block_bodies.get_mut(label).unwrap().push(line);
        }
    }
    assert!(
        block_bodies.contains_key("effect_exc_unwind"),
        "the block label was just found by contains() above, so it \
         must parse as a block; got:\n{ir}"
    );
    // A block's outgoing edges, read off its own terminator line.
    let successors = |label: &str| -> Vec<&str> {
        let body = &block_bodies[label];
        let terminator = body
            .last()
            .expect("every parsed block has a terminator line");
        if let Some(rest) = terminator.strip_prefix("br i1 ") {
            rest.split(", label ")
                .skip(1)
                .map(|s| s.trim_start_matches('%'))
                .collect()
        } else if let Some(rest) = terminator.strip_prefix("br label ") {
            vec![rest.trim_start_matches('%')]
        } else {
            Vec::new()
        }
    };
    let mut visited: Vec<&str> = Vec::new();
    let mut stack = vec!["effect_exc_unwind"];
    while let Some(label) = stack.pop() {
        if visited.contains(&label) {
            continue;
        }
        visited.push(label);
        stack.extend(successors(label));
    }
    // Report the reachable blocks in the order they appear in the IR so
    // the assertion failure message below reads like a normal block
    // listing rather than a BFS/DFS visitation order.
    let unwind_block = block_order
        .iter()
        .filter(|label| visited.contains(label))
        .flat_map(|label| block_bodies[label].iter().copied())
        .collect::<Vec<&str>>()
        .join("\n");
    assert!(
        unwind_block.contains("call void @pycc_rt_bigint_release"),
        "the #638 unwind block must release the pending operand before \
         branching to the exception target; block body:\n{unwind_block}"
    );
}

#[test]
fn a_lone_raising_binop_with_nothing_pending_emits_no_unwind_block() {
    // `1 // n` alone: no sibling has already produced an owned `Ty::Int`
    // temporary before this `FloorDiv`'s own `guard_statement_effects`
    // call, so `rt.exceptions.pending_int_releases` is empty when that
    // call runs. This is the codegen-time check
    // `guard_statement_effects`'s own doc comment describes as what
    // keeps the D-084/D-140 nbody hot loop's guard sites at their
    // original two-block shape -- proven here directly rather than only
    // by the nbody throughput gate, which this environment cannot run
    // locally (no exact CPython 3.14.7 oracle available).
    let mir = MirModule {
        items: vec![MirItem::Function {
            name: "lone_raising_binop".to_string(),
            params: vec![("n".to_string(), Ty::Int)],
            return_ty: Ty::Int,
            body: vec![MirStmt::Return(Some(MirExpr::BinOp {
                op: pycc_mir::BinOpKind::FloorDiv,
                left: Box::new(MirExpr::IntLiteral(1)),
                right: Box::new(int_name("n")),
                ty: Ty::Int,
            }))],
        }],
        class_defs: Vec::new(),
    };
    let ir = emitted_ir("bigint_rc_exc_unwind_absent", &mir);
    assert!(
        !ir.contains("effect_exc_unwind"),
        "a guard site with nothing pending must keep its original \
         two-block shape, not gain an extra unwind block; got:\n{ir}"
    );
}

#[test]
fn an_early_tuple_elements_owned_word_is_released_on_a_later_elements_exception_edge() {
    // `(n + n, n, 1 // n)`: the first element is a fresh, owned `BinOp`
    // result -- `int_temporary_word` classifies it as a pending release
    // candidate before the third element (`1 // n`, a `FloorDiv`, one of
    // `expression_can_set_exception`'s three exception-settable
    // `BinOp`s) is evaluated. The second element (a bare `Name` read) is
    // a *borrowed* reference and must not itself be pushed. Reaching
    // `1 // n`'s own `guard_statement_effects` call with the first
    // element's word still pending must emit an `effect_exc_unwind`
    // block that releases it -- exactly mirroring
    // `an_outer_binops_owned_left_operand_is_released_on_the_inner_binops_exception_edge`
    // above, but for `MirExpr::TupleLiteral`'s own element loop rather
    // than `build_call_to_with_leading_args`'s or `BinOp`'s.
    let owned_sum = MirExpr::BinOp {
        op: pycc_mir::BinOpKind::Add,
        left: Box::new(int_name("n")),
        right: Box::new(int_name("n")),
        ty: Ty::Int,
    };
    let raising_floordiv = MirExpr::BinOp {
        op: pycc_mir::BinOpKind::FloorDiv,
        left: Box::new(MirExpr::IntLiteral(1)),
        right: Box::new(int_name("n")),
        ty: Ty::Int,
    };
    let mir = MirModule {
        items: vec![MirItem::Function {
            name: "tuple_literal_exception_edge".to_string(),
            params: vec![("n".to_string(), Ty::Int)],
            return_ty: Ty::Int,
            body: vec![MirStmt::Return(Some(MirExpr::Subscript {
                base: Box::new(MirExpr::TupleLiteral(vec![
                    owned_sum,
                    int_name("n"),
                    raising_floordiv,
                ])),
                index: Box::new(MirExpr::IntLiteral(0)),
            }))],
        }],
        class_defs: Vec::new(),
    };
    let ir = emitted_ir("bigint_rc_tuple_exc_unwind_present", &mir);
    assert!(
        ir.contains("effect_exc_unwind"),
        "a pending owned tuple element at a later element's exception \
         guard must emit the #638 unwind block; got:\n{ir}"
    );
    // Same structural walk as
    // `an_outer_binops_owned_left_operand_is_released_on_the_inner_binops_exception_edge`
    // above: isolate `effect_exc_unwind`'s own reachable closure rather
    // than splitting on the next `effect_exc_cont` occurrence, for the
    // identical reason that test's own comment explains.
    let mut block_bodies: std::collections::HashMap<&str, Vec<&str>> =
        std::collections::HashMap::new();
    let mut block_order: Vec<&str> = Vec::new();
    let mut current_block: Option<&str> = None;
    for raw in ir.lines() {
        let line = raw.trim();
        if !raw.starts_with([' ', '@', '%', '}', '!']) {
            if let Some((label, _)) = line.split_once(':') {
                current_block = Some(label);
                block_order.push(label);
                block_bodies.entry(label).or_default();
            }
            continue;
        }
        if let Some(label) = current_block {
            block_bodies.get_mut(label).unwrap().push(line);
        }
    }
    assert!(
        block_bodies.contains_key("effect_exc_unwind"),
        "the block label was just found by contains() above, so it \
         must parse as a block; got:\n{ir}"
    );
    let successors = |label: &str| -> Vec<&str> {
        let body = &block_bodies[label];
        let terminator = body
            .last()
            .expect("every parsed block has a terminator line");
        if let Some(rest) = terminator.strip_prefix("br i1 ") {
            rest.split(", label ")
                .skip(1)
                .map(|s| s.trim_start_matches('%'))
                .collect()
        } else if let Some(rest) = terminator.strip_prefix("br label ") {
            vec![rest.trim_start_matches('%')]
        } else {
            Vec::new()
        }
    };
    let mut visited: Vec<&str> = Vec::new();
    let mut stack = vec!["effect_exc_unwind"];
    while let Some(label) = stack.pop() {
        if visited.contains(&label) {
            continue;
        }
        visited.push(label);
        stack.extend(successors(label));
    }
    let unwind_block = block_order
        .iter()
        .filter(|label| visited.contains(label))
        .flat_map(|label| block_bodies[label].iter().copied())
        .collect::<Vec<&str>>()
        .join("\n");
    assert!(
        unwind_block.contains("call void @pycc_rt_bigint_release"),
        "the #638 unwind block must release the pending tuple element \
         before branching to the exception target; block body:\n{unwind_block}"
    );
}

#[test]
fn a_successfully_completed_tuple_literal_truncates_without_releasing() {
    // `(n + n, n)`: both elements complete evaluation successfully, so
    // the tuple-literal element loop's own `pending_int_releases` entry
    // for the first (owned) element must be truncated away -- never
    // released -- because ownership of that word transferred into the
    // aggregate's own field via `build_insert_value`. Releasing it here
    // instead would double-free the word `Ty::Tuple`'s own D-124
    // slot-death release (or the value's ordinary consumer) later
    // releases again.
    //
    // There is no exception-settable sibling in this fixture, so no
    // `effect_exc_unwind` block is emitted at all (mirroring
    // `a_lone_raising_binop_with_nothing_pending_emits_no_unwind_block`
    // above) -- the only way this test can fail is if a stray
    // unconditional release call for the first element leaked into the
    // normal fallthrough path.
    let owned_sum = MirExpr::BinOp {
        op: pycc_mir::BinOpKind::Add,
        left: Box::new(int_name("n")),
        right: Box::new(int_name("n")),
        ty: Ty::Int,
    };
    let mir = MirModule {
        items: vec![MirItem::Function {
            name: "tuple_literal_success_path".to_string(),
            params: vec![("n".to_string(), Ty::Int)],
            return_ty: Ty::Int,
            body: vec![MirStmt::Return(Some(MirExpr::Subscript {
                base: Box::new(MirExpr::TupleLiteral(vec![owned_sum, int_name("n")])),
                index: Box::new(MirExpr::IntLiteral(0)),
            }))],
        }],
        class_defs: Vec::new(),
    };
    let ir = emitted_ir("bigint_rc_tuple_success_path", &mir);
    assert!(
        !ir.contains("effect_exc_unwind"),
        "no sibling element here can set D-173's pending-exception \
         state, so no unwind block should be emitted; got:\n{ir}"
    );
    assert!(
        !ir.contains("call void @pycc_rt_bigint_release"),
        "the success path must truncate the pending entry for the \
         first (owned) tuple element without ever releasing it; got:\n{ir}"
    );
}

/// #146 Part 2 (D-181). `int_cmp` raises `OverflowError` and returns its
/// `0` sentinel on a bigint operand (Part C of #1038, formerly an abort),
/// and `Compare` is not an `expression_can_set_exception` checkpoint, so
/// unlike before Part C these releases *are* reachable with a real heap
/// word: they run against the freshly owned operands after the raise, and
/// freeing them there is exactly what keeps the pending exception from
/// leaking them. A runtime-value assertion therefore cannot pin this
/// shape, and the static retain/release counts below are the only place it
/// is observable at all.
///
/// `(n + n) < (n + n)`: each inner `+` has two borrowed `Name` operands
/// and releases neither, while the comparison itself consumes two
/// freshly built words and releases both.
#[test]
fn an_int_comparison_releases_both_of_its_freshly_built_operands() {
    let sum = || MirExpr::BinOp {
        op: pycc_mir::BinOpKind::Add,
        left: Box::new(int_name("n")),
        right: Box::new(int_name("n")),
        ty: Ty::Int,
    };
    let mir = MirModule {
        items: vec![MirItem::Function {
            name: "compare".to_string(),
            params: vec![("n".to_string(), Ty::Int)],
            return_ty: Ty::Bool,
            body: vec![MirStmt::Return(Some(MirExpr::Compare {
                op: pycc_mir::CmpOpKind::Lt,
                left: Box::new(sum()),
                right: Box::new(sum()),
                ty: Ty::Bool,
            }))],
        }],
        class_defs: Vec::new(),
    };
    let calls = refcount_calls_in("bigint_rc_compare_operands", &mir);
    let retains = calls
        .iter()
        .filter(|c| c.callee == "pycc_rt_bigint_retain")
        .count();
    let releases = calls
        .iter()
        .filter(|c| c.callee == "pycc_rt_bigint_release")
        .count();
    assert_eq!((retains, releases), (0, 2), "got {calls:?}");
}

/// The same for a `*` result. `int_mul`/`int_floordiv`/`int_floormod`/
/// `int_pow` all reject a bigint operand exactly like `int_cmp` does,
/// and all four reach the identical release site in `MirExpr::BinOp`'s
/// own `Ty::Int` arm, so one of them stands for the group.
#[test]
fn a_multiplying_binop_releases_both_of_its_freshly_built_operands() {
    let sum = || MirExpr::BinOp {
        op: pycc_mir::BinOpKind::Add,
        left: Box::new(int_name("n")),
        right: Box::new(int_name("n")),
        ty: Ty::Int,
    };
    let mir = MirModule {
        items: vec![MirItem::Function {
            name: "multiply".to_string(),
            params: vec![("n".to_string(), Ty::Int)],
            return_ty: Ty::Int,
            body: vec![MirStmt::Return(Some(MirExpr::BinOp {
                op: pycc_mir::BinOpKind::Mul,
                left: Box::new(sum()),
                right: Box::new(sum()),
                ty: Ty::Int,
            }))],
        }],
        class_defs: Vec::new(),
    };
    let calls = refcount_calls_in("bigint_rc_mul_operands", &mir);
    let retains = calls
        .iter()
        .filter(|c| c.callee == "pycc_rt_bigint_retain")
        .count();
    let releases = calls
        .iter()
        .filter(|c| c.callee == "pycc_rt_bigint_release")
        .count();
    assert_eq!((retains, releases), (0, 2), "got {calls:?}");
}

/// #146 Part 2 (D-181): the tuple-egress classification, from both
/// sides at once. `t[0]` is *borrowed*: under D-182 the tuple field
/// holds its own ingress reference, and a read hands the same word
/// back out without transferring it, so the reader owns nothing. The
/// `+` therefore releases only its literal operand -- releasing `t[0]`
/// too would retire the tuple's own reference on a read, which is
/// exactly the #633 direction-A use-after-free D-181 closed.
///
/// The `TupleLiteral` itself is where the two retains come from: `n`
/// is a borrowed `Name`, so D-182's ingress retain fires once per
/// element.
///
/// The literal is `2^62` rather than a small one on purpose: only an
/// out-of-range literal is materialized by a runtime
/// `pycc_rt_int_from_i64` call, so only that shape produces a word
/// whose heap-ness is unknown at compile time. An in-range literal is
/// a constant tagged smallint, `emit_bigint_refcount_call` skips it,
/// and the release this test is about would not exist to observe.
#[test]
fn a_tuple_element_operand_is_borrowed_while_a_literal_operand_is_owned() {
    let mir = MirModule {
        items: vec![MirItem::Function {
            name: "read_tuple".to_string(),
            params: vec![("n".to_string(), Ty::Int)],
            return_ty: Ty::Int,
            body: vec![
                MirStmt::Assign {
                    target: "t".to_string(),
                    value: MirExpr::TupleLiteral(vec![int_name("n"), int_name("n")]),
                },
                MirStmt::Return(Some(MirExpr::BinOp {
                    op: pycc_mir::BinOpKind::Add,
                    left: Box::new(MirExpr::Subscript {
                        base: Box::new(MirExpr::Name {
                            name: "t".to_string(),
                            ty: Ty::Tuple(Box::new(vec![Ty::Int, Ty::Int])),
                        }),
                        index: Box::new(MirExpr::IntLiteral(0)),
                    }),
                    right: Box::new(MirExpr::IntLiteral(4_611_686_018_427_387_904)),
                    ty: Ty::Int,
                })),
            ],
        }],
        class_defs: Vec::new(),
    };
    let calls = refcount_calls_in("bigint_rc_tuple_operand", &mir);
    let retains = calls
        .iter()
        .filter(|c| c.callee == "pycc_rt_bigint_retain")
        .count();
    let releases = calls
        .iter()
        .filter(|c| c.callee == "pycc_rt_bigint_release")
        .count();
    assert_eq!((retains, releases), (2, 1), "got {calls:?}");
}

/// D-181 decision 12: an in-range `IntLiteral` operand is a *constant*
/// tagged smallint, so `emit_bigint_refcount_call` must emit nothing at
/// all for it -- not even a guard LLVM would fold away.
///
/// This pins the early return mechanically. Without it the literal `1`
/// below still reaches the emitter, whose four guard halves inkwell then
/// constant-folds, leaving a `bigint_rc_call`/`bigint_rc_cont` pair
/// behind a branch whose condition is no longer an SSA definition. The
/// other operand is a `Name`, which is borrowed and never released, so
/// the literal is the function's only candidate refcount site and the
/// whole body must therefore be free of guard blocks.
#[test]
fn an_in_range_literal_operand_emits_no_refcount_guard_at_all() {
    let mir = MirModule {
        items: vec![MirItem::Function {
            name: "borrowed_plus_literal".to_string(),
            params: vec![("n".to_string(), Ty::Int)],
            return_ty: Ty::Int,
            body: vec![MirStmt::Return(Some(MirExpr::BinOp {
                op: pycc_mir::BinOpKind::Add,
                left: Box::new(int_name("n")),
                right: Box::new(MirExpr::IntLiteral(1)),
                ty: Ty::Int,
            }))],
        }],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("bigint_rc_const_literal")
        .expect("failed to create scratch dir");
    let obj_path = dir.join("bigint_rc_const_literal.o");
    let mut ir = String::new();
    // D-029: never let `print_to_string`'s `LLVMString` drop; route it
    // through `llvm_string_to_owned` exactly as the sibling tests do.
    let mut observer = |module: &inkwell::module::Module<'_>, _| {
        ir = llvm_string_to_owned(module.print_to_string());
    };
    compile_to_object_with_observer(
        &mir,
        &obj_path,
        &CompileOptions::default(),
        Some(&mut observer),
    )
    .expect("codegen should succeed");
    assert!(
        !ir.contains("bigint_rc_call"),
        "a constant word must not reach the guard emitter, but the IR \
         contains a `bigint_rc_call` block:\n{ir}"
    );
    assert!(
        !ir.contains("bigint_rc_cont"),
        "a constant word must not reach the guard emitter, but the IR \
         contains a `bigint_rc_cont` block:\n{ir}"
    );
}

/// The retain side of the same classification (#633 direction A):
/// binding a tuple element to an `int` local makes that local a second
/// owner of a word the tuple's supplier still holds, so the bind must
/// retain. Without this arm, overwriting the local released a reference
/// it never owned and freed the supplier's object out from under it.
#[test]
fn binding_a_tuple_element_to_an_int_local_retains_the_shared_word() {
    let mir = MirModule {
        items: vec![MirItem::Function {
            name: "bind_tuple".to_string(),
            params: vec![("n".to_string(), Ty::Int)],
            return_ty: Ty::Int,
            body: vec![
                MirStmt::Assign {
                    target: "t".to_string(),
                    value: MirExpr::TupleLiteral(vec![int_name("n"), int_name("n")]),
                },
                MirStmt::Assign {
                    target: "y".to_string(),
                    value: MirExpr::Subscript {
                        base: Box::new(MirExpr::Name {
                            name: "t".to_string(),
                            ty: Ty::Tuple(Box::new(vec![Ty::Int, Ty::Int])),
                        }),
                        index: Box::new(MirExpr::IntLiteral(0)),
                    },
                },
                MirStmt::Return(Some(int_name("y"))),
            ],
        }],
        class_defs: Vec::new(),
    };
    let calls = refcount_calls_in("bigint_rc_tuple_bind", &mir);
    let retains = calls
        .iter()
        .filter(|c| c.callee == "pycc_rt_bigint_retain")
        .count();
    let releases = calls
        .iter()
        .filter(|c| c.callee == "pycc_rt_bigint_release")
        .count();
    // D-182's two ingress retains for the `TupleLiteral`'s borrowed
    // `n` elements, plus the tuple bind's retain and the `return`'s,
    // against the `y` slot's own release-before-store.
    assert_eq!((retains, releases), (4, 1), "got {calls:?}");
}

/// Counts the refcount calls in a one-statement function whose only
/// body is `t = TupleLiteral(elements)`, with `params` in scope.
fn tuple_literal_refcount_counts(
    label: &str,
    params: Vec<(String, Ty)>,
    elements: Vec<MirExpr>,
) -> (usize, usize) {
    let mir = MirModule {
        items: vec![MirItem::Function {
            name: "build_tuple".to_string(),
            params,
            return_ty: Ty::None,
            body: vec![MirStmt::Assign {
                target: "t".to_string(),
                value: MirExpr::TupleLiteral(elements),
            }],
        }],
        class_defs: Vec::new(),
    };
    let calls = refcount_calls_in(label, &mir);
    let retains = calls
        .iter()
        .filter(|c| c.callee == "pycc_rt_bigint_retain")
        .count();
    let releases = calls
        .iter()
        .filter(|c| c.callee == "pycc_rt_bigint_release")
        .count();
    (retains, releases)
}

/// #633 direction B (D-182): `MirExpr::TupleLiteral` takes a reference
/// for the field it is about to store -- but only for a *borrowed*
/// element word.
///
/// This is the assertion that discriminates D-182's chosen shape from
/// the unconditional-retain shape it rejected. An owning element
/// already arrives holding the single reference the field will keep;
/// retaining it too would leave rc at 2 with one owner, which a future
/// `Ty::Tuple` slot-death release under D-124 could never balance.
///
/// Nothing releases a tuple field today, so every case here expects a
/// release count of zero -- D-182's accepted, deliberate imbalance.
#[test]
fn a_tuple_literal_retains_only_its_borrowed_int_elements() {
    // A borrowed `Name` element: exactly one ingress retain. The
    // in-range `IntLiteral` sibling is a *constant* tagged smallint,
    // so `emit_bigint_refcount_call` emits nothing at all for it.
    assert_eq!(
        tuple_literal_refcount_counts(
            "bigint_rc_tuple_ingress_borrowed",
            vec![("n".to_string(), Ty::Int)],
            vec![int_name("n"), MirExpr::IntLiteral(1)],
        ),
        (1, 0)
    );

    // An *owning* element: no ingress retain. The literal is `2^62`
    // rather than a small one on purpose -- only an out-of-range
    // literal is materialized by a runtime `pycc_rt_int_from_i64`
    // call, so only that shape produces a non-constant word that
    // `emit_bigint_refcount_call` would not have skipped anyway. An
    // in-range literal would prove nothing about the classification.
    assert_eq!(
        tuple_literal_refcount_counts(
            "bigint_rc_tuple_ingress_owning",
            Vec::new(),
            vec![
                MirExpr::IntLiteral(4_611_686_018_427_387_904),
                MirExpr::IntLiteral(1),
            ],
        ),
        (0, 0)
    );

    // `float`/`bool` elements are not `int` words at all: the
    // `Scalar::Int` test in `retain_if_int_duplicate` filters them out
    // before the source-expression classification is even consulted.
    assert_eq!(
        tuple_literal_refcount_counts(
            "bigint_rc_tuple_ingress_non_int",
            vec![("f".to_string(), Ty::Float), ("b".to_string(), Ty::Bool)],
            vec![
                MirExpr::Name {
                    name: "f".to_string(),
                    ty: Ty::Float,
                },
                MirExpr::Name {
                    name: "b".to_string(),
                    ty: Ty::Bool,
                },
            ],
        ),
        (0, 0)
    );
}
