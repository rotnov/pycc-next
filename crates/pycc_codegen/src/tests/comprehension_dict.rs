//! Dict comprehensions over range, list, set and dict sources.

use super::*;

// ---- PR-12 Task 5b: `DictCompAssign` codegen tests ----

#[test]
fn a_range_sourced_dict_comprehension_with_an_fstring_key_computes_every_entry() {
    // `named = {f"n{i}": i for i in range(3)}` (PR-12 Task 5b, D-117):
    // the common, no-aliasing-hazard shape -- `key` is a fresh
    // `MirExpr::FString` (already owning exactly one reference from its
    // own construction), so this arm's own `incref_if_str_duplicate`
    // call on `key` is a no-op here (see that arm's own doc comment
    // above). Exercises `MirStmt::DictCompAssign`'s own `Range` branch
    // and the `cond: None` unconditional-insert path.
    let mir = MirModule {
        items: vec![
            MirItem::TopLevelStmt(MirStmt::DictCompAssign {
                target: "named".to_string(),
                var: "i".to_string(),
                var_ty: Ty::Int,
                source: CompSource::Range {
                    start: MirExpr::IntLiteral(0),
                    stop: MirExpr::IntLiteral(3),
                    step: MirExpr::IntLiteral(1),
                },
                cond: None,
                key: Box::new(MirExpr::FString(vec![
                    pycc_mir::MirFStringPart::Literal("n".to_string()),
                    pycc_mir::MirFStringPart::Interpolation(Box::new(MirExpr::Name {
                        name: "i".to_string(),
                        ty: Ty::Int,
                    })),
                ])),
                value: Box::new(MirExpr::Name {
                    name: "i".to_string(),
                    ty: Ty::Int,
                }),
            }),
            MirItem::TopLevelStmt(print_expr(MirExpr::Call {
                callee: "len".to_string(),
                args: vec![dict_name("named")],
                ty: Ty::Int,
            })),
            MirItem::TopLevelStmt(print_expr(MirExpr::DictGet {
                dict: Box::new(dict_name("named")),
                key: Box::new(MirExpr::StringLiteral("n0".to_string())),
            })),
            MirItem::TopLevelStmt(print_expr(MirExpr::DictGet {
                dict: Box::new(dict_name("named")),
                key: Box::new(MirExpr::StringLiteral("n1".to_string())),
            })),
            MirItem::TopLevelStmt(print_expr(MirExpr::DictGet {
                dict: Box::new(dict_name("named")),
                key: Box::new(MirExpr::StringLiteral("n2".to_string())),
            })),
        ],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("dictcomp_range_fstring_key")
        .expect("failed to create scratch dir");
    let obj_path = dir.join("dictcomp_range_fstring_key.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
    let bin_path = dir.join("dictcomp_range_fstring_key");
    link_object_with_runtime(&obj_path, &bin_path);
    let output = Command::new(&bin_path).output().expect("binary should run");
    assert_eq!(output.stdout, b"3\n0\n1\n2\n");
}

#[test]
fn a_dict_sourced_dict_comprehension_builds_an_independent_copy_and_leaves_the_source_intact() {
    // `d = {"a": 10, "b": 20}` then `d2 = {k: 1 for k in d}` (PR-12 Task
    // 5b, D-117's own dedicated safety test, brief item (d)): `d`, a
    // pre-existing `dict[str, int]`, is the one `Dict`-sourced
    // `DictCompAssign` shape reachable from real, type-checked source
    // (T0036: `var`/`k` is `Ty::Str`, satisfying `dict[str, int]`'s own
    // key-type gate directly -- see this arm's own doc comment on
    // `DictCompAssign` in `lib.rs` for the full argument). Confirms two things: `d2`'s own contents are
    // correct and independently readable, and building `d2` leaves `d`'s
    // own contents unaffected -- exactly what this arm's own
    // `incref_if_str_duplicate` call on `key` (mirroring `MirStmt::
    // DictSet`'s own identical call, D-124) exists to guarantee: `d2`'s
    // own stored key becomes a genuinely independent reference, not a
    // bare, uncounted alias of `d`'s own key pointer.
    //
    // This is a functional-correctness test, not a use-after-free
    // detector: this compiler's container model keeps `dict[K, V]`
    // leak-only (D-124) and has no key-removal operation of any kind
    // (`del`, `.pop()`, ...) yet, so nothing currently implemented can
    // ever bring a dict's own claim on one of its keys down to zero --
    // the missing incref this test guards against cannot be forced into
    // an observable crash via any currently-reachable pycc-language
    // operation (see this task's own report for the full analysis). The
    // fix is still required by D-124's ownership contract and is
    // unconditionally applied here, independent of what this one test
    // can currently observe.
    let mir = MirModule {
        items: vec![
            MirItem::TopLevelStmt(MirStmt::Assign {
                target: "d".to_string(),
                value: MirExpr::DictLiteral(vec![
                    (
                        MirExpr::StringLiteral("a".to_string()),
                        MirExpr::IntLiteral(10),
                    ),
                    (
                        MirExpr::StringLiteral("b".to_string()),
                        MirExpr::IntLiteral(20),
                    ),
                ]),
            }),
            MirItem::TopLevelStmt(MirStmt::DictCompAssign {
                target: "d2".to_string(),
                var: "k".to_string(),
                var_ty: Ty::Str,
                source: CompSource::Dict("d".to_string()),
                cond: None,
                key: Box::new(MirExpr::Name {
                    name: "k".to_string(),
                    ty: Ty::Str,
                }),
                value: Box::new(MirExpr::IntLiteral(1)),
            }),
            // `d`'s own contents, read back after `d2` was built.
            MirItem::TopLevelStmt(print_expr(MirExpr::DictGet {
                dict: Box::new(dict_name("d")),
                key: Box::new(MirExpr::StringLiteral("a".to_string())),
            })),
            MirItem::TopLevelStmt(print_expr(MirExpr::DictGet {
                dict: Box::new(dict_name("d")),
                key: Box::new(MirExpr::StringLiteral("b".to_string())),
            })),
            MirItem::TopLevelStmt(print_expr(MirExpr::Call {
                callee: "len".to_string(),
                args: vec![dict_name("d")],
                ty: Ty::Int,
            })),
            // `d2`'s own contents, independently.
            MirItem::TopLevelStmt(print_expr(MirExpr::DictGet {
                dict: Box::new(dict_name("d2")),
                key: Box::new(MirExpr::StringLiteral("a".to_string())),
            })),
            MirItem::TopLevelStmt(print_expr(MirExpr::DictGet {
                dict: Box::new(dict_name("d2")),
                key: Box::new(MirExpr::StringLiteral("b".to_string())),
            })),
            MirItem::TopLevelStmt(print_expr(MirExpr::Call {
                callee: "len".to_string(),
                args: vec![dict_name("d2")],
                ty: Ty::Int,
            })),
        ],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("dictcomp_dict_source_independent_copy")
        .expect("failed to create scratch dir");
    let obj_path = dir.join("dictcomp_dict_source_independent_copy.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
    let bin_path = dir.join("dictcomp_dict_source_independent_copy");
    link_object_with_runtime(&obj_path, &bin_path);
    let output = Command::new(&bin_path).output().expect("binary should run");
    assert_eq!(output.stdout, b"10\n20\n2\n1\n1\n2\n");
}

#[test]
fn a_list_sourced_dict_comprehension_with_a_filter_only_keeps_matching_entries() {
    // `xs = [10, 20, 30]` then `named2 = {f"v{x}": x for x in xs if x >
    // 15}` (PR-12 Task 5b, D-117): exercises `MirStmt::DictCompAssign`'s
    // own `List` branch (reachable from real, type-checked source:
    // `list[int]`'s element type, T0034, satisfies `var`'s own
    // `Ty::Int`, and an f-string key is always `Ty::Str`, satisfying
    // T0036) and the `cond: Some(..)` filtered-insert path -- `10` must
    // be dropped, `20`/`30` kept.
    let mir = MirModule {
        items: vec![
            MirItem::TopLevelStmt(MirStmt::Assign {
                target: "xs".to_string(),
                value: MirExpr::ListLiteral(vec![
                    MirExpr::IntLiteral(10),
                    MirExpr::IntLiteral(20),
                    MirExpr::IntLiteral(30),
                ]),
            }),
            MirItem::TopLevelStmt(MirStmt::DictCompAssign {
                target: "named2".to_string(),
                var: "x".to_string(),
                var_ty: Ty::Int,
                source: CompSource::List("xs".to_string()),
                cond: Some(Box::new(MirExpr::Compare {
                    op: CmpOpKind::Gt,
                    left: Box::new(MirExpr::Name {
                        name: "x".to_string(),
                        ty: Ty::Int,
                    }),
                    right: Box::new(MirExpr::IntLiteral(15)),
                    ty: Ty::Bool,
                })),
                key: Box::new(MirExpr::FString(vec![
                    pycc_mir::MirFStringPart::Literal("v".to_string()),
                    pycc_mir::MirFStringPart::Interpolation(Box::new(MirExpr::Name {
                        name: "x".to_string(),
                        ty: Ty::Int,
                    })),
                ])),
                value: Box::new(MirExpr::Name {
                    name: "x".to_string(),
                    ty: Ty::Int,
                }),
            }),
            MirItem::TopLevelStmt(print_expr(MirExpr::Call {
                callee: "len".to_string(),
                args: vec![dict_name("named2")],
                ty: Ty::Int,
            })),
            MirItem::TopLevelStmt(print_expr(MirExpr::DictGet {
                dict: Box::new(dict_name("named2")),
                key: Box::new(MirExpr::StringLiteral("v20".to_string())),
            })),
            MirItem::TopLevelStmt(print_expr(MirExpr::DictGet {
                dict: Box::new(dict_name("named2")),
                key: Box::new(MirExpr::StringLiteral("v30".to_string())),
            })),
        ],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("dictcomp_list_with_filter")
        .expect("failed to create scratch dir");
    let obj_path = dir.join("dictcomp_list_with_filter.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
    let bin_path = dir.join("dictcomp_list_with_filter");
    link_object_with_runtime(&obj_path, &bin_path);
    let output = Command::new(&bin_path).output().expect("binary should run");
    assert_eq!(output.stdout, b"2\n20\n30\n");
}

#[test]
fn a_set_sourced_dict_comprehension_with_a_filter_only_keeps_matching_entries() {
    // `ys = {5, 6, 7, 10}` then `named3 = {f"s{y}": y for y in ys if y <
    // 10}` (PR-12 Task 5b, D-117): exercises `MirStmt::DictCompAssign`'s
    // own `Set` branch (reachable from real, type-checked source:
    // `set[int]`'s element type, T0038, satisfies `var`'s own `Ty::Int`)
    // and the `cond: Some(..)` filtered-insert path -- `10` must be
    // dropped, `5`/`6`/`7` kept.
    let mir = MirModule {
        items: vec![
            MirItem::TopLevelStmt(MirStmt::Assign {
                target: "ys".to_string(),
                value: MirExpr::SetLiteral {
                    elements: vec![
                        MirExpr::IntLiteral(5),
                        MirExpr::IntLiteral(6),
                        MirExpr::IntLiteral(7),
                        MirExpr::IntLiteral(10),
                    ],
                    ops: None,
                },
            }),
            MirItem::TopLevelStmt(MirStmt::DictCompAssign {
                target: "named3".to_string(),
                var: "y".to_string(),
                var_ty: Ty::Int,
                source: CompSource::Set("ys".to_string()),
                cond: Some(Box::new(MirExpr::Compare {
                    op: CmpOpKind::Lt,
                    left: Box::new(MirExpr::Name {
                        name: "y".to_string(),
                        ty: Ty::Int,
                    }),
                    right: Box::new(MirExpr::IntLiteral(10)),
                    ty: Ty::Bool,
                })),
                key: Box::new(MirExpr::FString(vec![
                    pycc_mir::MirFStringPart::Literal("s".to_string()),
                    pycc_mir::MirFStringPart::Interpolation(Box::new(MirExpr::Name {
                        name: "y".to_string(),
                        ty: Ty::Int,
                    })),
                ])),
                value: Box::new(MirExpr::Name {
                    name: "y".to_string(),
                    ty: Ty::Int,
                }),
            }),
            MirItem::TopLevelStmt(print_expr(MirExpr::Call {
                callee: "len".to_string(),
                args: vec![dict_name("named3")],
                ty: Ty::Int,
            })),
            MirItem::TopLevelStmt(print_expr(MirExpr::DictGet {
                dict: Box::new(dict_name("named3")),
                key: Box::new(MirExpr::StringLiteral("s5".to_string())),
            })),
            MirItem::TopLevelStmt(print_expr(MirExpr::DictGet {
                dict: Box::new(dict_name("named3")),
                key: Box::new(MirExpr::StringLiteral("s6".to_string())),
            })),
            MirItem::TopLevelStmt(print_expr(MirExpr::DictGet {
                dict: Box::new(dict_name("named3")),
                key: Box::new(MirExpr::StringLiteral("s7".to_string())),
            })),
        ],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("dictcomp_set_with_filter")
        .expect("failed to create scratch dir");
    let obj_path = dir.join("dictcomp_set_with_filter.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
    let bin_path = dir.join("dictcomp_set_with_filter");
    link_object_with_runtime(&obj_path, &bin_path);
    let output = Command::new(&bin_path).output().expect("binary should run");
    assert_eq!(output.stdout, b"3\n5\n6\n7\n");
}

#[test]
fn a_dict_sourced_dict_comprehension_that_rebinds_its_own_source_name_reads_the_pre_existing_value()
{
    // `d = {"a": 5, "b": 6}` then `d = {k: 9 for k in d}`, reusing the
    // same name for both the source *and* the target (PR-12 Task 5b,
    // D-117): the direct `DictCompAssign` analog of `ListCompAssign`'s
    // own review-round-1 regression (see that arm's own "point 1"/
    // "point 5" comments and this crate's `a_list_sourced_list_
    // comprehension_that_rebinds_its_own_source_name_reads_the_pre_
    // existing_value` test) -- a premature `emit_assign(target, ..)`
    // before the loop runs would make `emit_dict_name_read(..., "d")`
    // read the freshly allocated, still-empty dict instead of the
    // original `{"a": 5, "b": 6}`, producing an empty result instead of
    // two entries. Distinct from `a_dict_sourced_dict_comprehension_
    // builds_an_independent_copy_and_leaves_the_source_intact` above,
    // which uses distinct names (`d`/`d2`) and therefore does not
    // exercise this ordering fix at all -- this test is what actually
    // guards against reintroducing Task 5a's own review-round-1 defect
    // in this arm.
    let mir = MirModule {
        items: vec![
            MirItem::TopLevelStmt(MirStmt::Assign {
                target: "d".to_string(),
                value: MirExpr::DictLiteral(vec![
                    (
                        MirExpr::StringLiteral("a".to_string()),
                        MirExpr::IntLiteral(5),
                    ),
                    (
                        MirExpr::StringLiteral("b".to_string()),
                        MirExpr::IntLiteral(6),
                    ),
                ]),
            }),
            MirItem::TopLevelStmt(MirStmt::DictCompAssign {
                target: "d".to_string(),
                var: "k".to_string(),
                var_ty: Ty::Str,
                source: CompSource::Dict("d".to_string()),
                cond: None,
                key: Box::new(MirExpr::Name {
                    name: "k".to_string(),
                    ty: Ty::Str,
                }),
                value: Box::new(MirExpr::IntLiteral(9)),
            }),
            MirItem::TopLevelStmt(print_expr(MirExpr::Call {
                callee: "len".to_string(),
                args: vec![dict_name("d")],
                ty: Ty::Int,
            })),
            MirItem::TopLevelStmt(print_expr(MirExpr::DictGet {
                dict: Box::new(dict_name("d")),
                key: Box::new(MirExpr::StringLiteral("a".to_string())),
            })),
            MirItem::TopLevelStmt(print_expr(MirExpr::DictGet {
                dict: Box::new(dict_name("d")),
                key: Box::new(MirExpr::StringLiteral("b".to_string())),
            })),
        ],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("dictcomp_self_referential_rebind")
        .expect("failed to create scratch dir");
    let obj_path = dir.join("dictcomp_self_referential_rebind.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
    let bin_path = dir.join("dictcomp_self_referential_rebind");
    link_object_with_runtime(&obj_path, &bin_path);
    let output = Command::new(&bin_path).output().expect("binary should run");
    assert_eq!(output.stdout, b"2\n9\n9\n");
}

#[test]
fn a_range_sourced_dict_comprehension_whose_bound_reads_its_own_rebound_target_uses_the_pre_existing_length()
 {
    // `d = {"a": 1, "b": 2, "c": 3}` then `d = {f"n{i}": i for i in
    // range(len(d))}` (PR-12 Task 5b, D-117): the `DictCompAssign`
    // analog of `ListCompAssign`'s own `a_range_sourced_list_
    // comprehension_whose_bound_reads_its_own_rebound_target_uses_the_
    // pre_existing_length` regression test -- distinct from
    // `a_dict_sourced_dict_comprehension_that_rebinds_its_own_source_
    // name_reads_the_pre_existing_value` above, since it exercises
    // `source`'s own `stop` expression (`len(d)`) reading `target`'s
    // pre-existing value during the *preheader*, before `test_bb` even
    // exists, not `var`'s own per-iteration container read (`source`
    // here is `CompSource::Range`, not `CompSource::Dict`). Real
    // CPython evaluates `range(len(d))` once, against the original
    // 3-entry `d`, before the comprehension loop runs at all, giving a
    // 3-entry result (`{"n0": 0, "n1": 1, "n2": 2}`); the premature-
    // rebind bug this file's `ListCompAssign` neighbor documents would
    // instead have read the just-emptied `d` here, giving `range(0)`
    // and an empty result.
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
                    (
                        MirExpr::StringLiteral("c".to_string()),
                        MirExpr::IntLiteral(3),
                    ),
                ]),
            }),
            MirItem::TopLevelStmt(MirStmt::DictCompAssign {
                target: "d".to_string(),
                var: "i".to_string(),
                var_ty: Ty::Int,
                source: CompSource::Range {
                    start: MirExpr::IntLiteral(0),
                    stop: MirExpr::Call {
                        callee: "len".to_string(),
                        args: vec![dict_name("d")],
                        ty: Ty::Int,
                    },
                    step: MirExpr::IntLiteral(1),
                },
                cond: None,
                key: Box::new(MirExpr::FString(vec![
                    pycc_mir::MirFStringPart::Literal("n".to_string()),
                    pycc_mir::MirFStringPart::Interpolation(Box::new(MirExpr::Name {
                        name: "i".to_string(),
                        ty: Ty::Int,
                    })),
                ])),
                value: Box::new(MirExpr::Name {
                    name: "i".to_string(),
                    ty: Ty::Int,
                }),
            }),
            MirItem::TopLevelStmt(print_expr(MirExpr::Call {
                callee: "len".to_string(),
                args: vec![dict_name("d")],
                ty: Ty::Int,
            })),
            MirItem::TopLevelStmt(print_expr(MirExpr::DictGet {
                dict: Box::new(dict_name("d")),
                key: Box::new(MirExpr::StringLiteral("n0".to_string())),
            })),
            MirItem::TopLevelStmt(print_expr(MirExpr::DictGet {
                dict: Box::new(dict_name("d")),
                key: Box::new(MirExpr::StringLiteral("n1".to_string())),
            })),
            MirItem::TopLevelStmt(print_expr(MirExpr::DictGet {
                dict: Box::new(dict_name("d")),
                key: Box::new(MirExpr::StringLiteral("n2".to_string())),
            })),
        ],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("dictcomp_self_referential_range_bound")
        .expect("failed to create scratch dir");
    let obj_path = dir.join("dictcomp_self_referential_range_bound.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
    let bin_path = dir.join("dictcomp_self_referential_range_bound");
    link_object_with_runtime(&obj_path, &bin_path);
    let output = Command::new(&bin_path).output().expect("binary should run");
    assert_eq!(output.stdout, b"3\n0\n1\n2\n");
}
