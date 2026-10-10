//! List and set comprehensions over range, list, set and dict sources.

use super::*;

/// `MirStmt::ForSet { var: "v", var_ty: int, set: <set>, body: [print(v)] }` -- the
/// `SetCompAssign` test suite's own analog of `print_each_int` above,
/// for the identical reason (container `to_str`/`truthy` remain
/// unimplemented, D-107/D-124, so a produced `set[int]` cannot be
/// printed directly and must be walked element-by-element via
/// `MirStmt::ForSet` instead, whose own insertion-order iteration this
/// crate's pre-existing `a_return_inside_a_for_set_body_returns_
/// immediately_without_looping` test already pins).
fn print_each_int_from_set(set: &str) -> MirStmt {
    MirStmt::ForSet {
        var_ty: pycc_mir::Ty::Int,
        var: "v".to_string(),
        set: set.to_string(),
        body: vec![MirStmt::ExprStmt(MirExpr::Call {
            callee: "print".to_string(),
            args: vec![MirExpr::Name {
                name: "v".to_string(),
                ty: Ty::Int,
            }],
            ty: Ty::None,
        })],
    }
}

#[test]
fn a_range_sourced_list_comprehension_with_no_filter_computes_every_element() {
    // `xs = [i * 2 for i in range(5)]` (PR-12 Task 5a, D-117):
    // `CompSource::Range`, no `cond`. Exercises the `Range` branch of
    // `MirStmt::ListCompAssign`'s own internal `match source` (mirrors
    // `MirStmt::ForRange`'s own shape), the `cond: None` unconditional-
    // append path, and confirms `elt` (`i * 2`, not a bare `Name`) is
    // evaluated fresh every iteration rather than the loop's own raw
    // induction value being appended untransformed.
    let mir = MirModule {
        items: vec![
            MirItem::TopLevelStmt(MirStmt::ListCompAssign {
                target: "xs".to_string(),
                var: "i".to_string(),
                var_ty: Ty::Int,
                source: CompSource::Range {
                    start: MirExpr::IntLiteral(0),
                    stop: MirExpr::IntLiteral(5),
                    step: MirExpr::IntLiteral(1),
                },
                cond: None,
                elt: Box::new(MirExpr::BinOp {
                    op: BinOpKind::Mul,
                    left: Box::new(MirExpr::Name {
                        name: "i".to_string(),
                        ty: Ty::Int,
                    }),
                    right: Box::new(MirExpr::IntLiteral(2)),
                    ty: Ty::Int,
                }),
            }),
            MirItem::TopLevelStmt(print_each_int("xs")),
        ],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("listcomp_range_no_filter")
        .expect("failed to create scratch dir");
    let obj_path = dir.join("listcomp_range_no_filter.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
    let bin_path = dir.join("listcomp_range_no_filter");
    link_object_with_runtime(&obj_path, &bin_path);
    let output = Command::new(&bin_path).output().expect("binary should run");
    assert_eq!(output.stdout, b"0\n2\n4\n6\n8\n");
}

#[test]
fn a_list_sourced_list_comprehension_with_a_filter_only_keeps_matching_elements() {
    // `xs = [i for i in range(5)]` then `ys = [x for x in xs if x > 2]`
    // (PR-12 Task 5a, D-117): the second comprehension's own
    // `CompSource::List` is sourced from the *first* comprehension's own
    // produced list, not a literal -- exercising the `List` branch of
    // `MirStmt::ListCompAssign`'s own internal `match source` (mirrors
    // `MirStmt::ForList`'s own shape, via `emit_list_name_read`) and the
    // `cond: Some(..)` filtered-append path (the `listcomp_if_taken`/
    // `listcomp_if_skip` block pair): `0` and `1` and `2` must be
    // dropped, `3` and `4` kept.
    let mir = MirModule {
        items: vec![
            MirItem::TopLevelStmt(MirStmt::ListCompAssign {
                target: "xs".to_string(),
                var: "i".to_string(),
                var_ty: Ty::Int,
                source: CompSource::Range {
                    start: MirExpr::IntLiteral(0),
                    stop: MirExpr::IntLiteral(5),
                    step: MirExpr::IntLiteral(1),
                },
                cond: None,
                elt: Box::new(MirExpr::Name {
                    name: "i".to_string(),
                    ty: Ty::Int,
                }),
            }),
            MirItem::TopLevelStmt(MirStmt::ListCompAssign {
                target: "ys".to_string(),
                var: "x".to_string(),
                var_ty: Ty::Int,
                source: CompSource::List("xs".to_string()),
                cond: Some(Box::new(MirExpr::Compare {
                    op: CmpOpKind::Gt,
                    left: Box::new(MirExpr::Name {
                        name: "x".to_string(),
                        ty: Ty::Int,
                    }),
                    right: Box::new(MirExpr::IntLiteral(2)),
                    ty: Ty::Bool,
                })),
                elt: Box::new(MirExpr::Name {
                    name: "x".to_string(),
                    ty: Ty::Int,
                }),
            }),
            MirItem::TopLevelStmt(print_each_int("ys")),
        ],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("listcomp_list_with_filter")
        .expect("failed to create scratch dir");
    let obj_path = dir.join("listcomp_list_with_filter.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
    let bin_path = dir.join("listcomp_list_with_filter");
    link_object_with_runtime(&obj_path, &bin_path);
    let output = Command::new(&bin_path).output().expect("binary should run");
    assert_eq!(output.stdout, b"3\n4\n");
}

#[test]
fn a_list_sourced_list_comprehension_that_rebinds_its_own_source_name_reads_the_pre_existing_value()
{
    // Regression test (review round 1, post-Task-5a): `xs = [i for i in
    // range(5)]` then `xs = [x for x in xs if x > 2]`, reusing the same
    // name for both the source *and* the target of the second
    // comprehension. Real CPython evaluates the entire RHS -- the whole
    // loop over the pre-existing `xs` (`[0, 1, 2, 3, 4]`) -- before
    // rebinding `xs` to the result (`[3, 4]`), exactly like an ordinary
    // `xs = xs + [1]` never sees its own partially-updated target mid-
    // expression. An earlier version of this arm stored the target's
    // freshly allocated *empty* list into `xs`'s own slot before
    // evaluating `source`, so `emit_list_name_read(..., "xs")` read the
    // brand-new empty list instead of the original one, and this
    // produced `len(xs) == 0` instead of `2`. Fixed by deferring
    // `emit_assign(target, ..)` until after the loop (see this arm's own
    // "point 1"/"point 5" doc comments on `ListCompAssign` in `lib.rs`).
    let mir = MirModule {
        items: vec![
            MirItem::TopLevelStmt(MirStmt::ListCompAssign {
                target: "xs".to_string(),
                var: "i".to_string(),
                var_ty: Ty::Int,
                source: CompSource::Range {
                    start: MirExpr::IntLiteral(0),
                    stop: MirExpr::IntLiteral(5),
                    step: MirExpr::IntLiteral(1),
                },
                cond: None,
                elt: Box::new(MirExpr::Name {
                    name: "i".to_string(),
                    ty: Ty::Int,
                }),
            }),
            MirItem::TopLevelStmt(MirStmt::ListCompAssign {
                target: "xs".to_string(),
                var: "x".to_string(),
                var_ty: Ty::Int,
                source: CompSource::List("xs".to_string()),
                cond: Some(Box::new(MirExpr::Compare {
                    op: CmpOpKind::Gt,
                    left: Box::new(MirExpr::Name {
                        name: "x".to_string(),
                        ty: Ty::Int,
                    }),
                    right: Box::new(MirExpr::IntLiteral(2)),
                    ty: Ty::Bool,
                })),
                elt: Box::new(MirExpr::Name {
                    name: "x".to_string(),
                    ty: Ty::Int,
                }),
            }),
            MirItem::TopLevelStmt(print_each_int("xs")),
        ],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("listcomp_self_referential_source")
        .expect("failed to create scratch dir");
    let obj_path = dir.join("listcomp_self_referential_source.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
    let bin_path = dir.join("listcomp_self_referential_source");
    link_object_with_runtime(&obj_path, &bin_path);
    let output = Command::new(&bin_path).output().expect("binary should run");
    assert_eq!(output.stdout, b"3\n4\n");
}

#[test]
fn a_range_sourced_list_comprehension_whose_bound_reads_its_own_rebound_target_uses_the_pre_existing_length()
 {
    // Regression test (review round 1, post-Task-5a): `xs = [i for i in
    // range(5)]` then `xs = [i * 2 for i in range(len(xs))]`. `source`
    // is `CompSource::Range` here, not `CompSource::List` -- distinct
    // from the test directly above, since it exercises `source`'s own
    // `stop` expression (`len(xs)`) reading `target`'s pre-existing
    // value during the *preheader*, before `test_bb` even exists, not
    // `var`'s own per-iteration container read. Real CPython evaluates
    // `range(len(xs))` once, against the original 5-element `xs`,
    // before the comprehension loop runs at all, giving a 5-element
    // result (`[0, 2, 4, 6, 8]`); the same premature-rebind bug this
    // file's neighboring regression test documents would instead have
    // read the just-emptied `xs` here, giving `range(0)` and an empty
    // result.
    let mir = MirModule {
        items: vec![
            MirItem::TopLevelStmt(MirStmt::ListCompAssign {
                target: "xs".to_string(),
                var: "i".to_string(),
                var_ty: Ty::Int,
                source: CompSource::Range {
                    start: MirExpr::IntLiteral(0),
                    stop: MirExpr::IntLiteral(5),
                    step: MirExpr::IntLiteral(1),
                },
                cond: None,
                elt: Box::new(MirExpr::Name {
                    name: "i".to_string(),
                    ty: Ty::Int,
                }),
            }),
            MirItem::TopLevelStmt(MirStmt::ListCompAssign {
                target: "xs".to_string(),
                var: "i".to_string(),
                var_ty: Ty::Int,
                source: CompSource::Range {
                    start: MirExpr::IntLiteral(0),
                    stop: MirExpr::Call {
                        callee: "len".to_string(),
                        args: vec![MirExpr::Name {
                            name: "xs".to_string(),
                            ty: Ty::List(Box::new(Ty::Int)),
                        }],
                        ty: Ty::Int,
                    },
                    step: MirExpr::IntLiteral(1),
                },
                cond: None,
                elt: Box::new(MirExpr::BinOp {
                    op: BinOpKind::Mul,
                    left: Box::new(MirExpr::Name {
                        name: "i".to_string(),
                        ty: Ty::Int,
                    }),
                    right: Box::new(MirExpr::IntLiteral(2)),
                    ty: Ty::Int,
                }),
            }),
            MirItem::TopLevelStmt(print_each_int("xs")),
        ],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("listcomp_self_referential_range_bound")
        .expect("failed to create scratch dir");
    let obj_path = dir.join("listcomp_self_referential_range_bound.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
    let bin_path = dir.join("listcomp_self_referential_range_bound");
    link_object_with_runtime(&obj_path, &bin_path);
    let output = Command::new(&bin_path).output().expect("binary should run");
    assert_eq!(output.stdout, b"0\n2\n4\n6\n8\n");
}

#[test]
fn a_list_comprehensions_elt_reading_its_own_rebound_target_reads_the_pre_existing_value() {
    // Regression test (review round 1, post-Task-5a): `xs = [1, 2, 3]`
    // then `xs = [xs[0] for i in range(2)]`. Distinct from both tests
    // above: `elt` (not `source`) reads `target`'s own name here,
    // *inside* the loop body, once per iteration. Real CPython gives
    // `[1, 1]` (`xs[0]` is `1` throughout, since the original `xs` is
    // untouched until the whole comprehension finishes). The
    // premature-rebind bug this file's other two regression tests above
    // document made this specific shape crash outright, not merely
    // print the wrong value: `xs`'s slot held the freshly allocated
    // *empty* list for the entire loop, so the very first
    // `xs[0]` read inside `elt` panicked with `pycc_rt`'s own honest
    // "list index out of range" before any element was ever appended.
    let mir = MirModule {
        items: vec![
            MirItem::TopLevelStmt(MirStmt::Assign {
                target: "xs".to_string(),
                value: MirExpr::ListLiteral(vec![
                    MirExpr::IntLiteral(1),
                    MirExpr::IntLiteral(2),
                    MirExpr::IntLiteral(3),
                ]),
            }),
            MirItem::TopLevelStmt(MirStmt::ListCompAssign {
                target: "xs".to_string(),
                var: "i".to_string(),
                var_ty: Ty::Int,
                source: CompSource::Range {
                    start: MirExpr::IntLiteral(0),
                    stop: MirExpr::IntLiteral(2),
                    step: MirExpr::IntLiteral(1),
                },
                cond: None,
                elt: Box::new(MirExpr::Subscript {
                    base: Box::new(MirExpr::Name {
                        name: "xs".to_string(),
                        ty: Ty::List(Box::new(Ty::Int)),
                    }),
                    index: Box::new(MirExpr::IntLiteral(0)),
                }),
            }),
            MirItem::TopLevelStmt(print_each_int("xs")),
        ],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("listcomp_self_referential_elt")
        .expect("failed to create scratch dir");
    let obj_path = dir.join("listcomp_self_referential_elt.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
    let bin_path = dir.join("listcomp_self_referential_elt");
    link_object_with_runtime(&obj_path, &bin_path);
    let output = Command::new(&bin_path).output().expect("binary should run");
    assert_eq!(output.stdout, b"1\n1\n");
}

#[test]
fn a_set_sourced_list_comprehension_with_no_filter_visits_every_element() {
    // `zs = [x for x in some_set]` (PR-12 Task 5a, D-117): `set[int]`'s
    // own element type is always `Ty::Int` (T0038), satisfying
    // `list[int]`'s own T0034 gate trivially, so this source is
    // reachable and type-safe from real Python source. Exercises the
    // `Set` branch of `MirStmt::ListCompAssign`'s own internal `match
    // source` (mirrors `MirStmt::ForSet`'s own shape, via
    // `emit_set_name_read`/`build_int_set_get`/`build_int_set_len`).
    // `{1, 2, 3}` is read back in insertion order (pinned by this
    // crate's own `a_return_inside_a_for_set_body_returns_immediately_
    // without_looping` test, which reads the same literal's first
    // element as `1`), so the produced list's own order is
    // deterministic here.
    let mir = MirModule {
        items: vec![
            MirItem::TopLevelStmt(MirStmt::Assign {
                target: "some_set".to_string(),
                value: MirExpr::SetLiteral {
                    elements: vec![
                        MirExpr::IntLiteral(1),
                        MirExpr::IntLiteral(2),
                        MirExpr::IntLiteral(3),
                    ],
                    ops: None,
                },
            }),
            MirItem::TopLevelStmt(MirStmt::ListCompAssign {
                target: "zs".to_string(),
                var: "x".to_string(),
                var_ty: Ty::Int,
                source: CompSource::Set("some_set".to_string()),
                cond: None,
                elt: Box::new(MirExpr::Name {
                    name: "x".to_string(),
                    ty: Ty::Int,
                }),
            }),
            MirItem::TopLevelStmt(print_each_int("zs")),
        ],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("listcomp_set_no_filter")
        .expect("failed to create scratch dir");
    let obj_path = dir.join("listcomp_set_no_filter.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
    let bin_path = dir.join("listcomp_set_no_filter");
    link_object_with_runtime(&obj_path, &bin_path);
    let output = Command::new(&bin_path).output().expect("binary should run");
    assert_eq!(output.stdout, b"1\n2\n3\n");
}

#[test]
fn an_empty_range_sourced_list_comprehension_produces_a_genuinely_valid_empty_list() {
    // `zs = [i for i in range(0)]` (PR-12 Task 5a, D-117): the loop body
    // never executes, so `zs` must still be a real, non-null
    // `PyIntListObj` -- not a null/dangling pointer that merely happens
    // never to be read. `len(zs)` proves the pointer is valid enough for
    // `pycc_rt_int_list_len` to read (a null pointer would segfault, not
    // silently return `0`); appending to it afterward and reading the
    // appended element back out proves it is independently *writable*
    // too, not merely readable.
    let mir = MirModule {
        items: vec![
            MirItem::TopLevelStmt(MirStmt::ListCompAssign {
                target: "zs".to_string(),
                var: "i".to_string(),
                var_ty: Ty::Int,
                source: CompSource::Range {
                    start: MirExpr::IntLiteral(0),
                    stop: MirExpr::IntLiteral(0),
                    step: MirExpr::IntLiteral(1),
                },
                cond: None,
                elt: Box::new(MirExpr::Name {
                    name: "i".to_string(),
                    ty: Ty::Int,
                }),
            }),
            MirItem::TopLevelStmt(MirStmt::ExprStmt(MirExpr::Call {
                callee: "print".to_string(),
                args: vec![MirExpr::Call {
                    callee: "len".to_string(),
                    args: vec![MirExpr::Name {
                        name: "zs".to_string(),
                        ty: Ty::List(Box::new(Ty::Int)),
                    }],
                    ty: Ty::Int,
                }],
                ty: Ty::None,
            })),
            MirItem::TopLevelStmt(MirStmt::ExprStmt(MirExpr::ListAppend {
                list: pycc_mir::MirContainerReceiver::Name("zs".to_string()),
                value: Box::new(MirExpr::IntLiteral(9)),
            })),
            MirItem::TopLevelStmt(MirStmt::ExprStmt(MirExpr::Call {
                callee: "print".to_string(),
                args: vec![MirExpr::Subscript {
                    base: Box::new(MirExpr::Name {
                        name: "zs".to_string(),
                        ty: Ty::List(Box::new(Ty::Int)),
                    }),
                    index: Box::new(MirExpr::IntLiteral(0)),
                }],
                ty: Ty::None,
            })),
        ],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("listcomp_range_empty")
        .expect("failed to create scratch dir");
    let obj_path = dir.join("listcomp_range_empty.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
    let bin_path = dir.join("listcomp_range_empty");
    link_object_with_runtime(&obj_path, &bin_path);
    let output = Command::new(&bin_path).output().expect("binary should run");
    assert_eq!(output.stdout, b"0\n9\n");
}

#[test]
fn a_dict_sourced_list_comprehension_binds_its_key_without_crashing() {
    // `CompSource::Dict` reaching `MirStmt::ListCompAssign` is an
    // internal-error-panic path no type-checked program can ever
    // trigger from real source (PR-12 Task 5a's own brief, D-117): every
    // reachable `list[int]`-producing comprehension has `elt: Ty::Int`
    // (T0034), and this compiler has no `str`-to-`int` builtin of any
    // kind, so `pycc_types` can never route a dict-typed base into a
    // `list[int]` comprehension's own source. This test bypasses
    // `pycc_types` entirely (hand-built MIR, mirroring this crate's own
    // established convention for other unreachable-from-real-source
    // shapes) to confirm the dict-sourced per-iteration binding code
    // path -- the `pycc_rt_str_incref` call on the read key, exactly
    // like `MirStmt::ForDict`'s own per-iteration bind -- does not crash
    // even though nothing reachable exercises it today. `elt` is a
    // constant (`7`), not a read of the bound key, since `var`'s own
    // type here is `Ty::Str` and a `list[int]`'s own element type is
    // `Ty::Int` -- the binding itself is what this test is pinning, not
    // what `elt` does with it afterward (this crate has no `str`-typed
    // `elt` to give it that would still type as `Ty::Int`).
    let mir = MirModule {
        items: vec![
            MirItem::TopLevelStmt(MirStmt::Assign {
                target: "d".to_string(),
                value: MirExpr::DictLiteral(vec![
                    (
                        MirExpr::StringLiteral("a".to_string()),
                        MirExpr::IntLiteral(1),
                    ),
                    (
                        MirExpr::StringLiteral("b".to_string()),
                        MirExpr::IntLiteral(2),
                    ),
                ]),
            }),
            MirItem::TopLevelStmt(MirStmt::ListCompAssign {
                target: "zs".to_string(),
                var: "k".to_string(),
                var_ty: Ty::Str,
                source: CompSource::Dict("d".to_string()),
                cond: None,
                elt: Box::new(MirExpr::IntLiteral(7)),
            }),
            MirItem::TopLevelStmt(print_each_int("zs")),
        ],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("listcomp_dict_source_no_crash")
        .expect("failed to create scratch dir");
    let obj_path = dir.join("listcomp_dict_source_no_crash.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
    let bin_path = dir.join("listcomp_dict_source_no_crash");
    link_object_with_runtime(&obj_path, &bin_path);
    let output = Command::new(&bin_path).output().expect("binary should run");
    assert_eq!(output.stdout, b"7\n7\n");
}

// ---- PR-12 Task 5b: `SetCompAssign` codegen tests ----

#[test]
fn a_range_sourced_set_comprehension_with_a_filter_only_keeps_matching_elements() {
    // `evens = {x for x in range(10) if x % 2 == 0}` (PR-12 Task 5b,
    // D-117): exercises `MirStmt::SetCompAssign`'s own `Range` branch
    // (mirrors `ListCompAssign`'s own `Range` branch exactly) and the
    // `cond: Some(..)` filtered-insert path. Expected output verified
    // against `python3` on this exact source (`set[int]`'s own
    // insertion order is pinned by this crate's own
    // `a_return_inside_a_for_set_body_returns_immediately_without_
    // looping` test, so the filtered evens print back in ascending
    // order here).
    let mir = MirModule {
        items: vec![
            MirItem::TopLevelStmt(MirStmt::SetCompAssign {
                ops: None,
                target: "evens".to_string(),
                var: "x".to_string(),
                var_ty: Ty::Int,
                source: CompSource::Range {
                    start: MirExpr::IntLiteral(0),
                    stop: MirExpr::IntLiteral(10),
                    step: MirExpr::IntLiteral(1),
                },
                cond: Some(Box::new(MirExpr::Compare {
                    op: CmpOpKind::Eq,
                    left: Box::new(MirExpr::BinOp {
                        op: BinOpKind::Mod,
                        left: Box::new(MirExpr::Name {
                            name: "x".to_string(),
                            ty: Ty::Int,
                        }),
                        right: Box::new(MirExpr::IntLiteral(2)),
                        ty: Ty::Int,
                    }),
                    right: Box::new(MirExpr::IntLiteral(0)),
                    ty: Ty::Bool,
                })),
                elt: Box::new(MirExpr::Name {
                    name: "x".to_string(),
                    ty: Ty::Int,
                }),
            }),
            MirItem::TopLevelStmt(print_each_int_from_set("evens")),
        ],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("setcomp_range_with_filter")
        .expect("failed to create scratch dir");
    let obj_path = dir.join("setcomp_range_with_filter.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
    let bin_path = dir.join("setcomp_range_with_filter");
    link_object_with_runtime(&obj_path, &bin_path);
    let output = Command::new(&bin_path).output().expect("binary should run");
    assert_eq!(output.stdout, b"0\n2\n4\n6\n8\n");
}

#[test]
fn a_list_sourced_set_comprehension_with_no_filter_deduplicates_repeated_elements() {
    // `xs = [1, 2, 2, 3]` then `s = {x for x in xs}` (PR-12 Task 5b,
    // D-117): exercises `MirStmt::SetCompAssign`'s own `List` branch and
    // the `cond: None` unconditional-insert path, and confirms
    // `pycc_rt_int_set_add`'s own dedup check (D-121) collapses the
    // repeated `2` to a single entry -- `len(s) == 3`, not `4`.
    let mir = MirModule {
        items: vec![
            MirItem::TopLevelStmt(MirStmt::Assign {
                target: "xs".to_string(),
                value: MirExpr::ListLiteral(vec![
                    MirExpr::IntLiteral(1),
                    MirExpr::IntLiteral(2),
                    MirExpr::IntLiteral(2),
                    MirExpr::IntLiteral(3),
                ]),
            }),
            MirItem::TopLevelStmt(MirStmt::SetCompAssign {
                ops: None,
                target: "s".to_string(),
                var: "x".to_string(),
                var_ty: Ty::Int,
                source: CompSource::List("xs".to_string()),
                cond: None,
                elt: Box::new(MirExpr::Name {
                    name: "x".to_string(),
                    ty: Ty::Int,
                }),
            }),
            MirItem::TopLevelStmt(print_expr(MirExpr::Call {
                callee: "len".to_string(),
                args: vec![set_name("s")],
                ty: Ty::Int,
            })),
            MirItem::TopLevelStmt(print_each_int_from_set("s")),
        ],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("setcomp_list_no_filter_dedup")
        .expect("failed to create scratch dir");
    let obj_path = dir.join("setcomp_list_no_filter_dedup.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
    let bin_path = dir.join("setcomp_list_no_filter_dedup");
    link_object_with_runtime(&obj_path, &bin_path);
    let output = Command::new(&bin_path).output().expect("binary should run");
    assert_eq!(output.stdout, b"3\n1\n2\n3\n");
}

#[test]
fn a_set_sourced_set_comprehension_that_rebinds_its_own_source_name_reads_the_pre_existing_value() {
    // `s = {1, 2, 3}` then `s = {x for x in s if x > 1}`, reusing the
    // same name for both the source *and* the target (PR-12 Task 5b,
    // D-117): the direct `SetCompAssign` analog of `ListCompAssign`'s
    // own review-round-1 regression (see that arm's own "point 1"/
    // "point 5" comments, and this crate's `a_list_sourced_list_
    // comprehension_that_rebinds_its_own_source_name_reads_the_pre_
    // existing_value` test) -- a premature `emit_assign(target, ..)`
    // before the loop runs would make `emit_set_name_read(..., "s")`
    // read the freshly allocated, still-empty set instead of the
    // original `{1, 2, 3}`, producing an empty result instead of
    // `{2, 3}`. Also exercises `SetCompAssign`'s own `Set` branch and
    // the `cond: Some(..)` filtered path.
    let mir = MirModule {
        items: vec![
            MirItem::TopLevelStmt(MirStmt::Assign {
                target: "s".to_string(),
                value: MirExpr::SetLiteral {
                    elements: vec![
                        MirExpr::IntLiteral(1),
                        MirExpr::IntLiteral(2),
                        MirExpr::IntLiteral(3),
                    ],
                    ops: None,
                },
            }),
            MirItem::TopLevelStmt(MirStmt::SetCompAssign {
                ops: None,
                target: "s".to_string(),
                var: "x".to_string(),
                var_ty: Ty::Int,
                source: CompSource::Set("s".to_string()),
                cond: Some(Box::new(MirExpr::Compare {
                    op: CmpOpKind::Gt,
                    left: Box::new(MirExpr::Name {
                        name: "x".to_string(),
                        ty: Ty::Int,
                    }),
                    right: Box::new(MirExpr::IntLiteral(1)),
                    ty: Ty::Bool,
                })),
                elt: Box::new(MirExpr::Name {
                    name: "x".to_string(),
                    ty: Ty::Int,
                }),
            }),
            MirItem::TopLevelStmt(print_each_int_from_set("s")),
        ],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("setcomp_self_referential_rebind")
        .expect("failed to create scratch dir");
    let obj_path = dir.join("setcomp_self_referential_rebind.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
    let bin_path = dir.join("setcomp_self_referential_rebind");
    link_object_with_runtime(&obj_path, &bin_path);
    let output = Command::new(&bin_path).output().expect("binary should run");
    assert_eq!(output.stdout, b"2\n3\n");
}

#[test]
fn a_range_sourced_set_comprehension_whose_bound_reads_its_own_rebound_target_uses_the_pre_existing_length()
 {
    // `s = {1, 2, 3, 4, 5}` then `s = {i * 2 for i in range(len(s))}`
    // (PR-12 Task 5b, D-117): the `SetCompAssign` analog of
    // `ListCompAssign`'s own `a_range_sourced_list_comprehension_whose_
    // bound_reads_its_own_rebound_target_uses_the_pre_existing_length`
    // regression test -- distinct from the test directly above, since
    // it exercises `source`'s own `stop` expression (`len(s)`) reading
    // `target`'s pre-existing value during the *preheader*, before
    // `test_bb` even exists, not `var`'s own per-iteration container
    // read (`source` here is `CompSource::Range`, not `CompSource::
    // Set`). Real CPython evaluates `range(len(s))` once, against the
    // original 5-element `s`, before the comprehension loop runs at
    // all, giving a 5-element result (`{0, 2, 4, 6, 8}`); the premature-
    // rebind bug this file's `ListCompAssign` neighbor documents would
    // instead have read the just-emptied `s` here, giving `range(0)`
    // and an empty result.
    let mir = MirModule {
        items: vec![
            MirItem::TopLevelStmt(MirStmt::Assign {
                target: "s".to_string(),
                value: MirExpr::SetLiteral {
                    elements: vec![
                        MirExpr::IntLiteral(1),
                        MirExpr::IntLiteral(2),
                        MirExpr::IntLiteral(3),
                        MirExpr::IntLiteral(4),
                        MirExpr::IntLiteral(5),
                    ],
                    ops: None,
                },
            }),
            MirItem::TopLevelStmt(MirStmt::SetCompAssign {
                ops: None,
                target: "s".to_string(),
                var: "i".to_string(),
                var_ty: Ty::Int,
                source: CompSource::Range {
                    start: MirExpr::IntLiteral(0),
                    stop: MirExpr::Call {
                        callee: "len".to_string(),
                        args: vec![set_name("s")],
                        ty: Ty::Int,
                    },
                    step: MirExpr::IntLiteral(1),
                },
                cond: None,
                elt: Box::new(MirExpr::BinOp {
                    op: BinOpKind::Mul,
                    left: Box::new(MirExpr::Name {
                        name: "i".to_string(),
                        ty: Ty::Int,
                    }),
                    right: Box::new(MirExpr::IntLiteral(2)),
                    ty: Ty::Int,
                }),
            }),
            MirItem::TopLevelStmt(print_each_int_from_set("s")),
        ],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("setcomp_self_referential_range_bound")
        .expect("failed to create scratch dir");
    let obj_path = dir.join("setcomp_self_referential_range_bound.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
    let bin_path = dir.join("setcomp_self_referential_range_bound");
    link_object_with_runtime(&obj_path, &bin_path);
    let output = Command::new(&bin_path).output().expect("binary should run");
    assert_eq!(output.stdout, b"0\n2\n4\n6\n8\n");
}

#[test]
fn a_dict_sourced_set_comprehension_binds_its_key_without_crashing() {
    // `CompSource::Dict` reaching `MirStmt::SetCompAssign` is
    // unreachable from real, type-checked source for the identical
    // reason `ListCompAssign`'s own analogous test gives (`set[int]`'s
    // own element type is always `Ty::Int`, T0038, and this compiler has
    // no `str`-to-`int` builtin) -- hand-built MIR, mirroring this
    // crate's own `a_dict_sourced_list_comprehension_binds_its_key_
    // without_crashing` test exactly. `elt` is a constant (`7`), not a
    // read of the bound key; `pycc_rt_int_set_add`'s own dedup collapses
    // the two (identical) inserted `7`s to one entry, confirming the
    // dict-sourced per-iteration binding path -- the
    // `pycc_rt_str_incref` call on the read key -- does not crash.
    let mir = MirModule {
        items: vec![
            MirItem::TopLevelStmt(MirStmt::Assign {
                target: "d".to_string(),
                value: MirExpr::DictLiteral(vec![
                    (
                        MirExpr::StringLiteral("a".to_string()),
                        MirExpr::IntLiteral(1),
                    ),
                    (
                        MirExpr::StringLiteral("b".to_string()),
                        MirExpr::IntLiteral(2),
                    ),
                ]),
            }),
            MirItem::TopLevelStmt(MirStmt::SetCompAssign {
                ops: None,
                target: "zs".to_string(),
                var: "k".to_string(),
                var_ty: Ty::Str,
                source: CompSource::Dict("d".to_string()),
                cond: None,
                elt: Box::new(MirExpr::IntLiteral(7)),
            }),
            MirItem::TopLevelStmt(print_expr(MirExpr::Call {
                callee: "len".to_string(),
                args: vec![set_name("zs")],
                ty: Ty::Int,
            })),
            MirItem::TopLevelStmt(print_each_int_from_set("zs")),
        ],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("setcomp_dict_source_no_crash")
        .expect("failed to create scratch dir");
    let obj_path = dir.join("setcomp_dict_source_no_crash.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
    let bin_path = dir.join("setcomp_dict_source_no_crash");
    link_object_with_runtime(&obj_path, &bin_path);
    let output = Command::new(&bin_path).output().expect("binary should run");
    assert_eq!(output.stdout, b"1\n7\n");
}
