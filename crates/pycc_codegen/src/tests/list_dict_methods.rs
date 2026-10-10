//! Slices, `list.pop`, `dict.get`, `set.add` and function redefinition.

use super::*;

// -- PR-12 Task 9 (D-118): `MirExpr::Slice` codegen -----------------

/// `xs = [<values>]` as a `MirStmt`, this section's own analog of
/// `assign_list_literal` above for an arbitrary element list rather
/// than the fixed `[1, 2, 3]` that helper hardcodes.
fn assign_list_literal_values(target: &str, values: &[i64]) -> MirStmt {
    MirStmt::Assign {
        target: target.to_string(),
        value: MirExpr::ListLiteral(values.iter().map(|v| MirExpr::IntLiteral(*v)).collect()),
    }
}

fn slice_list_name(name: &str) -> MirExpr {
    MirExpr::Name {
        name: name.to_string(),
        ty: Ty::List(Box::new(Ty::Int)),
    }
}

#[test]
fn a_slice_with_all_three_bounds_present_returns_the_expected_sub_range() {
    // D-118's ordinary path, at the codegen layer: `xs[1:4:1]` on
    // `[10, 20, 30, 40, 50]` is `[20, 30, 40]`. Exercises every `Some`
    // arm of the new `MirExpr::Slice` match (`start`/`stop`/`step` all
    // present), matching `tests/slice1_codegen_depth.rs`'s own identical
    // real-Python-source case (`a_basic_slice_with_explicit_bounds_
    // returns_the_expected_sub_range`) -- this hand-built-MIR version is
    // what actually counts toward `cargo llvm-cov -p pycc_codegen`,
    // since a workspace-root `tests/*.rs` integration binary is
    // attributed to the `pycc` package, not this crate.
    let mir = MirModule {
        items: vec![
            MirItem::TopLevelStmt(assign_list_literal_values("xs", &[10, 20, 30, 40, 50])),
            MirItem::TopLevelStmt(MirStmt::Assign {
                target: "ys".to_string(),
                value: MirExpr::Slice {
                    base: Box::new(slice_list_name("xs")),
                    start: Some(Box::new(MirExpr::IntLiteral(1))),
                    stop: Some(Box::new(MirExpr::IntLiteral(4))),
                    step: Some(Box::new(MirExpr::IntLiteral(1))),
                },
            }),
            MirItem::TopLevelStmt(print_each_int("ys")),
        ],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("slice_all_bounds_present")
        .expect("failed to create scratch dir");
    let obj_path = dir.join("slice_all_bounds_present.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
    let bin_path = dir.join("slice_all_bounds_present");
    link_object_with_runtime(&obj_path, &bin_path);
    let output = Command::new(&bin_path).output().expect("binary should run");
    assert_eq!(output.stdout, b"20\n30\n40\n");
}

#[test]
fn a_slice_with_every_bound_omitted_defaults_to_the_whole_list_stepped_by_one() {
    // D-118's own defaulting rule (`start`/`stop`/`step` default to
    // `0`/`len(list)`/`1`): `xs[:]` returns every element unchanged.
    // Exercises every `None` arm of the new match, including the
    // deferred `build_int_list_len` call this arm's own doc comment
    // describes -- the one line no other test in this group reaches,
    // since every other test here supplies an explicit `stop`.
    let mir = MirModule {
        items: vec![
            MirItem::TopLevelStmt(assign_list_literal_values("xs", &[10, 20, 30, 40, 50])),
            MirItem::TopLevelStmt(MirStmt::Assign {
                target: "ys".to_string(),
                value: MirExpr::Slice {
                    base: Box::new(slice_list_name("xs")),
                    start: None,
                    stop: None,
                    step: None,
                },
            }),
            MirItem::TopLevelStmt(print_each_int("ys")),
        ],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("slice_all_bounds_omitted")
        .expect("failed to create scratch dir");
    let obj_path = dir.join("slice_all_bounds_omitted.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
    let bin_path = dir.join("slice_all_bounds_omitted");
    link_object_with_runtime(&obj_path, &bin_path);
    let output = Command::new(&bin_path).output().expect("binary should run");
    assert_eq!(output.stdout, b"10\n20\n30\n40\n50\n");
}

#[test]
fn a_slice_with_only_a_step_present_skips_every_other_element() {
    // `xs[::2]` on `[0, 1, 2, 3, 4, 5]` is `[0, 2, 4]` -- `start`/`stop`
    // stay omitted (re-exercising the `None` arms the test above
    // already covers) while `step`'s own `Some` arm carries a value
    // other than `1`, pinning D-118's Step 5(c) requirement
    // specifically (a step greater than one, not just present at all).
    let mir = MirModule {
        items: vec![
            MirItem::TopLevelStmt(assign_list_literal_values("xs", &[0, 1, 2, 3, 4, 5])),
            MirItem::TopLevelStmt(MirStmt::Assign {
                target: "ys".to_string(),
                value: MirExpr::Slice {
                    base: Box::new(slice_list_name("xs")),
                    start: None,
                    stop: None,
                    step: Some(Box::new(MirExpr::IntLiteral(2))),
                },
            }),
            MirItem::TopLevelStmt(print_each_int("ys")),
        ],
        class_defs: Vec::new(),
    };
    let dir =
        pycc_scratch::ScratchDir::new("slice_step_only").expect("failed to create scratch dir");
    let obj_path = dir.join("slice_step_only.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
    let bin_path = dir.join("slice_step_only");
    link_object_with_runtime(&obj_path, &bin_path);
    let output = Command::new(&bin_path).output().expect("binary should run");
    assert_eq!(output.stdout, b"0\n2\n4\n");
}

#[test]
fn a_sliced_list_is_a_genuinely_independent_allocation_from_its_base() {
    // D-107's leak-only policy still requires the slice result to be a
    // *new* allocation, not an alias of `xs`'s own backing storage
    // (`pycc_rt_int_list_slice` itself is unit-tested for this in
    // `pycc_rt`; this pins the same guarantee through the real codegen
    // call site). Appending to `xs` after slicing must not retroactively
    // change `ys`, and vice versa.
    let mir = MirModule {
        items: vec![
            MirItem::TopLevelStmt(assign_list_literal_values("xs", &[1, 2, 3])),
            MirItem::TopLevelStmt(MirStmt::Assign {
                target: "ys".to_string(),
                value: MirExpr::Slice {
                    base: Box::new(slice_list_name("xs")),
                    start: Some(Box::new(MirExpr::IntLiteral(0))),
                    stop: Some(Box::new(MirExpr::IntLiteral(3))),
                    step: Some(Box::new(MirExpr::IntLiteral(1))),
                },
            }),
            MirItem::TopLevelStmt(MirStmt::ExprStmt(MirExpr::ListAppend {
                list: pycc_mir::MirContainerReceiver::Name("xs".to_string()),
                value: Box::new(MirExpr::IntLiteral(99)),
            })),
            MirItem::TopLevelStmt(MirStmt::ExprStmt(MirExpr::ListAppend {
                list: pycc_mir::MirContainerReceiver::Name("ys".to_string()),
                value: Box::new(MirExpr::IntLiteral(77)),
            })),
            MirItem::TopLevelStmt(print_each_int("xs")),
            MirItem::TopLevelStmt(print_each_int("ys")),
        ],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("slice_result_is_independent")
        .expect("failed to create scratch dir");
    let obj_path = dir.join("slice_result_is_independent.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
    let bin_path = dir.join("slice_result_is_independent");
    link_object_with_runtime(&obj_path, &bin_path);
    let output = Command::new(&bin_path).output().expect("binary should run");
    assert_eq!(output.stdout, b"1\n2\n3\n99\n1\n2\n3\n77\n");
}

#[test]
fn a_list_sourced_slice_that_rebinds_its_own_source_name_reads_the_pre_existing_value() {
    // Regression test in this arm's own established style (see
    // `a_list_sourced_list_comprehension_that_rebinds_its_own_source_
    // name_reads_the_pre_existing_value` above, Task 5a's confirmed
    // review-round regression): `xs = [1,2,3,4,5]` then `xs =
    // xs[1:3]`, reusing `xs` as both the slice's own `base` and the
    // assignment's own `target`. Real CPython fully evaluates the RHS
    // (reading the original 5-element `xs`, producing `[2, 3]`) before
    // rebinding `xs` to that result -- an implementation that stored
    // the slice's target pointer into `xs`'s slot before `base` had
    // been evaluated and the slice call completed would corrupt this.
    // `MirExpr::Slice`'s own arm never writes to `locals` at all (see
    // its doc comment) -- `MirStmt::Assign` is the only thing that
    // ever does, and only after this whole expression already produced
    // a pointer to a brand new, independent result object -- so there
    // is no premature-rebind window here to begin with, unlike
    // `ListCompAssign`'s own multi-block loop construction, which
    // needed a deliberate fix to get this right.
    let mir = MirModule {
        items: vec![
            MirItem::TopLevelStmt(assign_list_literal_values("xs", &[1, 2, 3, 4, 5])),
            MirItem::TopLevelStmt(MirStmt::Assign {
                target: "xs".to_string(),
                value: MirExpr::Slice {
                    base: Box::new(slice_list_name("xs")),
                    start: Some(Box::new(MirExpr::IntLiteral(1))),
                    stop: Some(Box::new(MirExpr::IntLiteral(3))),
                    step: None,
                },
            }),
            MirItem::TopLevelStmt(print_each_int("xs")),
        ],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("slice_self_referential_rebind")
        .expect("failed to create scratch dir");
    let obj_path = dir.join("slice_self_referential_rebind.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
    let bin_path = dir.join("slice_self_referential_rebind");
    link_object_with_runtime(&obj_path, &bin_path);
    let output = Command::new(&bin_path).output().expect("binary should run");
    assert_eq!(output.stdout, b"2\n3\n");
}

// -- PR-12 Task 11 (D-119): `list.pop()`/`dict.get(key, default)`/
// `set.add(value)` codegen -----------------------------------------

#[test]
fn list_pop_removes_and_returns_the_last_element_and_shrinks_len() {
    // `xs = [1,2,3]\ny = xs.pop()\nprint(y)\nprint(len(xs))\n` end to
    // end -- real `MirExpr::ListPop` codegen. Expected output verified
    // against `python3` on this exact source: `3` then `2`.
    let mir = MirModule {
        items: vec![
            MirItem::TopLevelStmt(assign_list_literal_values("xs", &[1, 2, 3])),
            MirItem::TopLevelStmt(MirStmt::Assign {
                target: "y".to_string(),
                value: MirExpr::ListPop {
                    index: None,
                    list: pycc_mir::MirContainerReceiver::Name("xs".to_string()),
                    ty: Ty::Int,
                },
            }),
            MirItem::TopLevelStmt(print_expr(MirExpr::Name {
                name: "y".to_string(),
                ty: Ty::Int,
            })),
            MirItem::TopLevelStmt(print_expr(MirExpr::Call {
                callee: "len".to_string(),
                args: vec![slice_list_name("xs")],
                ty: Ty::Int,
            })),
        ],
        class_defs: Vec::new(),
    };
    let dir =
        pycc_scratch::ScratchDir::new("list_pop_basic").expect("failed to create scratch dir");
    let obj_path = dir.join("list_pop_basic.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
    let bin_path = dir.join("list_pop_basic");
    link_object_with_runtime(&obj_path, &bin_path);
    let output = Command::new(&bin_path).output().expect("binary should run");
    assert_eq!(output.stdout, b"3\n2\n");
}

#[test]
fn pop_twice_on_the_same_list_in_one_statement_removes_in_order() {
    // `xs = [1,2,3,4,5]\nys = [xs.pop(), xs.pop()]\n` -- this task's own
    // brief flags exactly this shape as a place a Task-5a-class
    // evaluation-order bug could hide (binding a target's storage slot
    // before fully evaluating a self-referential right-hand side).
    // Verified empirically here, not just by the `MirExpr::ListPop`
    // arm's own doc comment reasoning: real CPython evaluates
    // `[xs.pop(), xs.pop()]`'s two elements strictly left to right,
    // each `.pop()` observing the previous one's mutation, so the
    // first `.pop()` removes `5` (leaving `[1,2,3,4]`) and the second
    // removes the new last element `4` (leaving `[1,2,3]`) --
    // `ys == [5, 4]`, `xs == [1,2,3]`. A codegen bug that read `xs`'s
    // pointer once and cached it, or that evaluated both `.pop()`
    // calls against a stale snapshot, would produce a different
    // result here (e.g. both calls popping the same element, or
    // popping in the wrong order) -- this test would catch either.
    let mir = MirModule {
        items: vec![
            MirItem::TopLevelStmt(assign_list_literal_values("xs", &[1, 2, 3, 4, 5])),
            MirItem::TopLevelStmt(MirStmt::Assign {
                target: "ys".to_string(),
                value: MirExpr::ListLiteral(vec![
                    MirExpr::ListPop {
                        index: None,
                        list: pycc_mir::MirContainerReceiver::Name("xs".to_string()),
                        ty: Ty::Int,
                    },
                    MirExpr::ListPop {
                        index: None,
                        list: pycc_mir::MirContainerReceiver::Name("xs".to_string()),
                        ty: Ty::Int,
                    },
                ]),
            }),
            MirItem::TopLevelStmt(print_each_int("ys")),
            MirItem::TopLevelStmt(print_each_int("xs")),
        ],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("list_pop_twice_same_statement")
        .expect("failed to create scratch dir");
    let obj_path = dir.join("list_pop_twice_same_statement.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
    let bin_path = dir.join("list_pop_twice_same_statement");
    link_object_with_runtime(&obj_path, &bin_path);
    let output = Command::new(&bin_path).output().expect("binary should run");
    assert_eq!(output.stdout, b"5\n4\n1\n2\n3\n");
}

#[test]
fn dict_get_or_default_on_a_present_key_returns_the_stored_value_codegens_and_runs() {
    // `d = {"a": 1}\nprint(d.get("a", -1))\n` end to end -- real
    // `MirExpr::DictGetOrDefault` codegen, found-key path. Expected
    // output verified against `python3` on this exact source: `1`.
    let mir = MirModule {
        items: vec![
            MirItem::TopLevelStmt(MirStmt::Assign {
                target: "d".to_string(),
                value: MirExpr::DictLiteral(vec![(
                    MirExpr::StringLiteral("a".to_string()),
                    MirExpr::IntLiteral(1),
                )]),
            }),
            MirItem::TopLevelStmt(print_expr(MirExpr::DictGetOrDefault {
                dict: pycc_mir::MirContainerReceiver::Name("d".to_string()),
                key: Box::new(MirExpr::StringLiteral("a".to_string())),
                default: Box::new(MirExpr::IntLiteral(-1)),
                ty: Ty::Int,
            })),
        ],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("dict_get_or_default_present")
        .expect("failed to create scratch dir");
    let obj_path = dir.join("dict_get_or_default_present.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
    let bin_path = dir.join("dict_get_or_default_present");
    link_object_with_runtime(&obj_path, &bin_path);
    let output = Command::new(&bin_path).output().expect("binary should run");
    assert_eq!(output.stdout, b"1\n");
}

#[test]
fn dict_get_or_default_on_a_missing_key_returns_the_default_codegens_and_runs() {
    // `d = {"a": 1}\nprint(d.get("z", -1))\n` end to end -- real
    // `MirExpr::DictGetOrDefault` codegen, missing-key path (unlike
    // `MirExpr::DictGet`, this never panics). Expected output verified
    // against `python3` on this exact source: `-1`.
    let mir = MirModule {
        items: vec![
            MirItem::TopLevelStmt(MirStmt::Assign {
                target: "d".to_string(),
                value: MirExpr::DictLiteral(vec![(
                    MirExpr::StringLiteral("a".to_string()),
                    MirExpr::IntLiteral(1),
                )]),
            }),
            MirItem::TopLevelStmt(print_expr(MirExpr::DictGetOrDefault {
                dict: pycc_mir::MirContainerReceiver::Name("d".to_string()),
                key: Box::new(MirExpr::StringLiteral("z".to_string())),
                default: Box::new(MirExpr::IntLiteral(-1)),
                ty: Ty::Int,
            })),
        ],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("dict_get_or_default_missing")
        .expect("failed to create scratch dir");
    let obj_path = dir.join("dict_get_or_default_missing.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
    let bin_path = dir.join("dict_get_or_default_missing");
    link_object_with_runtime(&obj_path, &bin_path);
    let output = Command::new(&bin_path).output().expect("binary should run");
    assert_eq!(output.stdout, b"-1\n");
}

#[test]
fn dict_get_or_default_nested_in_its_own_default_argument_resolves_correctly() {
    // `d = {"a": 1}\ny = d.get("k", d.get("a", -1))\nprint(y)\n` --
    // this task's own brief flags exactly this nested shape. Unlike
    // `list.pop()`, `dict.get()` never mutates its dict, so there is
    // no analogous ordering hazard to begin with (both the outer and
    // inner `.get()` read the same unchanged `d`) -- verified
    // empirically here rather than only by that reasoning: `"k"` is
    // absent, so the outer call's `default` sub-expression
    // (`d.get("a", -1)`) must itself be evaluated, and `"a"` is
    // present, so it resolves to `1`. Expected output verified against
    // `python3` on this exact source: `1`.
    let mir = MirModule {
        items: vec![
            MirItem::TopLevelStmt(MirStmt::Assign {
                target: "d".to_string(),
                value: MirExpr::DictLiteral(vec![(
                    MirExpr::StringLiteral("a".to_string()),
                    MirExpr::IntLiteral(1),
                )]),
            }),
            MirItem::TopLevelStmt(MirStmt::Assign {
                target: "y".to_string(),
                value: MirExpr::DictGetOrDefault {
                    dict: pycc_mir::MirContainerReceiver::Name("d".to_string()),
                    key: Box::new(MirExpr::StringLiteral("k".to_string())),
                    default: Box::new(MirExpr::DictGetOrDefault {
                        dict: pycc_mir::MirContainerReceiver::Name("d".to_string()),
                        key: Box::new(MirExpr::StringLiteral("a".to_string())),
                        default: Box::new(MirExpr::IntLiteral(-1)),
                        ty: Ty::Int,
                    }),
                    ty: Ty::Int,
                },
            }),
            MirItem::TopLevelStmt(print_expr(MirExpr::Name {
                name: "y".to_string(),
                ty: Ty::Int,
            })),
        ],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("dict_get_or_default_nested_default")
        .expect("failed to create scratch dir");
    let obj_path = dir.join("dict_get_or_default_nested_default.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
    let bin_path = dir.join("dict_get_or_default_nested_default");
    link_object_with_runtime(&obj_path, &bin_path);
    let output = Command::new(&bin_path).output().expect("binary should run");
    assert_eq!(output.stdout, b"1\n");
}

#[test]
fn function_redefinition_uses_unique_mangled_names() {
    // Issue #22: each `def` gets a unique mangled name so redefinition
    // doesn't collide (`pyfn_{name}` for the first, `pyfn_{name}__redef_{n}`
    // for subsequent). The global function-pointer slot is initialized to
    // null and updated at each def's source position; calls dispatch
    // indirectly through the slot.
    let mir = MirModule {
        items: vec![
            MirItem::Function {
                name: "foo".to_string(),
                params: vec![("x".to_string(), Ty::Int)],
                return_ty: Ty::Int,
                body: vec![MirStmt::Return(Some(MirExpr::Name {
                    name: "x".to_string(),
                    ty: Ty::Int,
                }))],
            },
            MirItem::Function {
                name: "foo".to_string(),
                params: vec![("x".to_string(), Ty::Int)],
                return_ty: Ty::Int,
                body: vec![MirStmt::Return(Some(MirExpr::Name {
                    name: "x".to_string(),
                    ty: Ty::Int,
                }))],
            },
            // Call foo(42) and print the result -- exercises the
            // indirect call dispatch through the function-pointer slot.
            MirItem::TopLevelStmt(print_expr(MirExpr::Call {
                callee: "foo".to_string(),
                args: vec![MirExpr::IntLiteral(42)],
                ty: Ty::Int,
            })),
        ],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("fn_redef_unique_names")
        .expect("failed to create scratch dir");
    let obj_path = dir.join("fn_redef_unique_names.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
    let bin_path = dir.join("fn_redef_unique_names");
    link_object_with_runtime(&obj_path, &bin_path);
    let output = Command::new(&bin_path).output().expect("binary should run");
    // The second definition (which returns its argument) is the one
    // bound at call time, so foo(42) should print 42.
    assert_eq!(
        output.stdout,
        b"42
"
    );
    assert!(output.status.success());
}

#[test]
fn set_add_grows_the_set_and_a_repeated_value_still_dedups_codegens_and_runs() {
    // `s = {1,2}\ns.add(3)\nprint(len(s))\ns.add(1)\nprint(len(s))\n`
    // end to end -- real `MirExpr::SetAdd` codegen, the second,
    // user-facing call site for the already-existing
    // `pycc_rt_int_set_add` (`SetLiteral`'s own per-element
    // construction is the first). Two separate statements, each its
    // own independent `MirStmt`, executed strictly in source order --
    // this task's own brief flags `s.add(x); s.add(x)`-shaped repeated
    // calls as a shape to verify dedup still holds for. Expected
    // output verified against `python3` on this exact source: `3`
    // then `3` again (the repeated `.add(1)` does not grow the set).
    let mir = MirModule {
        items: vec![
            MirItem::TopLevelStmt(MirStmt::Assign {
                target: "s".to_string(),
                value: MirExpr::SetLiteral {
                    elements: vec![MirExpr::IntLiteral(1), MirExpr::IntLiteral(2)],
                    ops: None,
                },
            }),
            MirItem::TopLevelStmt(MirStmt::ExprStmt(MirExpr::SetAdd {
                ops: None,
                set: "s".to_string(),
                value: Box::new(MirExpr::IntLiteral(3)),
            })),
            MirItem::TopLevelStmt(print_expr(MirExpr::Call {
                callee: "len".to_string(),
                args: vec![set_name("s")],
                ty: Ty::Int,
            })),
            MirItem::TopLevelStmt(MirStmt::ExprStmt(MirExpr::SetAdd {
                ops: None,
                set: "s".to_string(),
                value: Box::new(MirExpr::IntLiteral(1)),
            })),
            MirItem::TopLevelStmt(print_expr(MirExpr::Call {
                callee: "len".to_string(),
                args: vec![set_name("s")],
                ty: Ty::Int,
            })),
        ],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("set_add_grows_and_dedups")
        .expect("failed to create scratch dir");
    let obj_path = dir.join("set_add_grows_and_dedups.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
    let bin_path = dir.join("set_add_grows_and_dedups");
    link_object_with_runtime(&obj_path, &bin_path);
    let output = Command::new(&bin_path).output().expect("binary should run");
    assert_eq!(output.stdout, b"3\n3\n");
}
