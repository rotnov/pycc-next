//! `Optional[float]` and `Optional[bool]` construction, narrowing and bare `None`.

use super::*;

// --- `Optional[float]`/`Optional[bool]` (#809, Part 3 of #747) --------

/// `float | None`, the widened-by-#809 counterpart of `optional_int()`
/// immediately above.
fn optional_float() -> Ty {
    Ty::Optional(Box::new(Ty::Float))
}

/// `bool | None`, the widened-by-#809 counterpart of `optional_int()`
/// immediately above.
fn optional_bool() -> Ty {
    Ty::Optional(Box::new(Ty::Bool))
}

#[test]
fn optional_float_annotated_assignment_constructs_a_present_struct_and_reads_its_payload() {
    // `x: float | None = 5.5` followed by `print(x is None)` and
    // `print(x is not None)` -- the `Optional[float]` counterpart of
    // `optional_int_annotated_assignment_constructs_a_present_struct_and_
    // reads_its_payload` above, proving `declare_module_globals`'s
    // widened `Ty::Optional` arm (this PR's own fix -- the module-scope
    // initializer previously forced every inner type's payload field
    // through `tag_smallint_const`, an LLVM constant type mismatch for
    // `f64`/`i8` fields that crashed the LLVM backend with "invalid
    // number of bytes" for `bool`) and `coerce_scalar_to_type`'s
    // `(inner, coerced)`-paired bare-payload arm both handle a real
    // `float` payload correctly.
    let mir = MirModule {
        items: vec![
            MirItem::TopLevelStmt(MirStmt::Assign {
                target: "x".to_string(),
                value: MirExpr::OptionalWrap(
                    Box::new(MirExpr::FloatLiteral(5.5)),
                    Box::new(Ty::Float),
                ),
            }),
            MirItem::TopLevelStmt(print_expr(MirExpr::Compare {
                op: pycc_mir::CmpOpKind::Is,
                left: Box::new(MirExpr::Name {
                    name: "x".to_string(),
                    ty: optional_float(),
                }),
                right: Box::new(MirExpr::NoneLiteral),
                ty: Ty::Bool,
            })),
            MirItem::TopLevelStmt(print_expr(MirExpr::Compare {
                op: pycc_mir::CmpOpKind::IsNot,
                left: Box::new(MirExpr::Name {
                    name: "x".to_string(),
                    ty: optional_float(),
                }),
                right: Box::new(MirExpr::NoneLiteral),
                ty: Ty::Bool,
            })),
        ],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("optional_float_present_is_none")
        .expect("failed to create scratch dir");
    let obj_path = dir.join("optional_float_present_is_none.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
    let bin_path = dir.join("optional_float_present_is_none");
    link_object_with_runtime(&obj_path, &bin_path);
    let output = Command::new(&bin_path).output().expect("binary should run");
    assert_eq!(output.stdout, b"False\nTrue\n");
}

#[test]
fn optional_bool_annotated_assignment_constructs_a_present_struct_and_reads_its_payload() {
    // The `Optional[bool]` counterpart of the `Optional[float]` test
    // immediately above -- `x: bool | None = True`. This exact shape is
    // what reproduced the module-global-initializer LLVM crash this PR's
    // `declare_module_globals` fix resolves (a `bool`'s `i8` payload field
    // being handed an `i64`-typed `tag_smallint_const` constant).
    let mir = MirModule {
        items: vec![
            MirItem::TopLevelStmt(MirStmt::Assign {
                target: "x".to_string(),
                value: MirExpr::OptionalWrap(
                    Box::new(MirExpr::BoolLiteral(true)),
                    Box::new(Ty::Bool),
                ),
            }),
            MirItem::TopLevelStmt(print_expr(MirExpr::Compare {
                op: pycc_mir::CmpOpKind::Is,
                left: Box::new(MirExpr::Name {
                    name: "x".to_string(),
                    ty: optional_bool(),
                }),
                right: Box::new(MirExpr::NoneLiteral),
                ty: Ty::Bool,
            })),
            MirItem::TopLevelStmt(print_expr(MirExpr::Compare {
                op: pycc_mir::CmpOpKind::IsNot,
                left: Box::new(MirExpr::Name {
                    name: "x".to_string(),
                    ty: optional_bool(),
                }),
                right: Box::new(MirExpr::NoneLiteral),
                ty: Ty::Bool,
            })),
        ],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("optional_bool_present_is_none")
        .expect("failed to create scratch dir");
    let obj_path = dir.join("optional_bool_present_is_none.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
    let bin_path = dir.join("optional_bool_present_is_none");
    link_object_with_runtime(&obj_path, &bin_path);
    let output = Command::new(&bin_path).output().expect("binary should run");
    assert_eq!(output.stdout, b"False\nTrue\n");
}

#[test]
fn optional_float_annotated_assignment_with_bare_none_constructs_an_absent_struct() {
    // `x: float | None = None` -- the mirror-image case of the present
    // test above, exercising `coerce_scalar_to_type`'s
    // placeholder-to-real-struct-shape branch for a `Ty::Float` inner
    // type, and `default_value_for_type`'s `Ty::Optional` arm recursing
    // into its own `Ty::Float` arm for the placeholder payload.
    let mir = MirModule {
        items: vec![
            MirItem::TopLevelStmt(MirStmt::Assign {
                target: "x".to_string(),
                value: MirExpr::OptionalWrap(Box::new(MirExpr::NoneLiteral), Box::new(Ty::Float)),
            }),
            MirItem::TopLevelStmt(print_expr(MirExpr::Compare {
                op: pycc_mir::CmpOpKind::Is,
                left: Box::new(MirExpr::Name {
                    name: "x".to_string(),
                    ty: optional_float(),
                }),
                right: Box::new(MirExpr::NoneLiteral),
                ty: Ty::Bool,
            })),
            MirItem::TopLevelStmt(print_expr(MirExpr::Compare {
                op: pycc_mir::CmpOpKind::IsNot,
                left: Box::new(MirExpr::Name {
                    name: "x".to_string(),
                    ty: optional_float(),
                }),
                right: Box::new(MirExpr::NoneLiteral),
                ty: Ty::Bool,
            })),
        ],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("optional_float_absent_is_none")
        .expect("failed to create scratch dir");
    let obj_path = dir.join("optional_float_absent_is_none.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
    let bin_path = dir.join("optional_float_absent_is_none");
    link_object_with_runtime(&obj_path, &bin_path);
    let output = Command::new(&bin_path).output().expect("binary should run");
    assert_eq!(output.stdout, b"True\nFalse\n");
}

#[test]
fn optional_bool_annotated_assignment_with_bare_none_constructs_an_absent_struct() {
    // `x: bool | None = None`, the `Ty::Bool` counterpart of the
    // `Optional[float]` absent test immediately above -- this is also
    // the exact module-global shape that reproduced the LLVM "invalid
    // number of bytes" crash (a bare `None` literal reaches
    // `declare_module_globals`'s `Ty::Optional(Ty::Bool)` arm exactly as
    // directly as the present-value case does).
    let mir = MirModule {
        items: vec![
            MirItem::TopLevelStmt(MirStmt::Assign {
                target: "x".to_string(),
                value: MirExpr::OptionalWrap(Box::new(MirExpr::NoneLiteral), Box::new(Ty::Bool)),
            }),
            MirItem::TopLevelStmt(print_expr(MirExpr::Compare {
                op: pycc_mir::CmpOpKind::Is,
                left: Box::new(MirExpr::Name {
                    name: "x".to_string(),
                    ty: optional_bool(),
                }),
                right: Box::new(MirExpr::NoneLiteral),
                ty: Ty::Bool,
            })),
            MirItem::TopLevelStmt(print_expr(MirExpr::Compare {
                op: pycc_mir::CmpOpKind::IsNot,
                left: Box::new(MirExpr::Name {
                    name: "x".to_string(),
                    ty: optional_bool(),
                }),
                right: Box::new(MirExpr::NoneLiteral),
                ty: Ty::Bool,
            })),
        ],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("optional_bool_absent_is_none")
        .expect("failed to create scratch dir");
    let obj_path = dir.join("optional_bool_absent_is_none.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
    let bin_path = dir.join("optional_bool_absent_is_none");
    link_object_with_runtime(&obj_path, &bin_path);
    let output = Command::new(&bin_path).output().expect("binary should run");
    assert_eq!(output.stdout, b"True\nFalse\n");
}

#[test]
fn optional_float_truthiness_follows_cpython_for_present_and_absent_values() {
    // `if x:` for three `Optional[float]` values -- absent (`None`),
    // present with a falsy payload (`0.0`), and present with a truthy
    // payload (`5.5`) -- exercising `truthy`'s `Scalar::Optional` arm's
    // `FloatValue` dispatch branch (this PR's own fix: previously this
    // arm always treated the payload as `int` and called
    // `pycc_rt_int_truthy` on it, which would either panic or misread an
    // `f64`'s raw bits as a D-141-encoded word).
    fn if_prints_one_else_zero(name: &str, ty: Ty) -> MirItem {
        MirItem::TopLevelStmt(MirStmt::If {
            test: MirExpr::Name {
                name: name.to_string(),
                ty,
            },
            body: vec![print_expr(MirExpr::IntLiteral(1))],
            orelse: vec![print_expr(MirExpr::IntLiteral(0))],
        })
    }
    let mir = MirModule {
        items: vec![
            MirItem::TopLevelStmt(MirStmt::Assign {
                target: "a".to_string(),
                value: MirExpr::OptionalWrap(Box::new(MirExpr::NoneLiteral), Box::new(Ty::Float)),
            }),
            MirItem::TopLevelStmt(MirStmt::Assign {
                target: "b".to_string(),
                value: MirExpr::OptionalWrap(
                    Box::new(MirExpr::FloatLiteral(0.0)),
                    Box::new(Ty::Float),
                ),
            }),
            MirItem::TopLevelStmt(MirStmt::Assign {
                target: "c".to_string(),
                value: MirExpr::OptionalWrap(
                    Box::new(MirExpr::FloatLiteral(5.5)),
                    Box::new(Ty::Float),
                ),
            }),
            if_prints_one_else_zero("a", optional_float()),
            if_prints_one_else_zero("b", optional_float()),
            if_prints_one_else_zero("c", optional_float()),
        ],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("optional_float_truthiness")
        .expect("failed to create scratch dir");
    let obj_path = dir.join("optional_float_truthiness.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
    let bin_path = dir.join("optional_float_truthiness");
    link_object_with_runtime(&obj_path, &bin_path);
    let output = Command::new(&bin_path).output().expect("binary should run");
    assert_eq!(output.stdout, b"0\n0\n1\n");
}

#[test]
fn optional_bool_truthiness_follows_cpython_for_present_and_absent_values() {
    // The `Optional[bool]` counterpart of the `Optional[float]`
    // truthiness test above -- absent (`None`), present-`False`, and
    // present-`True` -- exercising `truthy`'s `Scalar::Optional` arm's
    // plain-`i8` (bit-width-8 `IntValue`) dispatch branch, distinct from
    // both the `f64` branch above and the D-141-encoded-`int` branch
    // `optional_int_truthiness_follows_cpython_for_present_and_absent_
    // values` already covers. Also stands in for the plan's explicit
    // "`x: bool | None = None`" absent-case requirement: the `a` case
    // below is exactly that assignment, and its `False` output proves
    // both the placeholder payload (an uninitialized/mismatched `i8`
    // payload here would still read as some fixed bit pattern, but the
    // present flag alone -- not the payload -- must correctly gate this
    // result to `False`) and the present-flag-driven `Scalar::Optional`
    // truthiness path are both correct for the absent case.
    fn if_prints_one_else_zero(name: &str, ty: Ty) -> MirItem {
        MirItem::TopLevelStmt(MirStmt::If {
            test: MirExpr::Name {
                name: name.to_string(),
                ty,
            },
            body: vec![print_expr(MirExpr::IntLiteral(1))],
            orelse: vec![print_expr(MirExpr::IntLiteral(0))],
        })
    }
    let mir = MirModule {
        items: vec![
            MirItem::TopLevelStmt(MirStmt::Assign {
                target: "a".to_string(),
                value: MirExpr::OptionalWrap(Box::new(MirExpr::NoneLiteral), Box::new(Ty::Bool)),
            }),
            MirItem::TopLevelStmt(MirStmt::Assign {
                target: "b".to_string(),
                value: MirExpr::OptionalWrap(
                    Box::new(MirExpr::BoolLiteral(false)),
                    Box::new(Ty::Bool),
                ),
            }),
            MirItem::TopLevelStmt(MirStmt::Assign {
                target: "c".to_string(),
                value: MirExpr::OptionalWrap(
                    Box::new(MirExpr::BoolLiteral(true)),
                    Box::new(Ty::Bool),
                ),
            }),
            if_prints_one_else_zero("a", optional_bool()),
            if_prints_one_else_zero("b", optional_bool()),
            if_prints_one_else_zero("c", optional_bool()),
        ],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("optional_bool_truthiness")
        .expect("failed to create scratch dir");
    let obj_path = dir.join("optional_bool_truthiness.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
    let bin_path = dir.join("optional_bool_truthiness");
    link_object_with_runtime(&obj_path, &bin_path);
    let output = Command::new(&bin_path).output().expect("binary should run");
    assert_eq!(output.stdout, b"0\n0\n1\n");
}

#[test]
fn optional_float_narrowed_read_of_a_present_value_prints_the_payload() {
    // `x: float | None = 5.5; if x is not None: print(x)` -- the
    // `Optional[float]` counterpart of
    // `optional_int_narrowed_read_of_a_present_smallint_prints_the_
    // payload` above, exercising `MirExpr::OptionalUnwrap`'s codegen arm
    // dispatching to `Scalar::Float(payload.into_float_value())` (this
    // PR's own fix: previously this arm always built `Scalar::Int(payload.
    // into_int_value())` regardless of inner type, which panics when the
    // extracted field is an `f64` `BasicValueEnum`).
    let mir = MirModule {
        items: vec![
            MirItem::TopLevelStmt(MirStmt::Assign {
                target: "x".to_string(),
                value: MirExpr::OptionalWrap(
                    Box::new(MirExpr::FloatLiteral(5.5)),
                    Box::new(Ty::Float),
                ),
            }),
            MirItem::TopLevelStmt(MirStmt::If {
                test: MirExpr::Compare {
                    op: pycc_mir::CmpOpKind::IsNot,
                    left: Box::new(MirExpr::Name {
                        name: "x".to_string(),
                        ty: optional_float(),
                    }),
                    right: Box::new(MirExpr::NoneLiteral),
                    ty: Ty::Bool,
                },
                body: vec![print_expr(MirExpr::OptionalUnwrap(
                    Box::new(MirExpr::Name {
                        name: "x".to_string(),
                        ty: optional_float(),
                    }),
                    Box::new(Ty::Float),
                ))],
                orelse: vec![],
            }),
        ],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("optional_float_narrowed")
        .expect("failed to create scratch dir");
    let obj_path = dir.join("optional_float_narrowed.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
    let bin_path = dir.join("optional_float_narrowed");
    link_object_with_runtime(&obj_path, &bin_path);
    let output = Command::new(&bin_path).output().expect("binary should run");
    assert_eq!(output.stdout, b"5.5\n");
}

#[test]
fn optional_bool_narrowed_read_of_a_present_value_prints_the_payload() {
    // `x: bool | None = True; if x is not None: print(x)` -- the
    // `Optional[bool]` counterpart of the `Optional[float]` narrowed-read
    // test above, exercising `OptionalUnwrap`'s
    // `Scalar::Bool(payload.into_int_value())` dispatch branch.
    let mir = MirModule {
        items: vec![
            MirItem::TopLevelStmt(MirStmt::Assign {
                target: "x".to_string(),
                value: MirExpr::OptionalWrap(
                    Box::new(MirExpr::BoolLiteral(true)),
                    Box::new(Ty::Bool),
                ),
            }),
            MirItem::TopLevelStmt(MirStmt::If {
                test: MirExpr::Compare {
                    op: pycc_mir::CmpOpKind::IsNot,
                    left: Box::new(MirExpr::Name {
                        name: "x".to_string(),
                        ty: optional_bool(),
                    }),
                    right: Box::new(MirExpr::NoneLiteral),
                    ty: Ty::Bool,
                },
                body: vec![print_expr(MirExpr::OptionalUnwrap(
                    Box::new(MirExpr::Name {
                        name: "x".to_string(),
                        ty: optional_bool(),
                    }),
                    Box::new(Ty::Bool),
                ))],
                orelse: vec![],
            }),
        ],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("optional_bool_narrowed")
        .expect("failed to create scratch dir");
    let obj_path = dir.join("optional_bool_narrowed.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
    let bin_path = dir.join("optional_bool_narrowed");
    link_object_with_runtime(&obj_path, &bin_path);
    let output = Command::new(&bin_path).output().expect("binary should run");
    assert_eq!(output.stdout, b"True\n");
}

#[test]
fn an_optional_bool_function_that_raises_before_returning_still_produces_a_valid_default() {
    // The `Ty::Bool` counterpart of `an_optional_int_function_that_
    // raises_before_returning_still_produces_a_valid_default` above: a
    // function declared to return `Optional[bool]` raises mid-body
    // instead of returning normally, exercising `default_value_for_
    // type`'s `Ty::Optional` arm recursing into its `Ty::Bool` arm
    // (`context.i8_type().const_zero()`) for the exceptional-exit
    // placeholder, rather than the `Ty::Int`-specific
    // `tag_smallint_const` branch.
    let mir = MirModule {
        items: vec![
            MirItem::Function {
                name: "g".to_string(),
                params: vec![("x".to_string(), Ty::Int)],
                return_ty: optional_bool(),
                body: vec![MirStmt::Raise {
                    exception: MirExceptionValue::Constructed {
                        type_tag: 1, // ValueError
                        class_name: "ValueError".to_string(),
                        message: MirExpr::StringLiteral("boom".to_string()),
                    },
                    frame_function: "test_fn".to_string(),
                }],
            },
            MirItem::TopLevelStmt(MirStmt::Try {
                body: vec![MirStmt::Assign {
                    target: "z".to_string(),
                    value: MirExpr::Call {
                        callee: "g".to_string(),
                        args: vec![MirExpr::IntLiteral(1)],
                        ty: optional_bool(),
                    },
                }],
                handlers: vec![MirExceptHandler {
                    exc_type_tag: Some(vec![1]),
                    binding_name: None,
                    binding_ty: None,
                    body: vec![print_expr(MirExpr::IntLiteral(2))],
                }],
                orelse: Vec::new(),
                finalbody: Vec::new(),
            }),
        ],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("optional_bool_exceptional_exit")
        .expect("failed to create scratch dir");
    let obj_path = dir.join("optional_bool_exceptional_exit.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
    let bin_path = dir.join("optional_bool_exceptional_exit");
    link_object_with_runtime(&obj_path, &bin_path);
    let output = Command::new(&bin_path).output().expect("binary should run");
    assert_eq!(output.stdout, b"2\n");
}

#[test]
fn optional_bool_none_placeholder_and_real_absent_value_are_the_same_llvm_struct_type() {
    // #809's risk-log finding: `Optional[bool]`'s real representation is
    // an anonymous LLVM struct `{ i8, i8 }` (`ty_to_basic_type`'s
    // `Ty::Optional` arm, recursing into its own `Ty::Bool` arm for field
    // 0), and `MirExpr::NoneLiteral`'s own placeholder struct
    // (`coerce_scalar_to_type`'s `bare` arm building `{ i8, i8 }` before
    // any target type is known) has the *exact same* field-type list.
    // LLVM literal (non-identified) struct types are uniqued per-`Context`
    // by field-type list, so these two are not merely equal in shape --
    // they are the same `StructType` value. This test pins that finding
    // directly: it proves the collision is real, harmless (both sides are
    // built by `ty_to_basic_type`, and every `coerce_scalar_to_type` call
    // site re-derives the target type independently rather than relying
    // on the LLVM type alone to disambiguate `bool` from any other
    // `{i8,i8}`-shaped payload), and requires no `coerce_scalar_to_type`
    // fix -- discrimination always happens on the requested `Ty`, not on
    // introspecting the `StructValue`'s LLVM type.
    let context = Context::create();
    let bool_optional_struct_ty = ty_to_basic_type(&context, optional_bool()).into_struct_type();
    let none_placeholder_struct_ty =
        context.struct_type(&[context.i8_type().into(), context.i8_type().into()], false);
    assert_eq!(bool_optional_struct_ty, none_placeholder_struct_ty);
}

#[test]
fn optional_bool_absent_value_truthiness_and_narrowed_unwrap_are_both_correct() {
    // The empirical companion to the struct-type-collision test above:
    // proves that despite `Optional[bool]`'s real `{i8,i8}` shape and the
    // bare-`None` placeholder's `{i8,i8}` shape being the literal same
    // `StructType`, an absent `Optional[bool]` value still behaves
    // correctly end to end -- `x: bool | None = None` is falsy (`truthy`
    // reads the present flag, field 1, not distinguishing the collided
    // type), and narrowing with `is not None` correctly does NOT enter the
    // unwrap branch (proving the collision causes no false "present"
    // reading). Prints only `"0\n"`: the `if x:` in the `else` branch, and
    // no output at all from the `is not None` guard's body.
    let mir = MirModule {
        items: vec![
            MirItem::TopLevelStmt(MirStmt::Assign {
                target: "x".to_string(),
                value: MirExpr::OptionalWrap(Box::new(MirExpr::NoneLiteral), Box::new(Ty::Bool)),
            }),
            MirItem::TopLevelStmt(MirStmt::If {
                test: MirExpr::Name {
                    name: "x".to_string(),
                    ty: optional_bool(),
                },
                body: vec![print_expr(MirExpr::IntLiteral(1))],
                orelse: vec![print_expr(MirExpr::IntLiteral(0))],
            }),
            MirItem::TopLevelStmt(MirStmt::If {
                test: MirExpr::Compare {
                    op: pycc_mir::CmpOpKind::IsNot,
                    left: Box::new(MirExpr::Name {
                        name: "x".to_string(),
                        ty: optional_bool(),
                    }),
                    right: Box::new(MirExpr::NoneLiteral),
                    ty: Ty::Bool,
                },
                body: vec![print_expr(MirExpr::OptionalUnwrap(
                    Box::new(MirExpr::Name {
                        name: "x".to_string(),
                        ty: optional_bool(),
                    }),
                    Box::new(Ty::Bool),
                ))],
                orelse: vec![],
            }),
        ],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("optional_bool_absent_end_to_end")
        .expect("failed to create scratch dir");
    let obj_path = dir.join("optional_bool_absent_end_to_end.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
    let bin_path = dir.join("optional_bool_absent_end_to_end");
    link_object_with_runtime(&obj_path, &bin_path);
    let output = Command::new(&bin_path).output().expect("binary should run");
    assert_eq!(output.stdout, b"0\n");
}

#[test]
#[should_panic(
    expected = "internal error: an Optional[int|float|bool] assignment's payload did not evaluate to int, float, or bool"
)]
fn coerce_scalar_to_type_rejects_a_non_int_payload_widening_into_optional_int() {
    // `pycc_hir::func`'s `T0049` gate rejects every `Optional[T]`
    // annotation for `T` outside `{int, float, bool}` before this value
    // could ever be constructed from real source (D-197, #763, Part 1 of
    // #747; widened by #809) -- this pins `coerce_scalar_to_type`'s own
    // defensive backstop directly, via a hand-built `Scalar::Float`
    // targeting `Optional[int]` specifically (not `Optional[float]`,
    // which is a real, accepted widening as of #809): the function's own
    // `(_, scalar) => scalar` catch-all passes it through its
    // `Ty::Int`-targeted recursive call unchanged (there being no `float
    // -> int` widening arm), landing back here as a payload that is
    // `Scalar::Float` while the declared inner type is `Ty::Int` -- a
    // mismatch #809's `(inner.as_ref(), coerced)` match rejects.
    let context = Context::create();
    let builder = context.create_builder();
    coerce_scalar_to_type(
        &context,
        &builder,
        Scalar::Float(context.f64_type().const_float(1.0)),
        pycc_mir::Ty::Optional(Box::new(Ty::Int)),
    );
}

#[test]
#[should_panic(
    expected = "internal error: an Optional[_] placeholder targeted an unsupported inner type"
)]
fn coerce_scalar_to_type_rejects_a_none_placeholder_targeting_an_unsupported_inner_type() {
    // The sibling defensive backstop to the test immediately above, this
    // time for the *placeholder* branch (`v.get_type() != struct_ty`) added
    // by this PR's own fix: `T0049` never admits `Optional[str]`, so a
    // hand-built call reaching this arm directly (bypassing `T0049`
    // entirely, the same way the sibling test above does) proves the
    // `match inner.as_ref() { Ty::Int | Ty::Float | Ty::Bool => .., other
    // => panic!(..) }` catch-all is reachable and fires for any inner type
    // outside the three #809/#763 admit.
    let context = Context::create();
    let builder = context.create_builder();
    let none_placeholder = context
        .struct_type(&[context.i8_type().into(), context.i8_type().into()], false)
        .const_zero();
    coerce_scalar_to_type(
        &context,
        &builder,
        Scalar::Optional(none_placeholder),
        pycc_mir::Ty::Optional(Box::new(Ty::Str)),
    );
}

#[test]
fn coerce_scalar_to_type_builds_an_i8_zero_placeholder_payload_for_a_mismatched_optional_bool_struct()
 {
    // `Ty::Bool`'s real `Optional[bool]` struct shape (`{ i8, i8 }`)
    // happens to be the literal same `StructType` as the bare-`None`
    // placeholder's own fixed `{ i8, i8 }` shape (see
    // `optional_bool_none_placeholder_and_real_absent_value_are_the_same_
    // llvm_struct_type`), so no value ever built by real codegen takes
    // this arm's `else` branch for `inner == Ty::Bool` -- every real
    // `Optional[bool]` value's `v.get_type() == struct_ty` check is
    // always true. `match inner.as_ref() { .. Ty::Bool => .. }`'s own line
    // is therefore otherwise unreachable and untestable through any real
    // MIR program. Pin it directly with a hand-built, deliberately
    // mismatched `Scalar::Optional` (an `{ i64, i8 }`-shaped struct value,
    // never producible for a `Ty::Bool` inner type by real codegen) so
    // this arm still has direct coverage and is proven to build a
    // correctly-typed `i8` zero rather than the sibling `Ty::Int` arm's
    // `i64` constant, mirroring `truthy_of_an_optional_with_a_pointer_
    // payload_panics_as_an_internal_error` above's use of a positioned
    // builder to exercise a codegen function directly.
    let context = Context::create();
    let module = context.create_module("test");
    let builder = context.create_builder();
    let fn_type = context.void_type().fn_type(&[], false);
    let f = module.add_function("f", fn_type, None);
    let block = context.append_basic_block(f, "entry");
    builder.position_at_end(block);
    let mismatched = ty_to_basic_type(&context, pycc_mir::Ty::Optional(Box::new(Ty::Int)))
        .into_struct_type()
        .get_undef();
    let result = coerce_scalar_to_type(
        &context,
        &builder,
        Scalar::Optional(mismatched),
        pycc_mir::Ty::Optional(Box::new(Ty::Bool)),
    );
    let Scalar::Optional(v) = result else {
        panic!("coerce_scalar_to_type should return Scalar::Optional for an Optional target");
    };
    assert_eq!(
        v.get_type(),
        ty_to_basic_type(&context, optional_bool()).into_struct_type()
    );
}

// --- #812 review finding: bare `None` through the argument/return paths
// (`coerce_scalar_to_type`'s `(Ty::Optional(inner), Scalar::Optional(v))`
// arm, not the `OptionalWrap`-mediated assignment path the tests above
// exercise) -----------------------------------------------------------

#[test]
fn optional_float_bare_none_argument_constructs_a_correctly_typed_absent_struct() {
    // `def f(x: float | None): print(x is None); print(x is not None)` ;
    // `f(None)` -- a bare `None` literal argument reaches
    // `build_call_to_with_leading_args`'s own `coerce_scalar_to_type` call
    // directly, with no `MirExpr::OptionalWrap` wrapper in between (only
    // `HirStmt::Assign`/`HirStmt::AnnAssign` construct `OptionalWrap`).
    // Before this PR's fix, the mismatched-struct branch always inserted
    // `tag_smallint_const` (an `i64`) into field 0 regardless of the
    // parameter's declared inner type, producing a malformed
    // `{ f64, i8 }` constant for `Ty::Float` that neither `module.verify()`
    // nor LLVM's own IR printer rejected.
    let mir = MirModule {
        items: vec![
            MirItem::Function {
                name: "f".to_string(),
                params: vec![("x".to_string(), optional_float())],
                return_ty: Ty::None,
                body: vec![
                    print_expr(MirExpr::Compare {
                        op: pycc_mir::CmpOpKind::Is,
                        left: Box::new(MirExpr::Name {
                            name: "x".to_string(),
                            ty: optional_float(),
                        }),
                        right: Box::new(MirExpr::NoneLiteral),
                        ty: Ty::Bool,
                    }),
                    print_expr(MirExpr::Compare {
                        op: pycc_mir::CmpOpKind::IsNot,
                        left: Box::new(MirExpr::Name {
                            name: "x".to_string(),
                            ty: optional_float(),
                        }),
                        right: Box::new(MirExpr::NoneLiteral),
                        ty: Ty::Bool,
                    }),
                ],
            },
            MirItem::TopLevelStmt(MirStmt::ExprStmt(MirExpr::Call {
                callee: "f".to_string(),
                args: vec![MirExpr::NoneLiteral],
                ty: Ty::None,
            })),
        ],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("optional_float_bare_none_argument")
        .expect("failed to create scratch dir");
    let obj_path = dir.join("optional_float_bare_none_argument.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
    let bin_path = dir.join("optional_float_bare_none_argument");
    link_object_with_runtime(&obj_path, &bin_path);
    let output = Command::new(&bin_path).output().expect("binary should run");
    assert_eq!(output.stdout, b"True\nFalse\n");
}

#[test]
fn optional_bool_bare_none_argument_constructs_a_correctly_typed_absent_struct() {
    // The `Optional[bool]` counterpart of the `Optional[float]` argument
    // test immediately above. `Ty::Bool`'s real `{ i8, i8 }` shape happens
    // to collide with the placeholder's own fixed `{ i8, i8 }` (see
    // `optional_bool_none_placeholder_and_real_absent_value_are_the_same_
    // llvm_struct_type`), so this specific inner type was never at risk of
    // the field-0-type-mismatch this PR fixes -- kept for parity with the
    // rest of this file's float/bool pairing and to pin the argument path
    // for both admitted inner types.
    let mir = MirModule {
        items: vec![
            MirItem::Function {
                name: "f".to_string(),
                params: vec![("x".to_string(), optional_bool())],
                return_ty: Ty::None,
                body: vec![
                    print_expr(MirExpr::Compare {
                        op: pycc_mir::CmpOpKind::Is,
                        left: Box::new(MirExpr::Name {
                            name: "x".to_string(),
                            ty: optional_bool(),
                        }),
                        right: Box::new(MirExpr::NoneLiteral),
                        ty: Ty::Bool,
                    }),
                    print_expr(MirExpr::Compare {
                        op: pycc_mir::CmpOpKind::IsNot,
                        left: Box::new(MirExpr::Name {
                            name: "x".to_string(),
                            ty: optional_bool(),
                        }),
                        right: Box::new(MirExpr::NoneLiteral),
                        ty: Ty::Bool,
                    }),
                ],
            },
            MirItem::TopLevelStmt(MirStmt::ExprStmt(MirExpr::Call {
                callee: "f".to_string(),
                args: vec![MirExpr::NoneLiteral],
                ty: Ty::None,
            })),
        ],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("optional_bool_bare_none_argument")
        .expect("failed to create scratch dir");
    let obj_path = dir.join("optional_bool_bare_none_argument.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
    let bin_path = dir.join("optional_bool_bare_none_argument");
    link_object_with_runtime(&obj_path, &bin_path);
    let output = Command::new(&bin_path).output().expect("binary should run");
    assert_eq!(output.stdout, b"True\nFalse\n");
}

#[test]
fn optional_float_bare_none_return_constructs_a_correctly_typed_absent_struct() {
    // `def g() -> float | None: return None` ; `z = g()` ;
    // `print(z is None); print(z is not None)` -- a bare `None` literal
    // `return` reaches `coerce_scalar_to_type` from the function's own
    // return-handling call site, again with no `OptionalWrap` wrapper.
    let mir = MirModule {
        items: vec![
            MirItem::Function {
                name: "g".to_string(),
                params: vec![],
                return_ty: optional_float(),
                body: vec![MirStmt::Return(Some(MirExpr::NoneLiteral))],
            },
            MirItem::TopLevelStmt(MirStmt::Assign {
                target: "z".to_string(),
                value: MirExpr::Call {
                    callee: "g".to_string(),
                    args: vec![],
                    ty: optional_float(),
                },
            }),
            MirItem::TopLevelStmt(print_expr(MirExpr::Compare {
                op: pycc_mir::CmpOpKind::Is,
                left: Box::new(MirExpr::Name {
                    name: "z".to_string(),
                    ty: optional_float(),
                }),
                right: Box::new(MirExpr::NoneLiteral),
                ty: Ty::Bool,
            })),
            MirItem::TopLevelStmt(print_expr(MirExpr::Compare {
                op: pycc_mir::CmpOpKind::IsNot,
                left: Box::new(MirExpr::Name {
                    name: "z".to_string(),
                    ty: optional_float(),
                }),
                right: Box::new(MirExpr::NoneLiteral),
                ty: Ty::Bool,
            })),
        ],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("optional_float_bare_none_return")
        .expect("failed to create scratch dir");
    let obj_path = dir.join("optional_float_bare_none_return.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
    let bin_path = dir.join("optional_float_bare_none_return");
    link_object_with_runtime(&obj_path, &bin_path);
    let output = Command::new(&bin_path).output().expect("binary should run");
    assert_eq!(output.stdout, b"True\nFalse\n");
}
