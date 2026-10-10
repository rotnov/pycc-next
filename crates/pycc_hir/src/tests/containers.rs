//! Annotated assignments, `Annotated`/`Final`, container literals and container methods.

use super::*;

#[test]
fn lowers_an_annotated_assignment_with_a_value() {
    let module = pycc_parser_test_helper::parse("x: int = 1\nprint(x)\n");
    let hir = lower_checked(&module).unwrap();
    assert_eq!(
        hir.items,
        vec![
            HirItem::TopLevelStmt(HirStmt::AnnAssign {
                is_final: false,
                target: "x".to_string(),
                annotation: Ty::Int,
                value: Some(HirExpr::IntLiteral(1)),
            }),
            HirItem::TopLevelStmt(HirStmt::ExprStmt(HirExpr::Call {
                callee: "print".to_string(),
                args: vec![HirExpr::Name("x".to_string())],
            })),
        ]
    );
}

#[test]
fn lowers_an_annotated_assignment_with_no_value() {
    let module = pycc_parser_test_helper::parse("x: int\n");
    let hir = lower_checked(&module).unwrap();
    assert_eq!(
        hir.items,
        vec![HirItem::TopLevelStmt(HirStmt::AnnAssign {
            is_final: false,
            target: "x".to_string(),
            annotation: Ty::Int,
            value: None,
        })]
    );
}

#[test]
fn rejects_an_annotated_assignment_to_a_non_name_target() {
    // `Stmt::AnnAssign` admits an attribute target only inside a function
    // body, and only as `list[int] = []`/`dict[str, int] = {}` (#1264,
    // `stmt/ann_assign.rs`); at module level `obj.attr: int = 1` is refused
    // with a message naming the admitted form.
    assert_capability_error_message(
        "obj.attr: int = 1\n",
        "an annotated attribute target at module level is not supported yet -- only \
         `<obj>.<attr>: list[int] = []` or `<obj>.<attr>: dict[str, int] = {}`",
    );
}

#[test]
fn rejects_a_parenthesized_annotated_assignment_target_instead_of_erasing_it() {
    // Regression test (advisor-review finding, pre-merge): `(x): int = 1`
    // still lowers `ann.target` to `Expr::Name("x")` -- identical to the
    // unparenthesized `x: int = 1` -- but upstream's own parser sets
    // `simple = false` for it (verified against the pinned
    // ruff_python_parser = "0.0.6" registry source), a real CPython
    // semantic difference this compiler doesn't model. An earlier draft
    // of this arm only matched on `Expr::Name` and ignored `ann.simple`
    // entirely, silently treating the two forms as identical instead of
    // producing the explicit capability diagnostic this file uses for
    // every other unmodeled AST field (see `lower_params`'s own
    // documented self-review finding above).
    assert_capability_error_message(
        "(x): int = 1\n",
        "a parenthesized annotated-assignment target is not supported yet",
    );
}

#[test]
fn an_annotated_assignment_with_an_unsupported_annotation_returns_a_capability_error() {
    // Exercises the `annotation_to_ty(...)?` early-return branch inside
    // the new AnnAssign arm specifically (as opposed to the already
    // covered function-parameter/return-annotation call sites).
    assert_capability_error_message(
        "x: list\n",
        "a bare `list` type annotation is not supported yet",
    );
}

#[test]
fn an_annotated_assignment_with_an_unsupported_value_returns_a_capability_error() {
    // Exercises the `lower_expr(...)?` early-return branch for
    // AnnAssign's value expression specifically.
    // (1, 2) no longer fails to lower (this task) -- lambda is still
    // unsupported and exercises the identical propagation path.
    assert_capability_error_message("x: int = lambda: 1\n", "expression kind not supported yet");
}

// -- Task 7 (D-105): list[int] frontend HIR forms --------------------

#[test]
fn lowers_a_list_literal() {
    // Not a `let PATTERN = ... else { panic!(...) }` destructure -- this
    // file's own coverage-gate lesson (see pycc_ast's documented
    // `re_exported_grammar_types_resolve_and_have_the_expected_shape`
    // finding) is that a hand-written panic arm is never taken on the
    // happy path and shows up as a permanently uncovered region under
    // D-014's 100%-regions gate. A direct `assert_eq!` against the whole
    // expected `HirItem` avoids that without weakening the assertion.
    let module = pycc_parser_test_helper::parse("x = [1, 2, 3]\n");
    let hir = lower_checked(&module).unwrap();
    assert_eq!(
        hir.items[0],
        HirItem::TopLevelStmt(HirStmt::Assign {
            target: "x".to_string(),
            value: HirExpr::ListLiteral(vec![
                HirExpr::IntLiteral(1),
                HirExpr::IntLiteral(2),
                HirExpr::IntLiteral(3),
            ]),
        })
    );
}

#[test]
fn lowers_a_read_subscript() {
    let module = pycc_parser_test_helper::parse("x = [1, 2, 3]\ny = x[0]\n");
    let hir = lower_checked(&module).unwrap();
    assert_eq!(
        hir.items[1],
        HirItem::TopLevelStmt(HirStmt::Assign {
            target: "y".to_string(),
            value: HirExpr::Subscript {
                base: Box::new(HirExpr::Name("x".to_string())),
                index: Box::new(HirExpr::IntLiteral(0)),
            },
        })
    );
}

#[test]
fn lowers_append_as_a_dedicated_hir_node_not_a_generic_call() {
    let module = pycc_parser_test_helper::parse("x = [1]\nx.append(2)\n");
    let hir = lower_checked(&module).unwrap();
    assert_eq!(
        hir.items[1],
        HirItem::TopLevelStmt(HirStmt::ExprStmt(HirExpr::ListAppend {
            list: crate::ContainerReceiver::Name("x".to_string()),
            value: Box::new(HirExpr::IntLiteral(2)),
        }))
    );
}

#[test]
fn list_append_used_as_a_value_lowers_successfully_today() {
    // Real Python's `list.append()` always returns `None`, so using its
    // result as a value (as opposed to a bare `ExprStmt`) is meaningless
    // -- but rejecting that is a type judgment, which is `pycc_types`'
    // job (see `HirExpr::ListAppend`'s own doc comment), not this
    // lowering step's. This test locks in today's actual behavior --
    // `y = x.append(2)` lowers successfully -- so a future change to
    // this arm doesn't silently start rejecting (or accepting some
    // different shape of) value-position `.append()` without its own
    // deliberate decision.
    let module = pycc_parser_test_helper::parse("x = [1]\ny = x.append(2)\n");
    let hir = lower_checked(&module).unwrap();
    assert_eq!(
        hir.items[1],
        HirItem::TopLevelStmt(HirStmt::Assign {
            target: "y".to_string(),
            value: HirExpr::ListAppend {
                list: crate::ContainerReceiver::Name("x".to_string()),
                value: Box::new(HirExpr::IntLiteral(2)),
            },
        })
    );
}

#[test]
fn lowers_for_over_a_list_name_to_for_list() {
    let module = pycc_parser_test_helper::parse("x = [1, 2, 3]\nfor v in x:\n    print(v)\n");
    let hir = lower_checked(&module).unwrap();
    assert_eq!(
        hir.items[1],
        HirItem::TopLevelStmt(HirStmt::ForList {
            var: "v".to_string(),
            list: "x".to_string(),
            body: vec![HirStmt::ExprStmt(HirExpr::Call {
                callee: "print".to_string(),
                args: vec![HirExpr::Name("v".to_string())],
            })],
        })
    );
}

// -- PR-11 Task 3 (D-123): dict[str, int] frontend HIR forms ---------

#[test]
fn lowers_a_dict_literal() {
    let module = pycc_parser_test_helper::parse("x = {\"a\": 1, \"b\": 2}\n");
    let hir = lower_checked(&module).unwrap();
    assert_eq!(
        hir.items[0],
        HirItem::TopLevelStmt(HirStmt::Assign {
            target: "x".to_string(),
            value: HirExpr::DictLiteral(vec![
                (
                    HirExpr::StringLiteral("a".to_string()),
                    HirExpr::IntLiteral(1),
                ),
                (
                    HirExpr::StringLiteral("b".to_string()),
                    HirExpr::IntLiteral(2),
                ),
            ]),
        })
    );
}

#[test]
fn lowers_an_empty_dict_literal() {
    // `{}` is an empty *dict* literal in Python grammar (an empty set has
    // no literal spelling -- `set()` is a call) -- `pycc_types` rejects
    // it (its element types can't be inferred), but lowering itself
    // succeeds, mirroring `HirExpr::ListLiteral(vec![])`'s own split of
    // responsibility.
    let module = pycc_parser_test_helper::parse("x = {}\n");
    let hir = lower_checked(&module).unwrap();
    assert_eq!(
        hir.items,
        vec![HirItem::TopLevelStmt(HirStmt::Assign {
            target: "x".to_string(),
            value: HirExpr::DictLiteral(vec![]),
        })]
    );
}

#[test]
fn dict_unpacking_inside_a_literal_is_unsupported() {
    assert_capability_error_message(
        "x = {**y}\n",
        "dict-unpacking (`**expr`) inside a dict literal is not supported yet",
    );
}

#[test]
fn a_dict_literal_with_an_unsupported_key_propagates_the_key_error() {
    // (1, 2) no longer fails to lower (this task) -- lambda is still
    // unsupported and exercises the identical propagation path.
    assert_capability_error_message(
        "x = {(lambda: 1): 1}\n",
        "expression kind not supported yet",
    );
}

#[test]
fn a_dict_literal_with_an_unsupported_value_propagates_the_value_error() {
    // (1, 2) no longer fails to lower (this task) -- lambda is still
    // unsupported and exercises the identical propagation path.
    assert_capability_error_message(
        "x = {\"a\": (lambda: 1)}\n",
        "expression kind not supported yet",
    );
}

// -- PR-11 Task 7 (D-123): set[int] frontend HIR forms ---------------

#[test]
fn lowers_a_set_literal() {
    let module = pycc_parser_test_helper::parse("x = {1, 2, 3}\n");
    let hir = lower_checked(&module).unwrap();
    assert_eq!(
        hir.items[0],
        HirItem::TopLevelStmt(HirStmt::Assign {
            target: "x".to_string(),
            value: HirExpr::SetLiteral(vec![
                HirExpr::IntLiteral(1),
                HirExpr::IntLiteral(2),
                HirExpr::IntLiteral(3),
            ]),
        })
    );
}

#[test]
fn a_set_literal_with_an_unsupported_element_propagates_the_element_error() {
    // (1, 2) no longer fails to lower (this task) -- lambda is still
    // unsupported and exercises the identical propagation path.
    assert_capability_error_message("x = {(lambda: 1)}\n", "expression kind not supported yet");
}

#[test]
fn subscripted_type_annotation_with_unknown_base_is_rejected() {
    // #435 (Part D): subscripted type annotations (`ClassName[type_arg]`)
    // are now supported for known class names (PEP 560
    // `__class_getitem__`). D-228 (issue #918) additionally lowers the
    // builtin container families -- `frozenset` among them since Part 1 of
    // #1319 -- so this test now pins a name that is neither: `type[int]`,
    // which has no `Ty` variant and so still falls through to the bare-name
    // recursion and its unknown-name message.
    assert_capability_error_message(
        "x: type[int] = int\n",
        "type annotation `type` is not supported yet",
    );
}

#[test]
fn annotated_unwraps_to_the_first_type_argument() {
    // PEP 593 (#383): `Annotated[X, "meta"]` unwraps to `X`, discarding
    // metadata. `Annotated` is recognized as a bare name without
    // requiring `from typing import Annotated`, matching the existing
    // `TypeAlias`/`Any` precedent.
    let module = pycc_parser_test_helper::parse(
        "def f(x: Annotated[int, \"meta\"]) -> int:\n    return x\n",
    );
    let hir = lower_checked(&module).unwrap();
    assert_eq!(
        hir.items,
        vec![HirItem::Function {
            name: "f".to_string(),
            params: vec![("x".to_string(), Ty::Int)],
            return_ty: Ty::Int,
            body: vec![HirStmt::Return(Some(HirExpr::Name("x".to_string())))],
        }]
    );
}

#[test]
fn annotated_with_a_single_argument_is_rejected() {
    // PEP 593 (#383): `Annotated[X]` without metadata is rejected — PEP 593
    // requires at least two arguments (the type and at least one metadata
    // element), matching CPython's own `TypeError`.
    assert_capability_error_message(
        "x: Annotated[str] = \"hello\"\n",
        "Annotated requires at least two arguments: the type and at least one metadata element",
    );
}

#[test]
fn annotated_with_multiple_metadata_args_discards_all_metadata() {
    // PEP 593 (#383): `Annotated[X, 1, "x", 2]` unwraps to `X`, discarding
    // all metadata arguments.
    let module = pycc_parser_test_helper::parse(
        "def f(x: Annotated[int, 1, \"x\", 2]) -> int:\n    return x\n",
    );
    let hir = lower_checked(&module).unwrap();
    assert_eq!(
        hir.items,
        vec![HirItem::Function {
            name: "f".to_string(),
            params: vec![("x".to_string(), Ty::Int)],
            return_ty: Ty::Int,
            body: vec![HirStmt::Return(Some(HirExpr::Name("x".to_string())))],
        }]
    );
}

#[test]
fn annotated_with_an_unsupported_inner_type_still_rejects() {
    // PEP 593 (#383): `Annotated[NonExistent, "meta"]` still rejects via
    // the recursive `annotation_to_ty` call — the unwrap does not bypass
    // type resolution.
    assert_capability_error_message(
        "x: Annotated[NonExistent, \"meta\"] = 1\n",
        "type annotation `NonExistent` is not supported yet",
    );
}

#[test]
fn annotated_with_an_empty_tuple_is_rejected() {
    // PEP 593 (#383): `Annotated[()]` (empty tuple) is rejected —
    // `Annotated` requires at least two arguments (the type and at
    // least one metadata element).
    assert_capability_error_message(
        "x: Annotated[()] = 1\n",
        "Annotated requires at least two arguments: the type and at least one metadata element",
    );
}

#[test]
fn final_unwraps_to_the_inner_type() {
    // PEP 591 (#383): `Final[X]` unwraps to `X`. `Final` is recognized
    // as a bare name without requiring `from typing import Final`,
    // matching the existing `TypeAlias`/`Any` precedent.
    let module = pycc_parser_test_helper::parse("x: Final[int] = 1\n");
    let hir = lower_checked(&module).unwrap();
    assert_eq!(
        hir.items,
        vec![HirItem::TopLevelStmt(HirStmt::AnnAssign {
            target: "x".to_string(),
            annotation: Ty::Int,
            value: Some(HirExpr::IntLiteral(1)),
            is_final: true,
        })]
    );
}

#[test]
fn final_with_str_unwraps_to_str() {
    let module = pycc_parser_test_helper::parse("x: Final[str] = \"hello\"\n");
    let hir = lower_checked(&module).unwrap();
    assert_eq!(
        hir.items,
        vec![HirItem::TopLevelStmt(HirStmt::AnnAssign {
            target: "x".to_string(),
            annotation: Ty::Str,
            value: Some(HirExpr::StringLiteral("hello".to_string())),
            is_final: true,
        })]
    );
}

#[test]
fn final_with_two_type_arguments_is_rejected() {
    // PEP 591 (#383): `Final[X, Y]` is rejected — `Final` takes exactly
    // one type argument.
    assert_capability_error_message(
        "x: Final[int, str] = 1\n",
        "Final takes exactly one type argument",
    );
}

#[test]
fn final_with_a_single_element_tuple_unwraps_to_the_inner_type() {
    // PEP 591 (#383): `Final[(int,)]` (a single-element tuple) unwraps
    // to `int` — the tuple shape with exactly one element is accepted.
    let module = pycc_parser_test_helper::parse("x: Final[(int,)] = 1\n");
    let hir = lower_checked(&module).unwrap();
    assert_eq!(
        hir.items,
        vec![HirItem::TopLevelStmt(HirStmt::AnnAssign {
            target: "x".to_string(),
            annotation: Ty::Int,
            value: Some(HirExpr::IntLiteral(1)),
            is_final: true,
        })]
    );
}

#[test]
fn final_value_less_declaration_sets_is_final() {
    // PEP 591 (#383): a value-less `Final` declaration (`x: Final[int]`
    // with no `= ...`) still sets `is_final` — the name is tracked as
    // non-reassignable even before its initial assignment.
    let module = pycc_parser_test_helper::parse("x: Final[int]\n");
    let hir = lower_checked(&module).unwrap();
    assert_eq!(
        hir.items,
        vec![HirItem::TopLevelStmt(HirStmt::AnnAssign {
            target: "x".to_string(),
            annotation: Ty::Int,
            value: None,
            is_final: true,
        })]
    );
}

#[test]
fn an_empty_list_literal_lowers_to_an_empty_vec() {
    let module = pycc_parser_test_helper::parse("x = []\n");
    let hir = lower_checked(&module).unwrap();
    assert_eq!(
        hir.items,
        vec![HirItem::TopLevelStmt(HirStmt::Assign {
            target: "x".to_string(),
            value: HirExpr::ListLiteral(vec![]),
        })]
    );
}

#[test]
fn appending_to_a_non_bare_name_base_is_unsupported() {
    // `.append()` recognition is restricted to a bare name (D-105) or, since
    // #1263, an instance attribute read; `f().append(1)` has a call as its
    // `attr.value`, so it must be rejected rather than silently accepted.
    assert_capability_error_message(
        "f().append(1)\n",
        "`.append()` is only supported on a name or an instance attribute so far",
    );
}

#[test]
fn append_with_zero_arguments_is_unsupported() {
    assert_capability_error_message(
        "x.append()\n",
        "list.append() takes exactly one argument, got 0",
    );
}

#[test]
fn append_with_two_arguments_is_unsupported() {
    assert_capability_error_message(
        "x.append(1, 2)\n",
        "list.append() takes exactly one argument, got 2",
    );
}

#[test]
fn calling_an_unrecognized_method_lowers_to_a_generic_method_call() {
    // Before D-154 (Part 1 of #375), any `.method()` call other than
    // `.append()`/`.pop()`/`.get()`/`.add()` was rejected right here at
    // HIR-lowering time (D-105, widened by D-119) -- this project had no
    // general method-dispatch shape at all yet. D-154 adds one
    // (`HirExpr::MethodCall`, for instance methods), and -- since this
    // lowering step has no type information to distinguish "receiver is
    // a class instance" from "receiver is anything else" -- every
    // `.method()` call not claimed by a hand-recognized container
    // method or a resolved stdlib symbol call now lowers successfully
    // into that generic shape. `foo`/`x` are never assigned in either
    // snippet below, so real rejection now happens downstream, at
    // `pycc_types` (an unbound-name or non-instance-receiver
    // diagnostic), not here. `.remove()` is kept as the same
    // deliberately-chosen example as before (a real `list` method this
    // compiler doesn't special-case, D-119) to show it now takes the
    // identical generic path as an arbitrary, entirely unrecognized
    // name like `.bar()`.
    let module = pycc_parser_test_helper::parse("foo.bar()\n");
    let hir = lower_checked(&module).unwrap();
    assert_eq!(
        hir.items,
        vec![HirItem::TopLevelStmt(HirStmt::ExprStmt(
            HirExpr::MethodCall {
                base: Box::new(HirExpr::Name("foo".to_string())),
                method: "bar".to_string(),
                args: vec![],
            }
        ))]
    );

    let module = pycc_parser_test_helper::parse("x.remove(1)\n");
    let hir = lower_checked(&module).unwrap();
    assert_eq!(
        hir.items,
        vec![HirItem::TopLevelStmt(HirStmt::ExprStmt(
            HirExpr::MethodCall {
                base: Box::new(HirExpr::Name("x".to_string())),
                method: "remove".to_string(),
                args: vec![HirExpr::IntLiteral(1)],
            }
        ))]
    );
}

// The five tests below exercise each new arm's own `?`-propagation path
// specifically (an inner element/base/index/argument/body expression
// that itself fails to lower), as opposed to every test above, which
// only ever supplies inner expressions that lower successfully.

#[test]
fn a_list_literal_with_an_unsupported_element_propagates_the_element_error() {
    // (1, 2) no longer fails to lower (this task) -- lambda is still
    // unsupported and exercises the identical propagation path.
    assert_capability_error_message("x = [(lambda: 1)]\n", "expression kind not supported yet");
}

#[test]
fn a_subscript_with_an_unsupported_base_propagates_the_base_error() {
    // (1, 2) no longer fails to lower (this task) -- lambda is still
    // unsupported and exercises the identical propagation path.
    assert_capability_error_message("y = (lambda: 1)[0]\n", "expression kind not supported yet");
}

#[test]
fn a_subscript_with_an_unsupported_index_propagates_the_index_error() {
    // (1, 2) no longer fails to lower (this task) -- lambda is still
    // unsupported and exercises the identical propagation path.
    assert_capability_error_message("y = x[lambda: 1]\n", "expression kind not supported yet");
}

#[test]
fn an_append_with_an_unsupported_argument_propagates_the_argument_error() {
    // (1, 2) no longer fails to lower (this task) -- lambda is still
    // unsupported and exercises the identical propagation path.
    assert_capability_error_message("x.append(lambda: 1)\n", "expression kind not supported yet");
}

// -- PR-12 Task 10 (D-119): remaining container methods depth --------
// `list.pop()`, `dict.get(key, default)`, `set.add(value)` -- each
// mirrors `.append()`'s own hand-recognized-special-form shape and test
// coverage exactly (bare-name-base gate, arity gate, value-position
// lowering, argument-propagation).

#[test]
fn lowers_pop_as_a_dedicated_hir_node_not_a_generic_call() {
    let module = pycc_parser_test_helper::parse("x = [1]\nx.pop()\n");
    let hir = lower_checked(&module).unwrap();
    assert_eq!(
        hir.items[1],
        HirItem::TopLevelStmt(HirStmt::ExprStmt(HirExpr::ListPop {
            list: crate::ContainerReceiver::Name("x".to_string()),
        }))
    );
}

#[test]
fn list_pop_used_as_a_value_lowers_successfully() {
    // Unlike `ListAppend`, `.pop()`'s value is the list's element type,
    // not `None` -- `y = x.pop()` is the primary intended use, not a
    // curiosity being merely tolerated.
    let module = pycc_parser_test_helper::parse("x = [1]\ny = x.pop()\n");
    let hir = lower_checked(&module).unwrap();
    assert_eq!(
        hir.items[1],
        HirItem::TopLevelStmt(HirStmt::Assign {
            target: "y".to_string(),
            value: HirExpr::ListPop {
                list: crate::ContainerReceiver::Name("x".to_string()),
            },
        })
    );
}

#[test]
fn popping_from_a_non_bare_name_base_is_unsupported() {
    assert_capability_error_message(
        "f().pop()\n",
        "`.pop()` is only supported on a name or an instance attribute so far",
    );
}

#[test]
fn pop_with_two_arguments_is_unsupported() {
    assert_capability_error_message(
        "x.pop(0, 1, 2)\n",
        "pop() takes at most two arguments, got 3",
    );
}

#[test]
fn lowers_get_as_a_dedicated_hir_node_not_a_generic_call() {
    let module = pycc_parser_test_helper::parse("d = {\"a\": 1}\nd.get(\"a\", 0)\n");
    let hir = lower_checked(&module).unwrap();
    assert_eq!(
        hir.items[1],
        HirItem::TopLevelStmt(HirStmt::ExprStmt(HirExpr::DictGetOrDefault {
            dict: crate::ContainerReceiver::Name("d".to_string()),
            key: Box::new(HirExpr::StringLiteral("a".to_string())),
            default: Box::new(HirExpr::IntLiteral(0)),
        }))
    );
}

#[test]
fn dict_get_used_as_a_value_lowers_successfully() {
    let module = pycc_parser_test_helper::parse("d = {\"a\": 1}\ny = d.get(\"a\", 0)\n");
    let hir = lower_checked(&module).unwrap();
    assert_eq!(
        hir.items[1],
        HirItem::TopLevelStmt(HirStmt::Assign {
            target: "y".to_string(),
            value: HirExpr::DictGetOrDefault {
                dict: crate::ContainerReceiver::Name("d".to_string()),
                key: Box::new(HirExpr::StringLiteral("a".to_string())),
                default: Box::new(HirExpr::IntLiteral(0)),
            },
        })
    );
}

#[test]
fn getting_from_a_non_bare_name_base_is_unsupported() {
    assert_capability_error_message(
        "f().get(\"a\", 0)\n",
        "`.get()` is only supported on a name or an instance attribute so far",
    );
}

#[test]
fn get_with_zero_arguments_is_unsupported() {
    assert_capability_error_message(
        "d.get()\n",
        "`.get()` is only supported as `dict.get(key, default)` with exactly two arguments so far, got 0",
    );
}

#[test]
fn get_with_one_argument_is_unsupported() {
    assert_capability_error_message(
        "d.get(\"a\")\n",
        "`.get()` is only supported as `dict.get(key, default)` with exactly two arguments so far, got 1",
    );
}

#[test]
fn get_with_three_arguments_is_unsupported() {
    assert_capability_error_message(
        "d.get(\"a\", 0, 1)\n",
        "`.get()` is only supported as `dict.get(key, default)` with exactly two arguments so far, got 3",
    );
}

#[test]
fn a_get_call_with_an_unsupported_key_propagates_the_key_error() {
    assert_capability_error_message("d.get(lambda: 1, 0)\n", "expression kind not supported yet");
}

#[test]
fn a_get_call_with_an_unsupported_default_propagates_the_default_error() {
    assert_capability_error_message(
        "d.get(\"a\", lambda: 1)\n",
        "expression kind not supported yet",
    );
}

#[test]
fn lowers_add_as_a_dedicated_hir_node_not_a_generic_call() {
    let module = pycc_parser_test_helper::parse("s = {1}\ns.add(2)\n");
    let hir = lower_checked(&module).unwrap();
    assert_eq!(
        hir.items[1],
        HirItem::TopLevelStmt(HirStmt::ExprStmt(HirExpr::SetAdd {
            set: "s".to_string(),
            value: Box::new(HirExpr::IntLiteral(2)),
        }))
    );
}

#[test]
fn set_add_used_as_a_value_lowers_successfully_today() {
    // Mirrors `ListAppend`'s own "today's actual behavior" test exactly
    // -- `.add()`'s value is always `None`, and D-131 lets an assignment
    // preserve that materialized unit value in ordinary storage.
    let module = pycc_parser_test_helper::parse("s = {1}\ny = s.add(2)\n");
    let hir = lower_checked(&module).unwrap();
    assert_eq!(
        hir.items[1],
        HirItem::TopLevelStmt(HirStmt::Assign {
            target: "y".to_string(),
            value: HirExpr::SetAdd {
                set: "s".to_string(),
                value: Box::new(HirExpr::IntLiteral(2)),
            },
        })
    );
}

#[test]
fn adding_to_a_non_bare_name_base_is_unsupported() {
    assert_capability_error_message(
        "a.b.add(1)\n",
        "`.add()` is only supported on a bare-name set so far",
    );
}

#[test]
fn add_with_zero_arguments_is_unsupported() {
    assert_capability_error_message("s.add()\n", "set.add() takes exactly one argument, got 0");
}

#[test]
fn add_with_two_arguments_is_unsupported() {
    assert_capability_error_message(
        "s.add(1, 2)\n",
        "set.add() takes exactly one argument, got 2",
    );
}

#[test]
fn an_add_with_an_unsupported_argument_propagates_the_argument_error() {
    assert_capability_error_message("s.add(lambda: 1)\n", "expression kind not supported yet");
}

#[test]
fn a_for_list_body_with_an_unsupported_statement_propagates_the_body_error() {
    // (1, 2) no longer fails to lower (this task) -- lambda is still
    // unsupported and exercises the identical propagation path.
    assert_capability_error_message(
        "x = [1, 2, 3]\nfor v in x:\n    lambda: 1\n",
        "expression kind not supported yet",
    );
}

// -- PR-11b Task 2 (D-116): tuple[...] frontend HIR forms -------------

#[test]
fn a_bare_unparenthesized_tuple_expression_lowers_the_same_as_parenthesized() {
    // Python's tuple literal syntax does not require parentheses
    // (`1, 2` and `(1, 2)` parse to the same `Expr::Tuple` node); this
    // locks in that this crate's own lowering treats both identically,
    // since `lower_expr` never inspects `ExprTuple::parenthesized`.
    let module = pycc_parser_test_helper::parse("x = 1, 2\n");
    let hir = lower_checked(&module).unwrap();
    assert_eq!(
        hir.items[0],
        HirItem::TopLevelStmt(HirStmt::Assign {
            target: "x".to_string(),
            value: HirExpr::TupleLiteral(vec![HirExpr::IntLiteral(1), HirExpr::IntLiteral(2)]),
        })
    );
}

#[test]
fn a_heterogeneous_tuple_literal_lowers_with_mixed_element_kinds() {
    // Unlike `ListLiteral`/`SetLiteral`, this crate does not reject
    // mixed element kinds at the HIR layer -- D-116 makes heterogeneity
    // tuple's own defining feature, judged (for element *type*, not
    // syntactic kind) entirely by `pycc_types`, not here.
    let module = pycc_parser_test_helper::parse("x = (1, True, 2.5)\n");
    let hir = lower_checked(&module).unwrap();
    assert_eq!(
        hir.items[0],
        HirItem::TopLevelStmt(HirStmt::Assign {
            target: "x".to_string(),
            value: HirExpr::TupleLiteral(vec![
                HirExpr::IntLiteral(1),
                HirExpr::BoolLiteral(true),
                HirExpr::FloatLiteral(2.5),
            ]),
        })
    );
}

#[test]
fn a_tuple_element_that_fails_to_lower_propagates_its_own_error() {
    assert_capability_error_message("x = (1, lambda: 1)\n", "expression kind not supported yet");
}
