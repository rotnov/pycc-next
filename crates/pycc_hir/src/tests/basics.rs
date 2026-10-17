//! Calls, assignments, operators, comparisons, `None` and `Optional` annotations.

use super::*;

#[test]
fn lowers_a_function_definition_without_calling_it() {
    // Defining `main` alone has no observable effect -- matches
    // CPython exactly (confirmed empirically: `python3.14 hello.py`
    // on this exact source prints nothing). Only an explicit call
    // (see the next test) makes it run.
    let module = pycc_parser_test_helper::parse("def main() -> None:\n    print(42)\n");
    let hir = lower_checked(&module).unwrap();
    assert_eq!(
        hir.items,
        vec![HirItem::Function {
            name: "main".to_string(),
            params: vec![],
            return_ty: Ty::None,
            body: vec![HirStmt::ExprStmt(HirExpr::Call {
                callee: "print".to_string(),
                args: vec![HirExpr::IntLiteral(42)],
            })],
        }]
    );
}

#[test]
fn lowers_a_call_to_a_user_defined_function() {
    let module = pycc_parser_test_helper::parse("def main() -> None:\n    print(42)\n\nmain()\n");
    let hir = lower_checked(&module).unwrap();
    assert_eq!(
        hir.items,
        vec![
            HirItem::Function {
                name: "main".to_string(),
                params: vec![],
                return_ty: Ty::None,
                body: vec![HirStmt::ExprStmt(HirExpr::Call {
                    callee: "print".to_string(),
                    args: vec![HirExpr::IntLiteral(42)],
                })],
            },
            HirItem::TopLevelStmt(HirStmt::ExprStmt(HirExpr::Call {
                callee: "main".to_string(),
                args: vec![],
            })),
        ]
    );
}

#[test]
fn lowers_top_level_print_with_no_main() {
    let module = pycc_parser_test_helper::parse("print(42)\n");
    let hir = lower_checked(&module).unwrap();
    assert_eq!(
        hir.items,
        vec![HirItem::TopLevelStmt(HirStmt::ExprStmt(HirExpr::Call {
            callee: "print".to_string(),
            args: vec![HirExpr::IntLiteral(42)],
        }))]
    );
}

#[test]
fn lowers_a_name_reference_used_as_a_call_argument() {
    // Exercises HirExpr::Name specifically -- every other test so far
    // only ever passes an IntLiteral or zero args to a call, never a
    // bare name reference used as a *value* (as opposed to an
    // assignment target, which Task 6 handles separately).
    let module = pycc_parser_test_helper::parse("def f() -> None:\n    print(x)\n");
    let hir = lower_checked(&module).unwrap();
    assert_eq!(
        hir.items,
        vec![HirItem::Function {
            name: "f".to_string(),
            params: vec![],
            return_ty: Ty::None,
            body: vec![HirStmt::ExprStmt(HirExpr::Call {
                callee: "print".to_string(),
                args: vec![HirExpr::Name("x".to_string())],
            })],
        }]
    );
}

#[test]
fn a_bare_boolean_literal_expression_is_now_supported() {
    let module = pycc_parser_test_helper::parse("True\n");
    let hir = lower_checked(&module).unwrap();
    assert_eq!(
        hir.items,
        vec![HirItem::TopLevelStmt(HirStmt::ExprStmt(
            HirExpr::BoolLiteral(true)
        ))]
    );
}

#[test]
fn lowers_an_assignment_and_a_later_reference_to_it() {
    let module = pycc_parser_test_helper::parse("x = 1\nprint(x)\n");
    let hir = lower_checked(&module).unwrap();
    assert_eq!(
        hir.items,
        vec![
            HirItem::TopLevelStmt(HirStmt::Assign {
                target: "x".to_string(),
                value: HirExpr::IntLiteral(1),
            }),
            HirItem::TopLevelStmt(HirStmt::ExprStmt(HirExpr::Call {
                callee: "print".to_string(),
                args: vec![HirExpr::Name("x".to_string())],
            })),
        ]
    );
}

#[test]
fn lowers_a_binary_addition() {
    let module = pycc_parser_test_helper::parse("x = 1 + 2\n");
    let hir = lower_checked(&module).unwrap();
    assert_eq!(
        hir.items,
        vec![HirItem::TopLevelStmt(HirStmt::Assign {
            target: "x".to_string(),
            value: HirExpr::BinOp {
                op: BinOpKind::Add,
                left: Box::new(HirExpr::IntLiteral(1)),
                right: Box::new(HirExpr::IntLiteral(2)),
            },
        })]
    );
}

#[test]
fn lowers_every_arithmetic_operator() {
    let cases = [
        ("x = 1 - 2\n", BinOpKind::Sub),
        ("x = 1 * 2\n", BinOpKind::Mul),
        ("x = 1 / 2\n", BinOpKind::Div),
        ("x = 1 // 2\n", BinOpKind::FloorDiv),
        ("x = 1 % 2\n", BinOpKind::Mod),
        ("x = 1 ** 2\n", BinOpKind::Pow),
    ];
    for (source, expected_op) in cases {
        let module = pycc_parser_test_helper::parse(source);
        let hir = lower_checked(&module).unwrap();
        assert_eq!(
            hir.items,
            vec![HirItem::TopLevelStmt(HirStmt::Assign {
                target: "x".to_string(),
                value: HirExpr::BinOp {
                    op: expected_op,
                    left: Box::new(HirExpr::IntLiteral(1)),
                    right: Box::new(HirExpr::IntLiteral(2)),
                },
            })],
            "wrong lowering for {source:?}"
        );
    }
}

#[test]
fn lowers_a_float_literal() {
    let module = pycc_parser_test_helper::parse("x = 1.5\n");
    let hir = lower_checked(&module).unwrap();
    assert_eq!(
        hir.items,
        vec![HirItem::TopLevelStmt(HirStmt::Assign {
            target: "x".to_string(),
            value: HirExpr::FloatLiteral(1.5),
        })]
    );
}

#[test]
fn a_chained_assignment_of_a_literal_lowers_to_one_assignment_per_target() {
    // #1213: an unobservable right-hand side is copied to each target, so
    // no temporary appears.
    let module = pycc_parser_test_helper::parse("x = y = 1\n");
    let hir = lower_checked(&module).unwrap();
    let assign = |target: &str| {
        HirItem::TopLevelStmt(HirStmt::Assign {
            target: target.to_string(),
            value: HirExpr::IntLiteral(1),
        })
    };
    assert_eq!(hir.items, vec![assign("x"), assign("y")]);
}

#[test]
fn a_chained_assignment_inside_a_function_binds_a_computed_value_once() {
    // #1213: a call is evaluated once into the temporary, then each target
    // is assigned from it, left to right.
    let module = pycc_parser_test_helper::parse(
        "def g() -> int:\n    return 1\ndef f() -> int:\n    a = b = g()\n    return a\n",
    );
    let hir = lower_checked(&module).unwrap();
    let body = hir
        .items
        .iter()
        .find_map(|item| match item {
            HirItem::Function { name, body, .. } if name == "f" => Some(body),
            _ => None,
        })
        .expect("f is lowered");
    let targets: Vec<&str> = body
        .iter()
        .filter_map(|stmt| match stmt {
            HirStmt::Assign { target, .. } => Some(target.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(targets, ["0chain_49", "a", "b"]);
    assert_eq!(
        body[1],
        HirStmt::Assign {
            target: "a".to_string(),
            value: HirExpr::Name("0chain_49".to_string()),
        }
    );
}

#[test]
fn assigning_to_an_attribute_target_lowers_to_attr_set() {
    // D-154 (Part 1 of #375) supersedes this test's own former
    // "`x.attr = 1` is unsupported" invariant: attribute-assignment
    // targets are now structurally recognized (`HirStmt::AttrSet`) for
    // every class method's own `self.<attr> = ...` writes, and -- since
    // this lowering step has no type information, mirroring
    // `HirStmt::DictSet`'s own bare-name-base precedent -- for any other
    // base expression too. `pycc_types` rejects a base that isn't
    // actually a class instance, or an attribute name the base's class
    // never declares.
    let module = pycc_parser_test_helper::parse("x.attr = 1\n");
    let hir = lower_checked(&module).unwrap();
    assert_eq!(
        hir.items,
        vec![HirItem::TopLevelStmt(HirStmt::AttrSet {
            base: HirExpr::Name("x".to_string()),
            attr: "attr".to_string(),
            value: HirExpr::IntLiteral(1),
        })]
    );
}

#[test]
fn assigning_to_an_attribute_target_propagates_an_unsupported_base_expression() {
    // Exercises the `?` inside `Stmt::Assign`'s own `Expr::Attribute`
    // arm's `base` lowering (D-154), mirroring
    // `method_call_propagates_an_unsupported_base_expression` above.
    let module = pycc_parser_test_helper::parse("(1j).attr = 1\n");
    let diagnostic = lower_checked(&module).unwrap_err();

    assert_eq!(diagnostic.code, "C0001");
}

#[test]
fn a_starred_tuple_unpacking_target_is_unsupported() {
    // Part 1 of #891 admits a flat tuple of bare names (see
    // `stmt/unpack/tests.rs`); a starred element is left to a later part.
    assert_capability_error_message(
        "a, *b = 1, 2\n",
        "a starred target (`*rest`) in a tuple-unpacking assignment is not supported yet",
    );
}

#[test]
fn subscript_assignment_to_a_bare_name_base_lowers_to_dict_set() {
    // PR-11 Task 3 (D-123) supersedes D-105's "no subscript assignment
    // target anywhere in this file" invariant this test used to lock in
    // (`list[int]` alone stayed read-only-indexed; `dict[str, int]`
    // ships `d[k] = v`). This lowering step has no type information (the
    // same reason `ForList`'s own bare-name iterable isn't type-checked
    // here either), so `x[0] = 1` lowers to `HirStmt::DictSet`
    // regardless of whether `x` actually turns out to be a `list` or a
    // `dict` -- `pycc_types` now owns rejecting a `list`-typed base with
    // `T0033` (see that crate's own test module), relocating rather than
    // removing the read-only-list invariant.
    let module = pycc_parser_test_helper::parse("x[0] = 1\n");
    let hir = lower_checked(&module).unwrap();
    assert_eq!(
        hir.items,
        vec![HirItem::TopLevelStmt(HirStmt::DictSet {
            dict: "x".to_string(),
            key: HirExpr::IntLiteral(0),
            value: HirExpr::IntLiteral(1),
        })]
    );
}

#[test]
fn subscript_assignment_to_a_non_bare_name_base_is_unsupported() {
    // `f()[0] = 1` has no plain variable name to record as `DictSet`'s
    // own `dict` field -- rejected explicitly rather than guessed at.
    assert_capability_error_message(
        "f()[0] = 1\n",
        "only assigning to a bare-name subscript target (`name[key] = value`) is supported so far",
    );
}

#[test]
fn a_dict_set_target_with_an_unsupported_key_propagates_the_key_error() {
    // (1, 2) no longer fails to lower (this task) -- lambda is still
    // unsupported and exercises the identical propagation path.
    assert_capability_error_message("x[lambda: 1] = 1\n", "expression kind not supported yet");
}

#[test]
fn a_dict_set_target_with_an_unsupported_value_propagates_the_value_error() {
    // (1, 2) no longer fails to lower (this task) -- lambda is still
    // unsupported and exercises the identical propagation path.
    assert_capability_error_message("x[0] = lambda: 1\n", "expression kind not supported yet");
}

#[test]
fn matrix_multiplication_is_unsupported() {
    assert_capability_error_message("x = a @ b\n", "binary operator `@` is not supported yet");
}

#[test]
fn a_with_statement_is_unsupported() {
    // `with` is valid Python but not implemented — it exercises the
    // same catch-all that
    // `unsupported_statement_and_expression_return_spanned_capability_diagnostics`
    // does for expressions. (#435: `pass` was previously used here but
    // is now supported as a no-op for PEP 487 hook bodies.)
    assert_capability_error_message(
        "with open(\"x\") as f:\n    pass\n",
        "statement kind not supported yet",
    );
}

#[test]
fn a_bare_literal_expression_statement_is_now_supported() {
    // `42` alone at module level is legal (if pointless) Python -- an
    // expression statement whose value is simply discarded. The old
    // HIR shape only ever represented a *call* expression statement,
    // so this used to panic; HirExpr::IntLiteral now represents any
    // expression, not just call arguments.
    let module = pycc_parser_test_helper::parse("42\n");
    let hir = lower_checked(&module).unwrap();
    assert_eq!(
        hir.items,
        vec![HirItem::TopLevelStmt(HirStmt::ExprStmt(
            HirExpr::IntLiteral(42)
        ))]
    );
}

#[test]
fn non_name_callee_is_unsupported() {
    // `foo.bar()` no longer reaches this message (Task 7, D-105): a
    // `.` callee is now checked for the `.append()` shape first, and
    // rejected with its own dedicated message when it isn't `.append`
    // (see `calling_a_non_append_method_is_unsupported`). This fixture
    // instead calls an `and`/`or` expression -- neither a bare name, an
    // attribute access nor a call (whose result may be a function value)
    // -- to keep exercising the fallback rejection.
    assert_capability_error_message("(a or b)()\n", "only calling a bare name");
}

#[test]
fn calling_a_zero_arg_function_other_than_print_is_supported() {
    let module = pycc_parser_test_helper::parse("foo()\n");
    let hir = lower_checked(&module).unwrap();
    assert_eq!(
        hir.items,
        vec![HirItem::TopLevelStmt(HirStmt::ExprStmt(HirExpr::Call {
            callee: "foo".to_string(),
            args: vec![],
        }))]
    );
}

#[test]
fn calling_a_non_print_function_with_arguments_is_now_supported() {
    // This used to panic in the pre-Task-5 HIR shape (only zero-arg
    // user-function calls were representable at all) -- HirExpr::Call
    // now carries arbitrary args for every callee, print included;
    // real type-checking of a call's arguments against a declared
    // signature is Task 9's job, not this lowering step's.
    let module = pycc_parser_test_helper::parse("foo(42)\n");
    let hir = lower_checked(&module).unwrap();
    assert_eq!(
        hir.items,
        vec![HirItem::TopLevelStmt(HirStmt::ExprStmt(HirExpr::Call {
            callee: "foo".to_string(),
            args: vec![HirExpr::IntLiteral(42)],
        }))]
    );
}

#[test]
fn print_with_more_than_one_argument_is_now_supported_at_the_hir_level() {
    // Same rationale as above -- HirExpr::Call no longer special-cases
    // `print`'s arity at lowering time.
    let module = pycc_parser_test_helper::parse("print(1, 2)\n");
    let hir = lower_checked(&module).unwrap();
    assert_eq!(
        hir.items,
        vec![HirItem::TopLevelStmt(HirStmt::ExprStmt(HirExpr::Call {
            callee: "print".to_string(),
            args: vec![HirExpr::IntLiteral(1), HirExpr::IntLiteral(2)],
        }))]
    );
}

#[test]
fn print_with_a_float_argument_is_now_supported_at_the_hir_level() {
    // MIR/codegen still only understands an integer-literal argument to
    // `print` (see pycc_mir::lower_instr); this is HIR-only.
    let module = pycc_parser_test_helper::parse("print(2.5)\n");
    let hir = lower_checked(&module).unwrap();
    assert_eq!(
        hir.items,
        vec![HirItem::TopLevelStmt(HirStmt::ExprStmt(HirExpr::Call {
            callee: "print".to_string(),
            args: vec![HirExpr::FloatLiteral(2.5)],
        }))]
    );
}

#[test]
fn print_with_an_integer_too_large_for_i64_lowers() {
    let module = pycc_parser_test_helper::parse("print(99999999999999999999999999999999)\n");
    assert!(lower_checked(&module).is_ok());
}

#[test]
fn a_complex_number_literal_is_unsupported() {
    // Complex isn't in v0.1's type-representation table (int/float/bool/str/None
    // per TYPE_SYSTEM.md) -- unlike float/bool, this isn't deferred to a later
    // PR-4 task, it's simply out of scope for pycc entirely.
    assert_capability_error_message("x = 3j\n", "numeric literal kind not supported yet");
}

#[test]
fn lowers_a_boolean_literal() {
    let module = pycc_parser_test_helper::parse("x = True\n");
    let hir = lower_checked(&module).unwrap();
    assert_eq!(
        hir.items,
        vec![HirItem::TopLevelStmt(HirStmt::Assign {
            target: "x".to_string(),
            value: HirExpr::BoolLiteral(true),
        })]
    );
}

#[test]
fn lowers_a_single_comparison() {
    let module = pycc_parser_test_helper::parse("x = 1 < 2\n");
    let hir = lower_checked(&module).unwrap();
    assert_eq!(
        hir.items,
        vec![HirItem::TopLevelStmt(HirStmt::Assign {
            target: "x".to_string(),
            value: HirExpr::Compare {
                op: CmpOpKind::Lt,
                left: Box::new(HirExpr::IntLiteral(1)),
                right: Box::new(HirExpr::IntLiteral(2)),
            },
        })]
    );
}

#[test]
fn lowers_every_comparison_operator() {
    let cases = [
        ("x = 1 == 2\n", CmpOpKind::Eq),
        ("x = 1 != 2\n", CmpOpKind::NotEq),
        ("x = 1 < 2\n", CmpOpKind::Lt),
        ("x = 1 <= 2\n", CmpOpKind::LtE),
        ("x = 1 > 2\n", CmpOpKind::Gt),
        ("x = 1 >= 2\n", CmpOpKind::GtE),
    ];
    for (source, expected_op) in cases {
        let module = pycc_parser_test_helper::parse(source);
        let hir = lower_checked(&module).unwrap();
        assert_eq!(
            hir.items,
            vec![HirItem::TopLevelStmt(HirStmt::Assign {
                target: "x".to_string(),
                value: HirExpr::Compare {
                    op: expected_op,
                    left: Box::new(HirExpr::IntLiteral(1)),
                    right: Box::new(HirExpr::IntLiteral(2)),
                },
            })],
            "wrong lowering for {source:?}"
        );
    }
}

#[test]
fn a_chained_comparison_with_in_is_not_supported_yet() {
    assert_capability_error_message(
        "x = 1 < 2 in [3]\n",
        "comparison operator not supported yet: In",
    );
}

#[test]
fn an_is_comparison_is_not_supported_yet() {
    assert_capability_error_message("x = 1 is 2\n", "comparison operator not supported yet");
}

#[test]
fn an_is_not_comparison_between_two_non_none_operands_is_not_supported_yet() {
    // Sibling of `an_is_comparison_is_not_supported_yet` for `is not`
    // (D-197, #763, Part 1 of #747): `is_none_operand_shape` is `false`
    // here too, since neither side is `Expr::NoneLiteral`, so this still
    // falls through to the pre-existing `C0001` rejection unchanged.
    assert_capability_error_message("x = 1 is not 2\n", "comparison operator not supported yet");
}

#[test]
fn none_literal_lowers_to_a_bare_hir_none_literal() {
    let module = pycc_parser_test_helper::parse("x = None\n");
    let hir = lower_checked(&module).unwrap();
    assert_eq!(
        hir.items,
        vec![HirItem::TopLevelStmt(HirStmt::Assign {
            target: "x".to_string(),
            value: HirExpr::NoneLiteral,
        })]
    );
}

#[test]
fn x_is_none_lowers_to_a_cmp_is_comparison() {
    // `None` on the right (D-197, #763, Part 1 of #747): exercises
    // `is_none_operand_shape`'s `cmp.comparators[0]` check.
    let module = pycc_parser_test_helper::parse("x = 1\ny = x is None\n");
    let hir = lower_checked(&module).unwrap();
    assert_eq!(
        hir.items[1],
        HirItem::TopLevelStmt(HirStmt::Assign {
            target: "y".to_string(),
            value: HirExpr::Compare {
                op: CmpOpKind::Is,
                left: Box::new(HirExpr::Name("x".to_string())),
                right: Box::new(HirExpr::NoneLiteral),
            },
        })
    );
}

#[test]
fn none_is_not_x_lowers_to_a_cmp_is_not_comparison() {
    // `None` on the left this time (D-197, #763, Part 1 of #747):
    // exercises `is_none_operand_shape`'s `cmp.left` check, the other half
    // of the `||` this test's sibling above does not reach.
    let module = pycc_parser_test_helper::parse("x = 1\ny = None is not x\n");
    let hir = lower_checked(&module).unwrap();
    assert_eq!(
        hir.items[1],
        HirItem::TopLevelStmt(HirStmt::Assign {
            target: "y".to_string(),
            value: HirExpr::Compare {
                op: CmpOpKind::IsNot,
                left: Box::new(HirExpr::NoneLiteral),
                right: Box::new(HirExpr::Name("x".to_string())),
            },
        })
    );
}

#[test]
fn int_or_none_annotation_produces_ty_optional_int() {
    let module = pycc_parser_test_helper::parse("def f(x: int | None) -> None:\n    pass\n");
    let hir = lower_checked(&module).unwrap();
    let HirItem::Function { params, .. } = &hir.items[0] else {
        panic!("expected a function item");
    };
    assert_eq!(params[0].1, Ty::Optional(Box::new(Ty::Int)));
}

#[test]
fn none_or_int_annotation_also_produces_ty_optional_int() {
    // The other operand order (D-197, #763, Part 1 of #747): exercises
    // `annotation_to_ty`'s `(Expr::NoneLiteral(_), other)` match arm rather
    // than its `(other, Expr::NoneLiteral(_))` sibling.
    let module = pycc_parser_test_helper::parse("def f(x: None | int) -> None:\n    pass\n");
    let hir = lower_checked(&module).unwrap();
    let HirItem::Function { params, .. } = &hir.items[0] else {
        panic!("expected a function item");
    };
    assert_eq!(params[0].1, Ty::Optional(Box::new(Ty::Int)));
}

#[test]
fn an_int_or_none_return_annotation_produces_ty_optional_int() {
    let module = pycc_parser_test_helper::parse("def f() -> int | None:\n    return None\n");
    let hir = lower_checked(&module).unwrap();
    let HirItem::Function { return_ty, .. } = &hir.items[0] else {
        panic!("expected a function item");
    };
    assert_eq!(*return_ty, Ty::Optional(Box::new(Ty::Int)));
}

#[test]
fn a_general_union_annotation_with_neither_side_none_produces_t0048() {
    let module = pycc_parser_test_helper::parse("def f(x: int | str) -> None:\n    pass\n");
    let diag = lower_checked(&module).unwrap_err();
    assert_eq!(diag.code, "T0048");
}

#[test]
fn a_three_way_union_chain_also_produces_t0048() {
    // `A | B | None` (D-197, #763, Part 1 of #747): confirms the scope cut
    // is on the syntactic 2-operand shape, not merely on whether `None`
    // appears anywhere in the chain -- `ast::BinOp` associates `|` left,
    // so the outer node's `left` is itself a `BinOp`, never
    // `Expr::NoneLiteral`, landing in the same `_ =>` T0048 arm as
    // `int | str`.
    let module = pycc_parser_test_helper::parse("def f(x: int | str | None) -> None:\n    pass\n");
    let diag = lower_checked(&module).unwrap_err();
    assert_eq!(diag.code, "T0048");
}

#[test]
fn an_optional_of_a_non_int_type_produces_t0049() {
    // A struct tuple of scalars is no pointer, so it cannot be optional;
    // `str | None` and the other pointer types can (strict mode).
    let module = pycc_parser_test_helper::parse("def f(x: tuple[int, int] | None) -> None:\n    pass\n");
    let diag = lower_checked(&module).unwrap_err();
    assert_eq!(diag.code, "T0049");
    let module = pycc_parser_test_helper::parse("def f(x: str | None) -> None:\n    pass\n");
    let hir = lower_checked(&module).unwrap();
    let HirItem::Function { params, .. } = &hir.items[0] else {
        panic!("expected a function item");
    };
    assert_eq!(params[0].1, Ty::Optional(Box::new(Ty::Str)));
}

#[test]
fn a_float_or_none_return_annotation_produces_ty_optional_float() {
    // #809 (Part 3 of #747): widens `T0049` to also admit `T == float`.
    let module = pycc_parser_test_helper::parse("def f() -> float | None:\n    return None\n");
    let hir = lower_checked(&module).unwrap();
    let HirItem::Function { return_ty, .. } = &hir.items[0] else {
        panic!("expected a function item");
    };
    assert_eq!(*return_ty, Ty::Optional(Box::new(Ty::Float)));
}

#[test]
fn a_bool_or_none_return_annotation_produces_ty_optional_bool() {
    // #809 (Part 3 of #747): widens `T0049` to also admit `T == bool`.
    let module = pycc_parser_test_helper::parse("def f() -> bool | None:\n    return None\n");
    let hir = lower_checked(&module).unwrap();
    let HirItem::Function { return_ty, .. } = &hir.items[0] else {
        panic!("expected a function item");
    };
    assert_eq!(*return_ty, Ty::Optional(Box::new(Ty::Bool)));
}

#[test]
fn a_tuple_literal_expression_lowers_successfully() {
    // Tuple literals were this file's own "genuinely unhandled at every
    // level" fixture before this task (list/dict/set literals filled
    // that role earlier and became supported in turn). Now that
    // `Expr::Tuple` has a real arm, this asserts the actual shape
    // rather than a lowering failure -- `pycc_types` (not this crate)
    // now owns which element types/index forms are valid (D-116).
    let module = pycc_parser_test_helper::parse("x = (1, 2)\n");
    let hir = lower_checked(&module).unwrap();
    assert_eq!(
        hir.items[0],
        HirItem::TopLevelStmt(HirStmt::Assign {
            target: "x".to_string(),
            value: HirExpr::TupleLiteral(vec![HirExpr::IntLiteral(1), HirExpr::IntLiteral(2)]),
        })
    );
}
