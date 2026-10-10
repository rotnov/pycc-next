//! Generic function checks and call-site instantiation.

use super::*;

#[test]
fn check_generic_function_accepts_consistent_single_type_param() {
    let param = Ty::Param(Box::new("T".to_string()));
    let func = generic_identity_fn(param.clone(), param);
    assert!(check_generic_function(&func).is_ok());
}

#[test]
fn check_generic_function_rejects_scalar_specific_op_on_bare_param() {
    // `def f[T](x: T) -> T: return x + 1` -- `T` is opaque, so `+` on a
    // bare `Ty::Param` value must fail exactly like any other
    // type-incompatible operand pair, reusing the existing `T0021`
    // numeric-operator diagnostic (`numeric_result_type`'s own
    // "operator ... not defined for ..." arm), not a new code.
    let param = Ty::Param(Box::new("T".to_string()));
    let func = HirItem::Function {
        name: "f".to_string(),
        params: vec![("x".to_string(), param.clone())],
        return_ty: param,
        body: vec![HirStmt::Return(Some(HirExpr::BinOp {
            op: BinOpKind::Add,
            left: Box::new(HirExpr::Name("x".to_string())),
            right: Box::new(HirExpr::IntLiteral(1)),
        }))],
    };
    let err = check_generic_function(&func).unwrap_err();
    assert_eq!(err.code, "T0021");
}

#[test]
fn check_generic_function_rejects_returning_own_param_as_a_concrete_scalar() {
    // `def bad[T](x: T) -> int: return x` -- a generic function returning
    // its *own* `Ty::Param` as a concrete scalar must be rejected with
    // T0022: after monomorphization `T` is substituted with the call-site
    // scalar (e.g. `str`), which would not match the declared `int` return
    // type and panic at codegen. `is_assignable`'s `from == Ty::Param`
    // clause lets this pass structurally (the clause is needed for
    // non-generic functions reading generic class instance attributes),
    // so `check_stmt_in_function`'s `Return` arm narrows it via the
    // threaded `own_type_param` context.
    let param = Ty::Param(Box::new("T".to_string()));
    let func = HirItem::Function {
        name: "bad".to_string(),
        params: vec![("x".to_string(), param.clone())],
        return_ty: Ty::Int,
        body: vec![HirStmt::Return(Some(HirExpr::Name("x".to_string())))],
    };
    let err = check_generic_function(&func).unwrap_err();
    assert_eq!(err.code, "T0022");
}

#[test]
fn check_generic_function_rejects_two_distinct_type_parameters() {
    // Defense in depth (`crates/pycc_hir`'s own frontend arity gate
    // already prevents this from real source, Task 1): a
    // hand-constructed `HirItem` with two distinct `Ty::Param` names
    // across its signature must still be rejected here.
    let func = HirItem::Function {
        name: "f".to_string(),
        params: vec![
            ("x".to_string(), Ty::Param(Box::new("T".to_string()))),
            ("y".to_string(), Ty::Param(Box::new("U".to_string()))),
        ],
        return_ty: Ty::Param(Box::new("T".to_string())),
        body: vec![HirStmt::Return(Some(HirExpr::Name("x".to_string())))],
    };
    let err = check_generic_function(&func).unwrap_err();
    assert_eq!(err.code, "T0042");
}

#[test]
fn check_generic_function_rejects_container_position_type_parameter() {
    // Defense in depth, same rationale as the two-type-parameter test
    // above. `crates/pycc_hir`'s `annotation_to_ty` does lower a
    // `list[T]`-shaped annotation since D-228 (issue #918), but it
    // rejects a `Ty::Param` element there itself, with this same `T0042`
    // code and wording and a real span -- precisely because
    // `substitute_ty` is not recursive. So a container-position type
    // parameter still cannot reach this function from real source, and
    // this can only be exercised via a hand-built `HirItem`.
    let func = HirItem::Function {
        name: "f".to_string(),
        params: vec![(
            "xs".to_string(),
            Ty::List(Box::new(Ty::Param(Box::new("T".to_string())))),
        )],
        return_ty: Ty::None,
        body: vec![HirStmt::Return(None)],
    };
    let err = check_generic_function(&func).unwrap_err();
    assert_eq!(err.code, "T0042");
}

#[test]
fn instantiate_generic_call_substitutes_int_and_mangles_name() {
    let param = Ty::Param(Box::new("T".to_string()));
    let func = generic_identity_fn(param.clone(), param);
    let instantiation = instantiate_generic_call(&func, &[Ty::Int]).unwrap();
    assert_eq!(instantiation.mangled_name, "0gen_identity__T_int");
    assert_eq!(instantiation.return_ty, Ty::Int);
    assert_eq!(
        instantiation.specialized,
        HirItem::Function {
            name: "0gen_identity__T_int".to_string(),
            params: vec![("x".to_string(), Ty::Int)],
            return_ty: Ty::Int,
            body: vec![HirStmt::Return(Some(HirExpr::Name("x".to_string())))],
        }
    );
}

#[test]
fn instantiate_generic_call_substitutes_str_at_a_different_call_site() {
    // Two call sites resolving `T` to different concrete types must
    // produce two independent, differently-mangled specializations
    // (D-134's own monomorphization requirement) -- and the same
    // concrete type from two call sites must produce the identical
    // mangled name and body, which is what Task 3 dedupes on.
    let param = Ty::Param(Box::new("T".to_string()));
    let func = generic_identity_fn(param.clone(), param);
    let str_instantiation = instantiate_generic_call(&func, &[Ty::Str]).unwrap();
    assert_eq!(str_instantiation.mangled_name, "0gen_identity__T_str");
    assert_eq!(str_instantiation.return_ty, Ty::Str);

    let int_instantiation_1 = instantiate_generic_call(&func, &[Ty::Int]).unwrap();
    let int_instantiation_2 = instantiate_generic_call(&func, &[Ty::Int]).unwrap();
    assert_eq!(
        int_instantiation_1.mangled_name,
        int_instantiation_2.mangled_name
    );
    assert_eq!(int_instantiation_1, int_instantiation_2);
}

#[test]
fn instantiate_generic_call_substitutes_nested_ann_assign_annotation() {
    // Exercises the recursive body-substitution walk itself: a nested
    // `AnnAssign` inside an `If` body is the one HIR shape that carries
    // an embedded `Ty` beyond the function's own signature, and must be
    // substituted too, not just `params`/`return_ty`. Compares the whole
    // specialized item by value (rather than destructuring it with a
    // `let-else`/`unreachable!()`, which the coverage gate would
    // otherwise always flag as an untaken branch when this test still
    // lived in `lib.rs`).
    let param = Ty::Param(Box::new("T".to_string()));
    let func = HirItem::Function {
        name: "f".to_string(),
        params: vec![("x".to_string(), param.clone())],
        return_ty: param.clone(),
        body: vec![HirStmt::If {
            test: HirExpr::BoolLiteral(true),
            body: vec![HirStmt::AnnAssign {
                is_final: false,
                target: "y".to_string(),
                annotation: param,
                value: None,
            }],
            orelse: vec![],
        }],
    };
    let instantiation = instantiate_generic_call(&func, &[Ty::Bool]).unwrap();
    assert_eq!(
        instantiation.specialized,
        HirItem::Function {
            name: "0gen_f__T_bool".to_string(),
            params: vec![("x".to_string(), Ty::Bool)],
            return_ty: Ty::Bool,
            body: vec![HirStmt::If {
                test: HirExpr::BoolLiteral(true),
                body: vec![HirStmt::AnnAssign {
                    is_final: false,
                    target: "y".to_string(),
                    annotation: Ty::Bool,
                    value: None,
                }],
                orelse: vec![],
            }],
        }
    );
}

#[test]
fn instantiate_generic_call_substitutes_nested_while_forrange_forlist_bodies() {
    // Exercises the remaining recursive-body-walk arms (`While`,
    // `ForRange`, `ForList`) that the `If` test above doesn't reach --
    // each must recurse into its own nested `body` to find and
    // substitute a further-nested `AnnAssign`.
    let param = Ty::Param(Box::new("T".to_string()));
    let nested = |marker: &str| {
        vec![HirStmt::AnnAssign {
            is_final: false,
            target: marker.to_string(),
            annotation: Ty::Param(Box::new("T".to_string())),
            value: None,
        }]
    };
    let func = HirItem::Function {
        name: "f".to_string(),
        params: vec![("x".to_string(), param.clone())],
        return_ty: param,
        body: vec![
            HirStmt::While {
                test: HirExpr::BoolLiteral(true),
                body: nested("w"),
            },
            HirStmt::ForRange {
                var: "i".to_string(),
                start: HirExpr::IntLiteral(0),
                stop: HirExpr::IntLiteral(1),
                step: HirExpr::IntLiteral(1),
                body: nested("r"),
            },
            HirStmt::ForList {
                var: "e".to_string(),
                list: "xs".to_string(),
                body: nested("l"),
            },
        ],
    };
    let instantiation = instantiate_generic_call(&func, &[Ty::Int]).unwrap();
    let expected_nested = |marker: &str| {
        vec![HirStmt::AnnAssign {
            is_final: false,
            target: marker.to_string(),
            annotation: Ty::Int,
            value: None,
        }]
    };
    assert_eq!(
        instantiation.specialized,
        HirItem::Function {
            name: "0gen_f__T_int".to_string(),
            params: vec![("x".to_string(), Ty::Int)],
            return_ty: Ty::Int,
            body: vec![
                HirStmt::While {
                    test: HirExpr::BoolLiteral(true),
                    body: expected_nested("w"),
                },
                HirStmt::ForRange {
                    var: "i".to_string(),
                    start: HirExpr::IntLiteral(0),
                    stop: HirExpr::IntLiteral(1),
                    step: HirExpr::IntLiteral(1),
                    body: expected_nested("r"),
                },
                HirStmt::ForList {
                    var: "e".to_string(),
                    list: "xs".to_string(),
                    body: expected_nested("l"),
                },
            ],
        }
    );
}

#[test]
#[should_panic(expected = "instantiate_generic_call called with a non-Function HirItem")]
fn instantiate_generic_call_panics_on_a_non_function_item() {
    let _ = instantiate_generic_call(&HirItem::TopLevelStmt(HirStmt::Return(None)), &[]);
}

#[test]
#[should_panic(expected = "check_function called with a non-Function HirItem")]
fn check_generic_function_skips_the_shape_gate_for_a_top_level_statement() {
    // `check_generic_function`'s own shape gate only applies to a
    // `HirItem::Function` -- a `TopLevelStmt` item has no signature to
    // scan for a type parameter at all, so it skips straight to
    // `check_function`, which still panics on a non-`Function` item
    // exactly as it always has (see `check_function_panics_on_a_non_function_item`
    // above): this test's own job is only to prove the `if let` shape
    // gate itself doesn't reject a `TopLevelStmt` some other way first.
    let item = HirItem::TopLevelStmt(HirStmt::ExprStmt(HirExpr::IntLiteral(1)));
    let _ = check_generic_function(&item);
}

#[test]
fn check_generic_function_rejects_type_parameter_nested_in_a_dict_key() {
    // Defense in depth: a hand-built `dict[T, str]`-shaped parameter,
    // covering `scan_signature_ty_for_param`'s `Ty::Dict` arm on its
    // key position (the `?` on the key's own recursive call).
    let func = HirItem::Function {
        name: "f".to_string(),
        params: vec![(
            "d".to_string(),
            Ty::Dict(Box::new((Ty::Param(Box::new("T".to_string())), Ty::Str))),
        )],
        return_ty: Ty::None,
        body: vec![HirStmt::Return(None)],
    };
    let err = check_generic_function(&func).unwrap_err();
    assert_eq!(err.code, "T0042");
}

#[test]
fn check_generic_function_rejects_type_parameter_nested_in_a_dict_value() {
    // Covers the dict value position too (the key succeeds, so this
    // exercises the `Ty::Dict` arm's second recursive call instead of
    // its first).
    let func = HirItem::Function {
        name: "f".to_string(),
        params: vec![(
            "d".to_string(),
            Ty::Dict(Box::new((Ty::Str, Ty::Param(Box::new("T".to_string()))))),
        )],
        return_ty: Ty::None,
        body: vec![HirStmt::Return(None)],
    };
    let err = check_generic_function(&func).unwrap_err();
    assert_eq!(err.code, "T0042");
}

#[test]
fn check_generic_function_rejects_type_parameter_nested_in_a_set() {
    // Defense in depth: covers `scan_signature_ty_for_param`'s
    // `Ty::List(elem) | Ty::Set(elem)` arm's `Set` alternative
    // specifically (the `List` case is already covered by
    // `check_generic_function_rejects_container_position_type_parameter`
    // above).
    let func = HirItem::Function {
        name: "f".to_string(),
        params: vec![(
            "xs".to_string(),
            Ty::Set(Box::new(Ty::Param(Box::new("T".to_string())))),
        )],
        return_ty: Ty::None,
        body: vec![HirStmt::Return(None)],
    };
    let err = check_generic_function(&func).unwrap_err();
    assert_eq!(err.code, "T0042");
}

#[test]
fn check_generic_function_rejects_container_position_type_parameter_in_the_return_type() {
    // Every other container-position test above puts the offending
    // shape in a parameter; `generic_type_param_name` scans
    // `return_ty` too (via its own separate call), which needs its own
    // test to cover that specific call site's error path.
    let func = HirItem::Function {
        name: "f".to_string(),
        params: vec![("n".to_string(), Ty::Int)],
        return_ty: Ty::List(Box::new(Ty::Param(Box::new("T".to_string())))),
        body: vec![HirStmt::Return(None)],
    };
    let err = check_generic_function(&func).unwrap_err();
    assert_eq!(err.code, "T0042");
}

#[test]
fn check_generic_function_rejects_type_parameter_nested_in_a_tuple_element() {
    // Defense in depth: covers `scan_signature_ty_for_param`'s
    // `Ty::Tuple` arm.
    let func = HirItem::Function {
        name: "f".to_string(),
        params: vec![(
            "t".to_string(),
            Ty::Tuple(Box::new(vec![
                Ty::Int,
                Ty::Param(Box::new("T".to_string())),
            ])),
        )],
        return_ty: Ty::None,
        body: vec![HirStmt::Return(None)],
    };
    let err = check_generic_function(&func).unwrap_err();
    assert_eq!(err.code, "T0042");
}

#[test]
fn check_generic_function_accepts_a_non_generic_tuple_typed_parameter() {
    // `check_generic_function` scans every function's signature, not
    // just an actually-generic one -- a plain, fully concrete
    // `Ty::Tuple` parameter with no `Ty::Param` anywhere inside it must
    // scan clean, exercising `scan_signature_ty_for_param`'s `Ty::Tuple`
    // arm's own success path (its `for` loop finishing without any
    // element raising `T0042`).
    let func = HirItem::Function {
        name: "f".to_string(),
        params: vec![("t".to_string(), Ty::Tuple(Box::new(vec![Ty::Int, Ty::Str])))],
        return_ty: Ty::None,
        body: vec![HirStmt::Return(None)],
    };
    assert!(check_generic_function(&func).is_ok());
}

#[test]
fn instantiate_generic_call_leaves_a_concrete_sibling_parameter_unchanged() {
    // A generic function can mix a `T`-typed parameter with an
    // ordinary concrete-typed one (see the assignability-rejection
    // test above for the failure path); when the call site's argument
    // for that concrete parameter is compatible, `substitute_ty` must
    // still run on it and simply clone it through unchanged, since it
    // carries no `Ty::Param` occurrence to substitute.
    let func = HirItem::Function {
        name: "f".to_string(),
        params: vec![
            ("x".to_string(), Ty::Param(Box::new("T".to_string()))),
            ("n".to_string(), Ty::Int),
        ],
        return_ty: Ty::Param(Box::new("T".to_string())),
        body: vec![HirStmt::Return(Some(HirExpr::Name("x".to_string())))],
    };
    let instantiation = instantiate_generic_call(&func, &[Ty::Str, Ty::Int]).unwrap();
    assert_eq!(
        instantiation.specialized,
        HirItem::Function {
            name: "0gen_f__T_str".to_string(),
            params: vec![("x".to_string(), Ty::Str), ("n".to_string(), Ty::Int),],
            return_ty: Ty::Str,
            body: vec![HirStmt::Return(Some(HirExpr::Name("x".to_string())))],
        }
    );
}

#[test]
fn instantiate_generic_call_rejects_inconsistent_call_site_substitution() {
    // `def f[T](x: T, y: T) -> T` called as `f(1, "a")` -- both
    // occurrences of `T` must agree (D-134's own named example).
    let param = Ty::Param(Box::new("T".to_string()));
    let func = HirItem::Function {
        name: "f".to_string(),
        params: vec![
            ("x".to_string(), param.clone()),
            ("y".to_string(), param.clone()),
        ],
        return_ty: param,
        body: vec![HirStmt::Return(Some(HirExpr::Name("x".to_string())))],
    };
    let err = instantiate_generic_call(&func, &[Ty::Int, Ty::Str]).unwrap_err();
    assert_eq!(err.code, "T0042");
}

#[test]
fn instantiate_generic_call_rejects_non_scalar_call_site_argument() {
    // A call site whose argument type used to resolve `T` is not one
    // of the four scalars is D-134's own named rejection case (e.g.
    // passing a `list[int]` value where `T` is inferred).
    let param = Ty::Param(Box::new("T".to_string()));
    let func = generic_identity_fn(param.clone(), param);
    let err = instantiate_generic_call(&func, &[Ty::List(Box::new(Ty::Int))]).unwrap_err();
    assert_eq!(err.code, "T0042");
}

#[test]
fn instantiate_generic_call_rejects_wrong_arity() {
    let param = Ty::Param(Box::new("T".to_string()));
    let func = generic_identity_fn(param.clone(), param);
    let err = instantiate_generic_call(&func, &[Ty::Int, Ty::Int]).unwrap_err();
    assert_eq!(err.code, "T0021");
}

#[test]
fn instantiate_generic_call_rejects_unresolvable_type_parameter() {
    // `T` appears only in the return type, never in any parameter --
    // no call-site argument can resolve it.
    let func = HirItem::Function {
        name: "f".to_string(),
        params: vec![("x".to_string(), Ty::Int)],
        return_ty: Ty::Param(Box::new("T".to_string())),
        body: vec![HirStmt::Return(Some(HirExpr::IntLiteral(0)))],
    };
    let err = instantiate_generic_call(&func, &[Ty::Int]).unwrap_err();
    assert_eq!(err.code, "T0042");
}

#[test]
fn instantiate_generic_call_rejects_non_generic_function() {
    let func = HirItem::Function {
        name: "f".to_string(),
        params: vec![("x".to_string(), Ty::Int)],
        return_ty: Ty::Int,
        body: vec![HirStmt::Return(Some(HirExpr::Name("x".to_string())))],
    };
    let err = instantiate_generic_call(&func, &[Ty::Int]).unwrap_err();
    assert_eq!(err.code, "T0042");
}

#[test]
fn instantiate_generic_call_checks_non_generic_parameter_assignability() {
    // A generic function can mix a `T`-typed parameter with an
    // ordinary concrete-typed one; the concrete one still gets
    // checked for assignability using the existing `is_assignable`
    // (and the existing `T0021` "argument expects" message shape),
    // not silently skipped.
    let func = HirItem::Function {
        name: "f".to_string(),
        params: vec![
            ("x".to_string(), Ty::Param(Box::new("T".to_string()))),
            ("n".to_string(), Ty::Int),
        ],
        return_ty: Ty::Param(Box::new("T".to_string())),
        body: vec![HirStmt::Return(Some(HirExpr::Name("x".to_string())))],
    };
    let err = instantiate_generic_call(&func, &[Ty::Int, Ty::Str]).unwrap_err();
    assert_eq!(err.code, "T0021");
}

#[test]
fn instantiate_generic_call_rejects_two_distinct_type_parameters() {
    // Defense in depth, mirrors `check_generic_function`'s own test.
    let func = HirItem::Function {
        name: "f".to_string(),
        params: vec![
            ("x".to_string(), Ty::Param(Box::new("T".to_string()))),
            ("y".to_string(), Ty::Param(Box::new("U".to_string()))),
        ],
        return_ty: Ty::Param(Box::new("T".to_string())),
        body: vec![HirStmt::Return(Some(HirExpr::Name("x".to_string())))],
    };
    let err = instantiate_generic_call(&func, &[Ty::Int, Ty::Int]).unwrap_err();
    assert_eq!(err.code, "T0042");
}

#[test]
fn instantiate_generic_call_rejects_container_position_type_parameter() {
    // Defense in depth, mirrors `check_generic_function`'s own test.
    let func = HirItem::Function {
        name: "f".to_string(),
        params: vec![(
            "xs".to_string(),
            Ty::List(Box::new(Ty::Param(Box::new("T".to_string())))),
        )],
        return_ty: Ty::None,
        body: vec![HirStmt::Return(None)],
    };
    let err = instantiate_generic_call(&func, &[Ty::List(Box::new(Ty::Int))]).unwrap_err();
    assert_eq!(err.code, "T0042");
}

#[test]
fn check_alone_type_checks_a_module_containing_a_valid_generic_function() {
    let param = Ty::Param(Box::new("T".to_string()));
    let identity = generic_identity_fn(param.clone(), param);
    let top = HirItem::TopLevelStmt(HirStmt::ExprStmt(HirExpr::Call {
        callee: "print".to_string(),
        args: vec![HirExpr::Call {
            callee: "identity".to_string(),
            args: vec![HirExpr::IntLiteral(1)],
        }],
    }));
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![identity, top],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    assert!(check(&hir).is_ok());
}

#[test]
fn check_reports_the_same_generic_diagnostic_as_check_and_resolve() {
    let func = HirItem::Function {
        name: "f".to_string(),
        params: vec![
            ("x".to_string(), Ty::Param(Box::new("T".to_string()))),
            ("y".to_string(), Ty::Param(Box::new("U".to_string()))),
        ],
        return_ty: Ty::Param(Box::new("T".to_string())),
        body: vec![HirStmt::Return(Some(HirExpr::Name("x".to_string())))],
    };
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![func],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    assert_eq!(check(&hir).unwrap_err().code, "T0042");
    assert_eq!(check_and_resolve(&hir).unwrap_err().code, "T0042");
}

#[test]
fn check_and_resolve_rejects_a_generic_call_site_with_the_wrong_arity() {
    // Exercises `infer_expr_in`'s own generic-dispatch arm propagating
    // an `instantiate_generic_call` error (as opposed to
    // `check_generic_function`'s shape gate, covered by the sibling
    // test above) through both public entry points.
    let param = Ty::Param(Box::new("T".to_string()));
    let identity = generic_identity_fn(param.clone(), param);
    let top = HirItem::TopLevelStmt(HirStmt::ExprStmt(HirExpr::Call {
        callee: "identity".to_string(),
        args: vec![HirExpr::IntLiteral(1), HirExpr::IntLiteral(2)],
    }));
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![identity.clone(), top.clone()],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    assert_eq!(check(&hir).unwrap_err().code, "T0021");
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![identity, top],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    assert_eq!(check_and_resolve(&hir).unwrap_err().code, "T0021");
}

#[test]
fn check_and_resolve_rejects_a_type_parameter_nested_in_a_parameter_container() {
    let func = HirItem::Function {
        name: "f".to_string(),
        params: vec![(
            "xs".to_string(),
            Ty::List(Box::new(Ty::Param(Box::new("T".to_string())))),
        )],
        return_ty: Ty::None,
        body: vec![HirStmt::Return(None)],
    };
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![func],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    assert_eq!(check_and_resolve(&hir).unwrap_err().code, "T0042");
}

#[test]
fn ty_contains_param_detects_a_param_nested_in_every_container_shape() {
    let t = Ty::Param(Box::new("T".to_string()));
    assert!(ty_contains_param(&t));
    assert!(ty_contains_param(&Ty::List(Box::new(t.clone()))));
    assert!(ty_contains_param(&Ty::Set(Box::new(t.clone()))));
    assert!(ty_contains_param(&Ty::Dict(Box::new((Ty::Str, t.clone())))));
    assert!(ty_contains_param(&Ty::Dict(Box::new((t.clone(), Ty::Int)))));
    assert!(ty_contains_param(&Ty::Tuple(Box::new(vec![
        Ty::Int,
        t.clone()
    ]))));
    assert!(!ty_contains_param(&Ty::Int));
    assert!(!ty_contains_param(&Ty::Tuple(Box::new(vec![
        Ty::Int,
        Ty::Str
    ]))));
}

#[test]
fn is_generic_signature_detects_a_param_nested_in_a_parameter_container() {
    assert!(is_generic_signature(
        &[(
            "xs".to_string(),
            Ty::List(Box::new(Ty::Param(Box::new("T".to_string()))))
        )],
        &Ty::None,
    ));
    assert!(is_generic_signature(
        &[],
        &Ty::List(Box::new(Ty::Param(Box::new("T".to_string()))))
    ));
    assert!(!is_generic_signature(
        &[("x".to_string(), Ty::Int)],
        &Ty::Int
    ));
}
