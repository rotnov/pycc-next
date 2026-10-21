//! Bool-to-int widening and bool identity through calls, slots and dicts.

use super::*;

#[test]
fn compiles_a_none_typed_parameter_and_value_return() {
    // `def f(x: None) -> None: return x` -- `None` returns stay LLVM
    // `void`, while the parameter and name read use the canonical i8
    // unit carrier. This is valid frontend input and must not fail only
    // when codegen declares the function.
    let mir = MirModule {
        items: vec![MirItem::Function {
            name: "f".to_string(),
            params: vec![("x".to_string(), Ty::None)],
            return_ty: Ty::None,
            body: vec![MirStmt::Return(Some(MirExpr::Name {
                name: "x".to_string(),
                ty: Ty::None,
            }))],
        }],
        class_defs: Vec::new(),
    };
    let dir =
        pycc_scratch::ScratchDir::new("none_param_compiles").expect("failed to create scratch dir");
    let obj_path = dir.join("none_param_compiles.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
}

#[test]
fn compiles_a_function_with_a_float_parameter_and_float_return_value() {
    // `def f(x: float) -> float: return x` ; `y = f(1.5)` -- exercises
    // `ty_to_basic_type`'s new `Ty::Float` arm (both the parameter and
    // return-type positions), `build_call_to`'s argument-marshaling
    // match's `Scalar::Float` arm, `emit_expr`'s `Call` arm's
    // `Ty::Float`-result match arm, and `emit_stmt`'s `Return` arm's
    // `Scalar::Float` match arm -- every `float`-typed position Task 5
    // could not support yet.
    let mir = MirModule {
        items: vec![
            MirItem::Function {
                name: "f".to_string(),
                params: vec![("x".to_string(), Ty::Float)],
                return_ty: Ty::Float,
                body: vec![MirStmt::Return(Some(MirExpr::Name {
                    name: "x".to_string(),
                    ty: Ty::Float,
                }))],
            },
            MirItem::TopLevelStmt(MirStmt::Assign {
                target: "y".to_string(),
                value: MirExpr::Call {
                    callee: "f".to_string(),
                    args: vec![MirExpr::FloatLiteral(1.5)],
                    ty: Ty::Float,
                },
            }),
        ],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("float_param_and_return")
        .expect("failed to create scratch dir");
    let obj_path = dir.join("float_param_and_return.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
}

#[test]
fn a_return_inside_a_for_range_body_returns_immediately_without_looping() {
    // `def first_of_range() -> int:\n    for i in range(0, 5, 1):\n
    // return i\n    return -1` ; `print(first_of_range())` -- the
    // trailing `return -1` is unreachable in practice (every
    // legitimate call actually returns from inside the loop on its
    // first iteration) but keeps this hand-built MIR shape well-formed
    // for a non-`None`-returning function (real `pycc_types` would
    // likely reject a bare `for` loop as satisfying T0024's
    // definite-return check on its own, since a `for` loop is never
    // assumed to execute at least once). Proves `ForRange`'s own
    // inline terminator-safety guard (this task's re-add, see the
    // `ForRange` arm's own comment) correctly skips the
    // increment-and-branch-back the moment `body`'s `Return` already
    // terminates `body_bb` -- without it, this would try to build a
    // second terminator onto an already-terminated block, which
    // `module.verify()` would (correctly) reject. Prints "0", not
    // "0\n1\n2\n3\n4\n" (which would mean the loop kept running) or a
    // crash.
    let mir = MirModule {
        items: vec![
            MirItem::Function {
                name: "first_of_range".to_string(),
                params: vec![],
                return_ty: Ty::Int,
                body: vec![
                    MirStmt::ForRange {
                        var: "i".to_string(),
                        start: MirExpr::IntLiteral(0),
                        stop: MirExpr::IntLiteral(5),
                        step: MirExpr::IntLiteral(1),
                        body: vec![MirStmt::Return(Some(MirExpr::Name {
                            name: "i".to_string(),
                            ty: Ty::Int,
                        }))],
                    },
                    MirStmt::Return(Some(MirExpr::IntLiteral(-1))),
                ],
            },
            MirItem::TopLevelStmt(MirStmt::ExprStmt(MirExpr::Call {
                callee: "print".to_string(),
                args: vec![MirExpr::Call {
                    callee: "first_of_range".to_string(),
                    args: vec![],
                    ty: Ty::Int,
                }],
                ty: Ty::None,
            })),
        ],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("for_range_return_inside_body")
        .expect("failed to create scratch dir");
    let obj_path = dir.join("for_range_return_inside_body.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
    let bin_path = dir.join("for_range_return_inside_body");
    link_object_with_runtime(&obj_path, &bin_path);
    let output = Command::new(&bin_path).output().expect("binary should run");
    assert_eq!(pycc_scratch::lf(&output.stdout), b"0\n");
}

#[test]
fn compiles_a_function_call_with_a_bool_argument() {
    // `def identity_bool(b: bool) -> bool: return b` ;
    // `x = identity_bool(True)` -- exercises `build_call_to`'s
    // `Scalar::Bool` argument-marshalling arm (every other
    // function-call test in this file passes only `int` arguments).
    let mir = MirModule {
        items: vec![
            MirItem::Function {
                name: "identity_bool".to_string(),
                params: vec![("b".to_string(), Ty::Bool)],
                return_ty: Ty::Bool,
                body: vec![MirStmt::Return(Some(MirExpr::Name {
                    name: "b".to_string(),
                    ty: Ty::Bool,
                }))],
            },
            MirItem::TopLevelStmt(MirStmt::Assign {
                target: "x".to_string(),
                value: MirExpr::Call {
                    callee: "identity_bool".to_string(),
                    args: vec![MirExpr::BoolLiteral(true)],
                    ty: Ty::Bool,
                },
            }),
        ],
        class_defs: Vec::new(),
    };
    let dir =
        pycc_scratch::ScratchDir::new("call_with_bool_arg").expect("failed to create scratch dir");
    let obj_path = dir.join("call_with_bool_arg.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
}

#[test]
fn a_bool_argument_widens_to_int_when_the_parameter_is_declared_int() {
    // `def f(x: int) -> None: print(x)` ; `f(True)` -- `bool` is an
    // `int` subtype (`pycc_types::is_assignable`), so this is valid,
    // type-checked v0.1 Python. `build_call_to` previously passed the
    // evaluated `Scalar::Bool` (an `i8`) straight through with no
    // widening, so the built call's argument type didn't match `f`'s
    // declared `i64` parameter -- `module.verify()` rejected the IR.
    // `x` is statically `int`-typed, but D-141 preserves and prints the
    // source object's `True` identity.
    let mir = MirModule {
        items: vec![
            MirItem::Function {
                name: "f".to_string(),
                params: vec![("x".to_string(), Ty::Int)],
                return_ty: Ty::None,
                body: vec![MirStmt::ExprStmt(MirExpr::Call {
                    callee: "print".to_string(),
                    args: vec![MirExpr::Name {
                        name: "x".to_string(),
                        ty: Ty::Int,
                    }],
                    ty: Ty::None,
                })],
            },
            MirItem::TopLevelStmt(MirStmt::ExprStmt(MirExpr::Call {
                callee: "f".to_string(),
                args: vec![MirExpr::BoolLiteral(true)],
                ty: Ty::None,
            })),
        ],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("bool_arg_widens_to_int")
        .expect("failed to create scratch dir");
    let obj_path = dir.join("bool_arg_widens_to_int.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
    let bin_path = dir.join("bool_arg_widens_to_int");
    link_object_with_runtime(&obj_path, &bin_path);
    let output = Command::new(&bin_path).output().expect("binary should run");
    assert_eq!(pycc_scratch::lf(&output.stdout), b"True\n");
}

#[test]
fn a_bool_return_value_widens_to_int_when_the_function_declares_int() {
    // `def f() -> int: return True` ; `print(f())` -- same
    // `bool`-is-`int` widening as the argument case above, but for
    // `MirStmt::Return`'s own value-emission arm: it previously mapped
    // the returned `Scalar::Bool` straight to a `BasicValueEnum` with no
    // widening, so the built `ret` instruction's operand type didn't
    // match `f`'s declared `i64` return type -- `module.verify()`
    // rejected the IR. D-141 now preserves and prints `True`.
    let mir = MirModule {
        items: vec![
            MirItem::Function {
                name: "f".to_string(),
                params: vec![],
                return_ty: Ty::Int,
                body: vec![MirStmt::Return(Some(MirExpr::BoolLiteral(true)))],
            },
            MirItem::TopLevelStmt(MirStmt::ExprStmt(MirExpr::Call {
                callee: "print".to_string(),
                args: vec![MirExpr::Call {
                    callee: "f".to_string(),
                    args: vec![],
                    ty: Ty::Int,
                }],
                ty: Ty::None,
            })),
        ],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("bool_return_widens_to_int")
        .expect("failed to create scratch dir");
    let obj_path = dir.join("bool_return_widens_to_int.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
    let bin_path = dir.join("bool_return_widens_to_int");
    link_object_with_runtime(&obj_path, &bin_path);
    let output = Command::new(&bin_path).output().expect("binary should run");
    assert_eq!(pycc_scratch::lf(&output.stdout), b"True\n");
}

#[test]
fn reassigning_an_int_local_with_a_bool_value_widens_it_to_int() {
    // `x = 5; x = True; print(x)` -- `pycc_types::check_assignment`'s
    // sticky-first-type rule (T0023) keeps `x` typed `int` throughout
    // (a later `bool` value is `is_assignable` into it, never rebinding
    // it), so `pycc_mir` reports every `Name("x")` read as `Ty::Int`.
    // `emit_assign` previously reused the first assignment's `i64`
    // alloca but stored the second assignment's raw `Scalar::Bool` (an
    // `i8`) into it verbatim -- an `i8` store into an `i64`-sized slot,
    // followed by an `i64` load expecting a full encoded int word.
    // D-141 stores the `True` marker rather than ordinary integer `1`.
    let mir = MirModule {
        items: vec![
            MirItem::TopLevelStmt(MirStmt::Assign {
                target: "x".to_string(),
                value: MirExpr::IntLiteral(5),
            }),
            MirItem::TopLevelStmt(MirStmt::Assign {
                target: "x".to_string(),
                value: MirExpr::BoolLiteral(true),
            }),
            MirItem::TopLevelStmt(MirStmt::ExprStmt(MirExpr::Call {
                callee: "print".to_string(),
                args: vec![MirExpr::Name {
                    name: "x".to_string(),
                    ty: Ty::Int,
                }],
                ty: Ty::None,
            })),
        ],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("reassign_bool_into_int")
        .expect("failed to create scratch dir");
    let obj_path = dir.join("reassign_bool_into_int.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
    let bin_path = dir.join("reassign_bool_into_int");
    link_object_with_runtime(&obj_path, &bin_path);
    let output = Command::new(&bin_path).output().expect("binary should run");
    assert_eq!(pycc_scratch::lf(&output.stdout), b"True\n");
}

#[test]
fn an_explicit_int_boundary_preserves_bool_identity_in_an_int_slot() {
    let mir = MirModule {
        items: vec![
            MirItem::TopLevelStmt(MirStmt::Assign {
                target: "x".to_string(),
                value: MirExpr::IntBoundary(Box::new(MirExpr::BoolLiteral(true))),
            }),
            MirItem::TopLevelStmt(MirStmt::ExprStmt(MirExpr::Call {
                callee: "print".to_string(),
                args: vec![MirExpr::Name {
                    name: "x".to_string(),
                    ty: Ty::Int,
                }],
                ty: Ty::None,
            })),
        ],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("int_boundary_bool_identity")
        .expect("failed to create scratch dir");
    let obj_path = dir.join("int_boundary_bool_identity.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
    let bin_path = dir.join("int_boundary_bool_identity");
    link_object_with_runtime(&obj_path, &bin_path);
    let output = Command::new(&bin_path).output().expect("binary should run");
    assert_eq!(pycc_scratch::lf(&output.stdout), b"True\n");
}

#[test]
fn bool_identity_survives_nested_int_forwarding_and_fstring_formatting() {
    let mir = MirModule {
        items: vec![
            MirItem::Function {
                name: "source".to_string(),
                params: vec![],
                return_ty: Ty::Int,
                body: vec![MirStmt::Return(Some(MirExpr::BoolLiteral(true)))],
            },
            MirItem::Function {
                name: "forward".to_string(),
                params: vec![("x".to_string(), Ty::Int)],
                return_ty: Ty::Int,
                body: vec![MirStmt::Return(Some(MirExpr::Name {
                    name: "x".to_string(),
                    ty: Ty::Int,
                }))],
            },
            MirItem::TopLevelStmt(MirStmt::ExprStmt(MirExpr::Call {
                callee: "print".to_string(),
                args: vec![MirExpr::FString(vec![
                    MirFStringPart::Literal("value=".to_string()),
                    MirFStringPart::Interpolation(Box::new(MirExpr::Call {
                        callee: "forward".to_string(),
                        args: vec![MirExpr::Call {
                            callee: "source".to_string(),
                            args: vec![],
                            ty: Ty::Int,
                        }],
                        ty: Ty::Int,
                    })),
                ])],
                ty: Ty::None,
            })),
        ],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("bool_identity_nested_fstring")
        .expect("failed to create scratch dir");
    let obj_path = dir.join("bool_identity_nested_fstring.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
    let bin_path = dir.join("bool_identity_nested_fstring");
    link_object_with_runtime(&obj_path, &bin_path);
    let output = Command::new(&bin_path).output().expect("binary should run");
    assert_eq!(pycc_scratch::lf(&output.stdout), b"value=True\n");
}

#[test]
fn a_bool_dict_value_round_trips_with_identity_preserved() {
    let dict_ty = Ty::Dict(Box::new((Ty::Str, Ty::Bool)));
    let mir = MirModule {
        items: vec![
            MirItem::TopLevelStmt(MirStmt::Assign {
                target: "d".to_string(),
                value: MirExpr::DictLiteral(vec![(
                    MirExpr::StringLiteral("k".to_string()),
                    MirExpr::BoolLiteral(true),
                )]),
            }),
            MirItem::TopLevelStmt(MirStmt::ExprStmt(MirExpr::Call {
                callee: "print".to_string(),
                args: vec![MirExpr::DictGet {
                    dict: Box::new(MirExpr::Name {
                        name: "d".to_string(),
                        ty: dict_ty,
                    }),
                    key: Box::new(MirExpr::StringLiteral("k".to_string())),
                }],
                ty: Ty::None,
            })),
        ],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("bool_dict_value_identity")
        .expect("failed to create scratch dir");
    let obj_path = dir.join("bool_dict_value_identity.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
    let bin_path = dir.join("bool_dict_value_identity");
    link_object_with_runtime(&obj_path, &bin_path);
    let output = Command::new(&bin_path).output().expect("binary should run");
    assert_eq!(pycc_scratch::lf(&output.stdout), b"True\n");
}

#[test]
fn a_print_call_used_as_a_value_prints_and_yields_none() {
    // `x = print(1)`: the call prints, and its value is `None`, as in
    // CPython. (A `print` statement goes through `emit_stmt`'s own arm;
    // only a `print` nested in another expression reaches `emit_expr`.)
    let mir = MirModule {
        items: vec![MirItem::TopLevelStmt(MirStmt::Assign {
            target: "x".to_string(),
            value: MirExpr::Call {
                callee: "print".to_string(),
                args: vec![MirExpr::IntLiteral(1)],
                ty: Ty::None,
            },
        })],
        class_defs: Vec::new(),
    };
    let dir = pycc_scratch::ScratchDir::new("print_result_nested")
        .expect("failed to create scratch dir");
    let obj_path = dir.join("print_result_nested.o");
    compile_to_object(&mir, &obj_path, None, false).expect("codegen should succeed");
    let bin_path = dir.join("print_result_nested");
    link_object_with_runtime(&obj_path, &bin_path);
    let output = Command::new(&bin_path).output().expect("binary should run");
    assert_eq!(pycc_scratch::lf(&output.stdout), b"1\n");
}

#[test]
#[should_panic(expected = "has no local slot")]
fn referencing_a_name_with_no_bound_local_is_an_internal_error() {
    // Real `pycc_types` already rejects any reference to an undefined
    // name (T0021) long before codegen runs, so this is hand-built
    // malformed MIR exercising `emit_expr`'s `Name` arm's own
    // defensive backstop directly. This panic's coverage used to come
    // from this file's earlier (Task 3) `referencing_a_function_
    // parameter_is_not_yet_supported` test, which happened to hit it
    // via an *unbound* parameter; now that Task 5 binds parameters for
    // real, that test was rewritten into a positive one (see
    // `a_function_parameter_can_be_reassigned_read_back_and_printed` above),
    // leaving this exact check's own coverage to this more direct,
    // dedicated test instead.
    let mir = MirModule {
        items: vec![MirItem::TopLevelStmt(MirStmt::ExprStmt(MirExpr::Name {
            name: "never_bound".to_string(),
            ty: Ty::Int,
        }))],
        class_defs: Vec::new(),
    };
    let dir =
        pycc_scratch::ScratchDir::new("unbound_name_panics").expect("failed to create scratch dir");
    let obj_path = dir.join("unbound_name_panics.o");
    let _ = compile_to_object(&mir, &obj_path, None, false);
}
