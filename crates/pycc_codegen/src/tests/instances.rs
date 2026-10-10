//! Class instantiation, attribute slots and instance-valued functions.

use super::*;

#[test]
fn dict_literal_construction_codegens_and_runs() {
    // `x = {"a": 1, "b": 2}\nprint(len(x))\n` end to end through
    // `compile_to_object` -- real, previously-panicking `MirExpr::
    // DictLiteral` construction (PR-11 Task 5) plus `len()`'s new
    // `Ty::Dict` branch. Expected output verified against `python3` on
    // this exact source.
    let mir = MirModule {
        items: vec![
            MirItem::TopLevelStmt(MirStmt::Assign {
                target: "x".to_string(),
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
            MirItem::TopLevelStmt(print_expr(MirExpr::Call {
                callee: "len".to_string(),
                args: vec![dict_name("x")],
                ty: Ty::Int,
            })),
        ],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("dict_literal_and_len")
        .expect("failed to create scratch dir");
    let obj_path = dir.join("dict_literal_and_len.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
    let bin_path = dir.join("dict_literal_and_len");
    link_object_with_runtime(&obj_path, &bin_path);
    let output = Command::new(&bin_path).output().expect("binary should run");
    assert_eq!(output.stdout, b"2\n");
}

#[test]
fn class_instantiation_attribute_and_method_call_codegens_and_runs() {
    // D-154 (Part 1 of #375), end to end through `compile_to_object`:
    //
    //     class Point:
    //         def __init__(self, x: int, y: int) -> None:
    //             self.x = x
    //             self.y = y
    //         def bump(self) -> None:
    //             self.x = self.x + 1
    //
    //     p = Point(1, 2)
    //     p.bump()
    //     print(p.x)
    //     print(p.y)
    //
    // Expected output verified against `python3` on this exact source
    // (`p.x` starts `1`, `bump()` increments it once to `2`; `p.y`
    // stays `2`). Exercises every new codegen path this issue adds in
    // one program: `MirExpr::Instantiate` (allocation + `__init__`
    // call), `MirStmt::AttrSet` (both inside `__init__` and inside
    // `bump`), `MirExpr::AttrGet` (both inside `bump`'s own `self.x +
    // 1` and at module scope for the two `print` calls), and an
    // ordinary method call lowered to `MirExpr::Call` with `self`
    // prepended.
    let self_ty = instance_ty("Point");
    let init = MirItem::Function {
        name: "Point.__init__".to_string(),
        params: vec![
            ("self".to_string(), self_ty.clone()),
            ("x".to_string(), Ty::Int),
            ("y".to_string(), Ty::Int),
        ],
        return_ty: Ty::None,
        body: vec![
            MirStmt::AttrSet {
                base: MirExpr::Name {
                    name: "self".to_string(),
                    ty: self_ty.clone(),
                },
                slot: 0,
                value: MirExpr::Name {
                    name: "x".to_string(),
                    ty: Ty::Int,
                },
            },
            MirStmt::AttrSet {
                base: MirExpr::Name {
                    name: "self".to_string(),
                    ty: self_ty.clone(),
                },
                slot: 1,
                value: MirExpr::Name {
                    name: "y".to_string(),
                    ty: Ty::Int,
                },
            },
            MirStmt::Return(None),
        ],
    };
    let bump = MirItem::Function {
        name: "Point.bump".to_string(),
        params: vec![("self".to_string(), self_ty.clone())],
        return_ty: Ty::None,
        body: vec![
            MirStmt::AttrSet {
                base: MirExpr::Name {
                    name: "self".to_string(),
                    ty: self_ty.clone(),
                },
                slot: 0,
                value: MirExpr::BinOp {
                    op: BinOpKind::Add,
                    left: Box::new(MirExpr::AttrGet {
                        base: Box::new(MirExpr::Name {
                            name: "self".to_string(),
                            ty: self_ty.clone(),
                        }),
                        slot: 0,
                        ty: Ty::Int,
                    }),
                    right: Box::new(MirExpr::IntLiteral(1)),
                    ty: Ty::Int,
                },
            },
            MirStmt::Return(None),
        ],
    };
    let mir = MirModule {
        items: vec![
            init,
            bump,
            MirItem::TopLevelStmt(MirStmt::Assign {
                target: "p".to_string(),
                value: MirExpr::Instantiate(Box::new(pycc_mir::InstantiateExpr {
                    ctor: "Point.__init__".to_string(),
                    class_name: "C".to_string(),
                    slot_names: vec!["s0".to_string(), "s1".to_string()],
                    args: vec![MirExpr::IntLiteral(1), MirExpr::IntLiteral(2)],
                    ty: self_ty.clone(),
                })),
            }),
            MirItem::TopLevelStmt(MirStmt::ExprStmt(MirExpr::Call {
                callee: "Point.bump".to_string(),
                args: vec![MirExpr::Name {
                    name: "p".to_string(),
                    ty: self_ty.clone(),
                }],
                ty: Ty::None,
            })),
            MirItem::TopLevelStmt(print_expr(MirExpr::AttrGet {
                base: Box::new(MirExpr::Name {
                    name: "p".to_string(),
                    ty: self_ty.clone(),
                }),
                slot: 0,
                ty: Ty::Int,
            })),
            MirItem::TopLevelStmt(print_expr(MirExpr::AttrGet {
                base: Box::new(MirExpr::Name {
                    name: "p".to_string(),
                    ty: self_ty,
                }),
                slot: 1,
                ty: Ty::Int,
            })),
        ],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("class_instantiation_attribute_and_method_call")
        .expect("failed to create scratch dir");
    let obj_path = dir.join("class_instantiation_attribute_and_method_call.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
    let bin_path = dir.join("class_instantiation_attribute_and_method_call");
    link_object_with_runtime(&obj_path, &bin_path);
    let output = Command::new(&bin_path).output().expect("binary should run");
    assert_eq!(output.stdout, b"2\n2\n");
}

#[test]
fn bool_float_and_str_typed_attribute_slots_round_trip_correctly() {
    // `class_instantiation_attribute_and_method_call_codegens_and_runs`
    // above only ever exercises `int`-typed attribute slots --
    // `slot_word_to_scalar`/`scalar_to_slot_word`'s own `Bool`/`Float`/
    // `Str` arms (D-154) need their own exercise. `__init__` forwarding
    // each constructor parameter straight into its own attribute slot
    // exercises both the write (`scalar_to_slot_word`, during
    // `__init__`) and the read (`slot_word_to_scalar`, at each
    // `print(w.<attr>)` below) halves of all three in one program:
    //
    //     class Widget:
    //         def __init__(self, flag: bool, ratio: float, label: str) -> None:
    //             self.flag = flag
    //             self.ratio = ratio
    //             self.label = label
    //
    //     w = Widget(True, 2.5, "hi")
    //     print(w.flag)
    //     print(w.ratio)
    //     print(w.label)
    //
    // Expected output verified against `python3` on this exact source.
    let self_ty = instance_ty("Widget");
    let init = MirItem::Function {
        name: "Widget.__init__".to_string(),
        params: vec![
            ("self".to_string(), self_ty.clone()),
            ("flag".to_string(), Ty::Bool),
            ("ratio".to_string(), Ty::Float),
            ("label".to_string(), Ty::Str),
        ],
        return_ty: Ty::None,
        body: vec![
            MirStmt::AttrSet {
                base: MirExpr::Name {
                    name: "self".to_string(),
                    ty: self_ty.clone(),
                },
                slot: 0,
                value: MirExpr::Name {
                    name: "flag".to_string(),
                    ty: Ty::Bool,
                },
            },
            MirStmt::AttrSet {
                base: MirExpr::Name {
                    name: "self".to_string(),
                    ty: self_ty.clone(),
                },
                slot: 1,
                value: MirExpr::Name {
                    name: "ratio".to_string(),
                    ty: Ty::Float,
                },
            },
            MirStmt::AttrSet {
                base: MirExpr::Name {
                    name: "self".to_string(),
                    ty: self_ty.clone(),
                },
                slot: 2,
                value: MirExpr::Name {
                    name: "label".to_string(),
                    ty: Ty::Str,
                },
            },
            MirStmt::Return(None),
        ],
    };
    let mir = MirModule {
        items: vec![
            init,
            MirItem::TopLevelStmt(MirStmt::Assign {
                target: "w".to_string(),
                value: MirExpr::Instantiate(Box::new(pycc_mir::InstantiateExpr {
                    ctor: "Widget.__init__".to_string(),
                    class_name: "C".to_string(),
                    slot_names: vec!["s0".to_string(), "s1".to_string(), "s2".to_string()],
                    args: vec![
                        MirExpr::BoolLiteral(true),
                        MirExpr::FloatLiteral(2.5),
                        MirExpr::StringLiteral("hi".to_string()),
                    ],
                    ty: self_ty.clone(),
                })),
            }),
            MirItem::TopLevelStmt(print_expr(MirExpr::AttrGet {
                base: Box::new(MirExpr::Name {
                    name: "w".to_string(),
                    ty: self_ty.clone(),
                }),
                slot: 0,
                ty: Ty::Bool,
            })),
            MirItem::TopLevelStmt(print_expr(MirExpr::AttrGet {
                base: Box::new(MirExpr::Name {
                    name: "w".to_string(),
                    ty: self_ty.clone(),
                }),
                slot: 1,
                ty: Ty::Float,
            })),
            MirItem::TopLevelStmt(print_expr(MirExpr::AttrGet {
                base: Box::new(MirExpr::Name {
                    name: "w".to_string(),
                    ty: self_ty,
                }),
                slot: 2,
                ty: Ty::Str,
            })),
        ],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("bool_float_str_attribute_slots")
        .expect("failed to create scratch dir");
    let obj_path = dir.join("bool_float_str_attribute_slots.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
    let bin_path = dir.join("bool_float_str_attribute_slots");
    link_object_with_runtime(&obj_path, &bin_path);
    let output = Command::new(&bin_path).output().expect("binary should run");
    assert_eq!(output.stdout, b"True\n2.5\nhi\n");
}

#[test]
fn a_str_attribute_read_twice_and_then_reassigned_does_not_use_after_free() {
    // D-154 Part 1's own post-merge review finding: the first version
    // of `MirStmt::AttrSet`'s codegen neither incref'd a `str` value
    // read out of an instance attribute (`str_value_is_a_duplicate_
    // reference` had no `MirExpr::AttrGet` arm) nor decref'd a slot's
    // previous `str` occupant before overwriting it. Reading a `str`
    // attribute a second time reliably observed freed memory (the
    // first `print` already decrefs the read pointer to 0, freeing it,
    // since the read was wrongly treated as a fresh, unshared value),
    // and `bool_float_and_str_typed_attribute_slots_round_trip_
    // correctly` above cannot catch this: it reads `w.label` exactly
    // once, which structurally cannot observe a premature free.
    //
    //     class Widget:
    //         def __init__(self, label: str) -> None:
    //             self.label = label
    //         def relabel(self, new_label: str) -> None:
    //             self.label = new_label
    //
    //     w = Widget("hi")
    //     print(w.label)
    //     print(w.label)
    //     w.relabel("bye")
    //     print(w.label)
    //
    // Exercises both halves of the fix in one program: the two
    // `print(w.label)` calls before `relabel` exercise
    // `incref_if_str_duplicate`'s new `AttrGet` arm (without it, the
    // second `print` reads freed memory); `relabel`'s own
    // `self.label = new_label` exercises
    // `decref_str_attr_slot_before_store` (without it, `"hi"`'s
    // `PyStrObj` leaks rather than being released when overwritten --
    // not itself a crash this test can observe, but exercised by the
    // same code path the crash-causing half shares).
    let self_ty = instance_ty("Widget");
    let init = MirItem::Function {
        name: "Widget.__init__".to_string(),
        params: vec![
            ("self".to_string(), self_ty.clone()),
            ("label".to_string(), Ty::Str),
        ],
        return_ty: Ty::None,
        body: vec![
            MirStmt::AttrSet {
                base: MirExpr::Name {
                    name: "self".to_string(),
                    ty: self_ty.clone(),
                },
                slot: 0,
                value: MirExpr::Name {
                    name: "label".to_string(),
                    ty: Ty::Str,
                },
            },
            MirStmt::Return(None),
        ],
    };
    let relabel = MirItem::Function {
        name: "Widget.relabel".to_string(),
        params: vec![
            ("self".to_string(), self_ty.clone()),
            ("new_label".to_string(), Ty::Str),
        ],
        return_ty: Ty::None,
        body: vec![
            MirStmt::AttrSet {
                base: MirExpr::Name {
                    name: "self".to_string(),
                    ty: self_ty.clone(),
                },
                slot: 0,
                value: MirExpr::Name {
                    name: "new_label".to_string(),
                    ty: Ty::Str,
                },
            },
            MirStmt::Return(None),
        ],
    };
    let mir = MirModule {
        items: vec![
            init,
            relabel,
            MirItem::TopLevelStmt(MirStmt::Assign {
                target: "w".to_string(),
                value: MirExpr::Instantiate(Box::new(pycc_mir::InstantiateExpr {
                    ctor: "Widget.__init__".to_string(),
                    class_name: "C".to_string(),
                    slot_names: vec!["s0".to_string()],
                    args: vec![MirExpr::StringLiteral("hi".to_string())],
                    ty: self_ty.clone(),
                })),
            }),
            MirItem::TopLevelStmt(print_expr(MirExpr::AttrGet {
                base: Box::new(MirExpr::Name {
                    name: "w".to_string(),
                    ty: self_ty.clone(),
                }),
                slot: 0,
                ty: Ty::Str,
            })),
            MirItem::TopLevelStmt(print_expr(MirExpr::AttrGet {
                base: Box::new(MirExpr::Name {
                    name: "w".to_string(),
                    ty: self_ty.clone(),
                }),
                slot: 0,
                ty: Ty::Str,
            })),
            MirItem::TopLevelStmt(MirStmt::ExprStmt(MirExpr::Call {
                callee: "Widget.relabel".to_string(),
                args: vec![
                    MirExpr::Name {
                        name: "w".to_string(),
                        ty: self_ty.clone(),
                    },
                    MirExpr::StringLiteral("bye".to_string()),
                ],
                ty: Ty::None,
            })),
            MirItem::TopLevelStmt(print_expr(MirExpr::AttrGet {
                base: Box::new(MirExpr::Name {
                    name: "w".to_string(),
                    ty: self_ty,
                }),
                slot: 0,
                ty: Ty::Str,
            })),
        ],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("str_attribute_read_twice_and_reassigned")
        .expect("failed to create scratch dir");
    let obj_path = dir.join("str_attribute_read_twice_and_reassigned.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
    let bin_path = dir.join("str_attribute_read_twice_and_reassigned");
    link_object_with_runtime(&obj_path, &bin_path);
    let output = Command::new(&bin_path).output().expect("binary should run");
    assert_eq!(output.stdout, b"hi\nhi\nbye\n");
}

#[test]
#[should_panic(expected = "an instance attribute of type `int | None` is not supported yet")]
fn slot_word_to_scalar_rejects_an_unsupported_attribute_type() {
    // `pycc_hir::class::init_slot::slot_ty_from_init_rhs` admits only a
    // container or scalar slot, never an `Optional` one, so an
    // `int | None`-typed slot can never reach this function from real,
    // type-checked source -- hand-built directly, matching this file's
    // own established internal-error-test convention (e.g.
    // `to_numeric_encoded_int_rejects_a_list_operand` above).
    let context = Context::create();
    let module = context.create_module("test");
    let builder = context.create_builder();
    let raw = context.i64_type().const_int(0, false);
    slot_word_to_scalar(
        &context,
        &builder,
        raw,
        &pycc_mir::Ty::Optional(Box::new(pycc_mir::Ty::Int)),
    );
    let _ = module;
}

#[test]
#[should_panic(expected = "cannot store this value into an instance attribute slot")]
fn scalar_to_slot_word_rejects_an_unsupported_scalar() {
    // Mirror image of `slot_word_to_scalar_rejects_an_unsupported_attribute_type`
    // above, for the write direction: a struct `Scalar::Tuple` can never
    // reach `scalar_to_slot_word` from real, type-checked source either
    // (a slot stores a tuple boxed).
    let context = Context::create();
    let module = context.create_module("test");
    let builder = context.create_builder();
    let tuple = context
        .struct_type(&[context.i64_type().into()], false)
        .const_zero();
    scalar_to_slot_word(&context, &builder, Scalar::Tuple(tuple));
    let _ = module;
}

#[test]
#[should_panic(expected = "did not evaluate to a class instance")]
fn attribute_read_over_a_non_instance_base_panics_with_an_internal_error() {
    // Bypasses `pycc_types::check` (T0043 would reject this) with a
    // hand-built `MirExpr::AttrGet` over an `int`-typed base, matching
    // `pycc_mir`'s own established internal-error-test convention.
    let mir = MirModule {
        items: vec![MirItem::TopLevelStmt(print_expr(MirExpr::AttrGet {
            base: Box::new(MirExpr::IntLiteral(1)),
            slot: 0,
            ty: Ty::Int,
        }))],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("attribute_read_over_a_non_instance_base")
        .expect("failed to create scratch dir");
    let obj_path = dir.join("attribute_read_over_a_non_instance_base.o");
    let _ = compile_to_object(&mir, &obj_path, None, false);
}

#[test]
#[should_panic(expected = "should have been registered as an ordinary user function")]
fn instantiation_of_an_unregistered_constructor_panics_with_an_internal_error() {
    // Bypasses `pycc_hir::class::lower_class` (which always mangles
    // `__init__` into `HirModule::items`) with a hand-built
    // `MirExpr::Instantiate` naming a constructor no `MirItem::Function`
    // ever declares.
    let mir = MirModule {
        items: vec![MirItem::TopLevelStmt(MirStmt::Assign {
            target: "g".to_string(),
            value: MirExpr::Instantiate(Box::new(pycc_mir::InstantiateExpr {
                ctor: "Ghost.__init__".to_string(),
                class_name: "C".to_string(),
                slot_names: vec![],
                args: vec![],
                ty: instance_ty("Ghost"),
            })),
        })],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("instantiation_of_an_unregistered_constructor")
        .expect("failed to create scratch dir");
    let obj_path = dir.join("instantiation_of_an_unregistered_constructor.o");
    let _ = compile_to_object(&mir, &obj_path, None, false);
}

#[test]
fn a_function_returning_an_instance_codegens_and_runs() {
    // `Ty::Instance`-typed call/return results are not reachable from
    // this PR's own frontend today (a method's return type is never
    // resolved to `Ty::Instance` by `pycc_types` -- see the plan's own
    // out-of-scope list for class-typed annotations), but
    // `emit_stmt`'s `Return` arm's own `Scalar::Instance` pass-through
    // (D-154) is still real, load-bearing codegen (a future PR that
    // does support `-> Self`/`-> ClassName` needs no further work
    // here) -- exercised directly with hand-built MIR: an `identity`
    // function that takes and returns a `Point`, called once and its
    // result's own attribute printed to prove the same pointer round-
    // trips correctly.
    let self_ty = instance_ty("Point");
    let init = MirItem::Function {
        name: "Point.__init__".to_string(),
        params: vec![
            ("self".to_string(), self_ty.clone()),
            ("x".to_string(), Ty::Int),
        ],
        return_ty: Ty::None,
        body: vec![
            MirStmt::AttrSet {
                base: MirExpr::Name {
                    name: "self".to_string(),
                    ty: self_ty.clone(),
                },
                slot: 0,
                value: MirExpr::Name {
                    name: "x".to_string(),
                    ty: Ty::Int,
                },
            },
            MirStmt::Return(None),
        ],
    };
    let identity = MirItem::Function {
        name: "identity".to_string(),
        params: vec![("self".to_string(), self_ty.clone())],
        return_ty: self_ty.clone(),
        body: vec![MirStmt::Return(Some(MirExpr::Name {
            name: "self".to_string(),
            ty: self_ty.clone(),
        }))],
    };
    let mir = MirModule {
        items: vec![
            init,
            identity,
            MirItem::TopLevelStmt(MirStmt::Assign {
                target: "p".to_string(),
                value: MirExpr::Instantiate(Box::new(pycc_mir::InstantiateExpr {
                    ctor: "Point.__init__".to_string(),
                    class_name: "C".to_string(),
                    slot_names: vec!["s0".to_string()],
                    args: vec![MirExpr::IntLiteral(7)],
                    ty: self_ty.clone(),
                })),
            }),
            MirItem::TopLevelStmt(MirStmt::Assign {
                target: "p2".to_string(),
                value: MirExpr::Call {
                    callee: "identity".to_string(),
                    args: vec![MirExpr::Name {
                        name: "p".to_string(),
                        ty: self_ty.clone(),
                    }],
                    ty: self_ty.clone(),
                },
            }),
            MirItem::TopLevelStmt(print_expr(MirExpr::AttrGet {
                base: Box::new(MirExpr::Name {
                    name: "p2".to_string(),
                    ty: self_ty,
                }),
                slot: 0,
                ty: Ty::Int,
            })),
        ],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("a_function_returning_an_instance")
        .expect("failed to create scratch dir");
    let obj_path = dir.join("a_function_returning_an_instance.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
    let bin_path = dir.join("a_function_returning_an_instance");
    link_object_with_runtime(&obj_path, &bin_path);
    let output = Command::new(&bin_path).output().expect("binary should run");
    assert_eq!(output.stdout, b"7\n");
}

#[test]
fn instantiating_a_class_at_module_scope_with_a_truthiness_check_codegens_and_runs() {
    // `p = Point(1, 2)\nif p:\n    print(1)\n` -- exercises `truthy`'s
    // new `Scalar::Instance` arm (always `True`, this PR's own
    // documented default-object rule, see that arm's doc comment) and
    // `declare_module_globals`'s new `Ty::Instance` arm together.
    let self_ty = instance_ty("Point");
    let init = MirItem::Function {
        name: "Point.__init__".to_string(),
        params: vec![
            ("self".to_string(), self_ty.clone()),
            ("x".to_string(), Ty::Int),
            ("y".to_string(), Ty::Int),
        ],
        return_ty: Ty::None,
        body: vec![
            MirStmt::AttrSet {
                base: MirExpr::Name {
                    name: "self".to_string(),
                    ty: self_ty.clone(),
                },
                slot: 0,
                value: MirExpr::Name {
                    name: "x".to_string(),
                    ty: Ty::Int,
                },
            },
            MirStmt::AttrSet {
                base: MirExpr::Name {
                    name: "self".to_string(),
                    ty: self_ty.clone(),
                },
                slot: 1,
                value: MirExpr::Name {
                    name: "y".to_string(),
                    ty: Ty::Int,
                },
            },
            MirStmt::Return(None),
        ],
    };
    let mir = MirModule {
        items: vec![
            init,
            MirItem::TopLevelStmt(MirStmt::Assign {
                target: "p".to_string(),
                value: MirExpr::Instantiate(Box::new(pycc_mir::InstantiateExpr {
                    ctor: "Point.__init__".to_string(),
                    class_name: "C".to_string(),
                    slot_names: vec!["s0".to_string(), "s1".to_string()],
                    args: vec![MirExpr::IntLiteral(1), MirExpr::IntLiteral(2)],
                    ty: self_ty.clone(),
                })),
            }),
            MirItem::TopLevelStmt(MirStmt::If {
                test: MirExpr::Name {
                    name: "p".to_string(),
                    ty: self_ty,
                },
                body: vec![print_expr(MirExpr::IntLiteral(1))],
                orelse: vec![],
            }),
        ],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("class_instance_truthiness")
        .expect("failed to create scratch dir");
    let obj_path = dir.join("class_instance_truthiness.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
    let bin_path = dir.join("class_instance_truthiness");
    link_object_with_runtime(&obj_path, &bin_path);
    let output = Command::new(&bin_path).output().expect("binary should run");
    assert_eq!(output.stdout, b"1\n");
}

#[test]
#[should_panic(
    expected = "pycc_codegen: string conversion of a class instance without `__repr__` is not supported yet"
)]
fn string_conversion_of_a_class_instance_panics_honestly() {
    // Mirrors `string_conversion_of_a_list_value_panics_honestly`
    // above exactly. #378 (PR-18) added `__repr__` support for dataclass
    // instances (the MIR rewrites `print(instance)` to a `__repr__` call
    // before codegen), and since #977 (D-237) `pycc_types` rejects every
    // other instance with `C0001` before lowering, so this arm is
    // reachable only from a hand-built `Scalar::Instance` like the one
    // below -- where it still panics honestly instead of handing a
    // `PyInstanceObj` pointer to a `pycc_rt_*_to_str` function expecting
    // a `PyStrObj`.
    let context = Context::create();
    let (_module, rt) = list_scalar_panic_fixture(&context);
    let builder = context.create_builder();
    let ptr = context
        .ptr_type(inkwell::AddressSpace::default())
        .const_null();
    to_str(&builder, &rt, Scalar::Instance(ptr));
}

#[test]
#[should_panic(expected = "internal error: expected an int-or-bool operand, got instance")]
fn to_numeric_encoded_int_rejects_an_instance_operand() {
    // Mirrors `to_numeric_encoded_int_rejects_a_list_operand` above:
    // `pycc_types`' `numeric_result_type` has no `as_numeric` mapping
    // for `Ty::Instance`, so any arithmetic with a class-instance
    // operand is rejected as `T0021` long before codegen -- this is
    // genuinely defensive, not a reachable feature gap.
    let context = Context::create();
    let ptr = context
        .ptr_type(inkwell::AddressSpace::default())
        .const_null();
    let builder = context.create_builder();
    to_numeric_encoded_int(&context, &builder, Scalar::Instance(ptr));
}

#[test]
#[should_panic(expected = "internal error: expected a numeric operand, got instance")]
fn to_float_rejects_an_instance_operand() {
    let context = Context::create();
    let (_module, rt) = list_scalar_panic_fixture(&context);
    let builder = context.create_builder();
    let ptr = context
        .ptr_type(inkwell::AddressSpace::default())
        .const_null();
    to_float(&context, &builder, &rt, Scalar::Instance(ptr));
}

#[test]
#[should_panic(expected = "pycc_codegen: internal error: range() start did not evaluate to int")]
fn range_operand_to_normalized_int_rejects_an_instance_operand() {
    let context = Context::create();
    let (_module, rt) = list_scalar_panic_fixture(&context);
    let builder = context.create_builder();
    let ptr = context
        .ptr_type(inkwell::AddressSpace::default())
        .const_null();
    range_operand_to_normalized_int(&context, &builder, &rt, Scalar::Instance(ptr), "start");
}
