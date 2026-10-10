//! Comprehension lowering.

use super::*;

// -- PR-12 Task 2 (D-117): comprehension frontend HIR forms ----------

#[test]
fn lowers_a_list_comprehension_over_range_to_list_comp_assign() {
    let module = pycc_parser_test_helper::parse("y = [i for i in range(3)]\n");
    let hir = lower_checked(&module).unwrap();
    assert_eq!(
        hir.items,
        vec![HirItem::TopLevelStmt(HirStmt::ListCompAssign {
            target: "y".to_string(),
            var: "0comp_11_i".to_string(),
            iter: CompIter::Range {
                start: HirExpr::IntLiteral(0),
                stop: HirExpr::IntLiteral(3),
                step: HirExpr::IntLiteral(1),
            },
            cond: None,
            elt: Box::new(HirExpr::Name("0comp_11_i".to_string())),
        })]
    );
}

#[test]
fn lowers_a_list_comprehension_with_an_if_filter() {
    let module = pycc_parser_test_helper::parse("y = [i for i in range(5) if i]\n");
    let hir = lower_checked(&module).unwrap();
    assert_eq!(
        hir.items,
        vec![HirItem::TopLevelStmt(HirStmt::ListCompAssign {
            target: "y".to_string(),
            var: "0comp_11_i".to_string(),
            iter: CompIter::Range {
                start: HirExpr::IntLiteral(0),
                stop: HirExpr::IntLiteral(5),
                step: HirExpr::IntLiteral(1),
            },
            cond: Some(Box::new(HirExpr::Name("0comp_11_i".to_string()))),
            elt: Box::new(HirExpr::Name("0comp_11_i".to_string())),
        })]
    );
}

#[test]
fn lowers_a_dict_comprehension_with_an_f_string_key_renaming_the_interpolation() {
    // Confirms `FString`'s own `rename_name_in_expr` arm actually
    // rewrites an interpolated loop-variable reference, not just a bare
    // `Name` expression.
    let module = pycc_parser_test_helper::parse("y = {f\"n{i}\": i for i in range(3)}\n");
    let hir = lower_checked(&module).unwrap();
    assert_eq!(
        hir.items,
        vec![HirItem::TopLevelStmt(HirStmt::DictCompAssign {
            target: "y".to_string(),
            var: "0comp_20_i".to_string(),
            iter: CompIter::Range {
                start: HirExpr::IntLiteral(0),
                stop: HirExpr::IntLiteral(3),
                step: HirExpr::IntLiteral(1),
            },
            cond: None,
            key: Box::new(HirExpr::FString(vec![
                FStringPart::Literal("n".to_string()),
                FStringPart::Interpolation(Box::new(HirExpr::Name("0comp_20_i".to_string()))),
            ])),
            value: Box::new(HirExpr::Name("0comp_20_i".to_string())),
        })]
    );
}

#[test]
fn lowers_a_set_comprehension_to_set_comp_assign() {
    let module = pycc_parser_test_helper::parse("y = {i for i in range(3)}\n");
    let hir = lower_checked(&module).unwrap();
    assert_eq!(
        hir.items,
        vec![HirItem::TopLevelStmt(HirStmt::SetCompAssign {
            target: "y".to_string(),
            var: "0comp_11_i".to_string(),
            iter: CompIter::Range {
                start: HirExpr::IntLiteral(0),
                stop: HirExpr::IntLiteral(3),
                step: HirExpr::IntLiteral(1),
            },
            cond: None,
            elt: Box::new(HirExpr::Name("0comp_11_i".to_string())),
        })]
    );
}

#[test]
fn lowers_a_set_comprehension_with_an_if_filter() {
    // `SetCompAssign`'s own `cond` field needs a dedicated if-filter
    // test distinct from the plain set-comprehension test above: the
    // `cond.map(|c| rename_name_in_expr(...))` closure inside
    // `lower_set_comp` is only reached when `cond.is_some()`.
    let module = pycc_parser_test_helper::parse("y = {i for i in range(5) if i}\n");
    let hir = lower_checked(&module).unwrap();
    assert_eq!(
        hir.items,
        vec![HirItem::TopLevelStmt(HirStmt::SetCompAssign {
            target: "y".to_string(),
            var: "0comp_11_i".to_string(),
            iter: CompIter::Range {
                start: HirExpr::IntLiteral(0),
                stop: HirExpr::IntLiteral(5),
                step: HirExpr::IntLiteral(1),
            },
            cond: Some(Box::new(HirExpr::Name("0comp_11_i".to_string()))),
            elt: Box::new(HirExpr::Name("0comp_11_i".to_string())),
        })]
    );
}

#[test]
fn lowers_a_dict_comprehension_with_an_if_filter() {
    // Same reasoning as `lowers_a_set_comprehension_with_an_if_filter`
    // above, for `lower_dict_comp`'s own `cond.map(...)` closure.
    let module = pycc_parser_test_helper::parse("y = {i: i for i in range(5) if i}\n");
    let hir = lower_checked(&module).unwrap();
    assert_eq!(
        hir.items,
        vec![HirItem::TopLevelStmt(HirStmt::DictCompAssign {
            target: "y".to_string(),
            var: "0comp_14_i".to_string(),
            iter: CompIter::Range {
                start: HirExpr::IntLiteral(0),
                stop: HirExpr::IntLiteral(5),
                step: HirExpr::IntLiteral(1),
            },
            cond: Some(Box::new(HirExpr::Name("0comp_14_i".to_string()))),
            key: Box::new(HirExpr::Name("0comp_14_i".to_string())),
            value: Box::new(HirExpr::Name("0comp_14_i".to_string())),
        })]
    );
}

#[test]
fn a_comprehension_with_two_for_clauses_is_unsupported() {
    assert_capability_error_message(
        "y = [i for i in range(3) for j in range(3)]\n",
        "a comprehension with more than one `for` clause is not supported yet",
    );
}

#[test]
fn a_comprehension_with_two_if_filters_is_unsupported() {
    assert_capability_error_message(
        "y = [i for i in range(5) if i if i]\n",
        "a comprehension with more than one `if` filter is not supported yet",
    );
}

#[test]
fn a_comprehension_with_a_non_bare_name_target_is_unsupported() {
    assert_capability_error_message(
        "y = [a for (a, b) in xs]\n",
        "only a bare name comprehension target is supported so far",
    );
}

#[test]
fn an_async_for_comprehension_is_unsupported() {
    assert_capability_error_message(
        "y = [i async for i in xs]\n",
        "async comprehensions are not supported yet",
    );
}

#[test]
fn an_async_comprehension_in_expression_position_is_unsupported() {
    // #1254 (D-250) lifted D-117's "only `Stmt::Assign`-RHS position"
    // restriction; the expression form shares the header checks, so its
    // refusals match the statement form's.
    assert_capability_error_message(
        "print([i async for i in xs])\n",
        "async comprehensions are not supported yet",
    );
}

#[test]
fn a_comprehension_iterating_a_bare_name_produces_comp_iter_name() {
    let module = pycc_parser_test_helper::parse("xs = [1, 2, 3]\ny = [i for i in xs]\n");
    let hir = lower_checked(&module).unwrap();
    assert_eq!(
        hir.items[1],
        HirItem::TopLevelStmt(HirStmt::ListCompAssign {
            target: "y".to_string(),
            var: "0comp_26_i".to_string(),
            iter: CompIter::Name("xs".to_string()),
            cond: None,
            elt: Box::new(HirExpr::Name("0comp_26_i".to_string())),
        })
    );
}

#[test]
fn a_comprehension_range_iterable_referencing_the_loop_variables_own_source_name_is_not_renamed() {
    // Pins `lower_comprehension_header`'s own documented asymmetry
    // (D-117): `iter` is deliberately never passed through
    // `rename_name_in_expr`, unlike `elt`/`cond`/`key`/`value` -- this
    // is correct CPython scoping, since a comprehension's iterable
    // expression evaluates in the *enclosing* scope, before the
    // comprehension's own loop variable is ever bound.
    // `[i for i in range(i)]`'s `range(i)` must read the *outer* `i`
    // (here, the module-level `i = 5`), not the comprehension's own
    // synthesized loop variable. Without this test, a future change
    // that "fixed" this asymmetry by renaming `iter` too would silently
    // break correct scoping with every other existing test (and 100%
    // coverage) still green, since no other test uses an iterable
    // expression that shares the loop variable's own source name.
    let module = pycc_parser_test_helper::parse("i = 5\ny = [i for i in range(i)]\n");
    let hir = lower_checked(&module).unwrap();
    assert_eq!(
        hir.items[1],
        HirItem::TopLevelStmt(HirStmt::ListCompAssign {
            target: "y".to_string(),
            var: "0comp_17_i".to_string(),
            iter: CompIter::Range {
                start: HirExpr::IntLiteral(0),
                // The un-renamed enclosing-scope `i`, not
                // `HirExpr::Name("0comp_17_i")`.
                stop: HirExpr::Name("i".to_string()),
                step: HirExpr::IntLiteral(1),
            },
            cond: None,
            elt: Box::new(HirExpr::Name("0comp_17_i".to_string())),
        })
    );
}

#[test]
fn a_comprehension_bare_name_iterable_sharing_the_loop_variables_own_source_name_is_not_renamed() {
    // Same reasoning as the `range(i)` test above, for `CompIter::Name`
    // instead of `CompIter::Range`: `[xs for xs in xs]`'s iterable `xs`
    // must resolve to the un-renamed enclosing-scope name, not the
    // comprehension's own synthesized loop variable, even though both
    // are spelled identically in the source.
    let module = pycc_parser_test_helper::parse("xs = [1, 2, 3]\ny = [xs for xs in xs]\n");
    let hir = lower_checked(&module).unwrap();
    assert_eq!(
        hir.items[1],
        HirItem::TopLevelStmt(HirStmt::ListCompAssign {
            target: "y".to_string(),
            var: "0comp_27_xs".to_string(),
            // The un-renamed enclosing-scope `xs`, not
            // `CompIter::Name("0comp_27_xs".to_string())`.
            iter: CompIter::Name("xs".to_string()),
            cond: None,
            elt: Box::new(HirExpr::Name("0comp_27_xs".to_string())),
        })
    );
}

// Part 1 of #1255: an iterable that is neither a bare name nor a
// `range(...)` call lowers to `CompIter::Iterable`; whether it is iterable
// is `pycc_types`' decision (its `object_comprehension` tests pin the
// refusal of every non-object type). The statement form becomes a plain
// `Assign` of the expression form, so none of the native-container
// `*CompAssign` passes sees it.
fn comprehension_assign_value(source: &str) -> HirComprehension {
    let module = pycc_parser_test_helper::parse(source);
    let hir = lower_checked(&module).unwrap();
    let HirItem::TopLevelStmt(HirStmt::Assign { target, value }) = &hir.items[0] else {
        panic!("expected a plain assignment, got {:?}", hir.items[0]);
    };
    assert_eq!(target, "y");
    let HirExpr::Comprehension(comp) = value else {
        panic!("expected a comprehension value, got {value:?}");
    };
    (**comp).clone()
}

#[test]
fn a_comprehension_iterating_a_list_literal_lowers_to_an_iterable_source() {
    let comp = comprehension_assign_value("y = [i for i in [1, 2]]\n");
    assert_eq!(
        comp.iter,
        CompIter::Iterable(Box::new(HirExpr::ListLiteral(vec![
            HirExpr::IntLiteral(1),
            HirExpr::IntLiteral(2),
        ])))
    );
    assert_eq!(comp.elt, CompElt::List(HirExpr::Name(comp.var.clone())));
}

#[test]
fn a_comprehension_iterating_a_method_call_lowers_to_an_iterable_source() {
    let comp = comprehension_assign_value("y = {i for i in o.keys() if i}\n");
    assert_eq!(
        comp.iter,
        CompIter::Iterable(Box::new(HirExpr::MethodCall {
            base: Box::new(HirExpr::Name("o".to_string())),
            method: "keys".to_string(),
            args: vec![],
        }))
    );
    assert_eq!(comp.elt, CompElt::Set(HirExpr::Name(comp.var.clone())));
    assert_eq!(comp.cond, Some(HirExpr::Name(comp.var.clone())));
}

#[test]
fn a_comprehension_iterating_a_non_range_call_lowers_to_an_iterable_source() {
    let comp = comprehension_assign_value("y = [i for i in foo(3)]\n");
    assert_eq!(
        comp.iter,
        CompIter::Iterable(Box::new(HirExpr::Call {
            callee: "foo".to_string(),
            args: vec![HirExpr::IntLiteral(3)],
        }))
    );
}

#[test]
fn a_dict_comprehension_over_an_iterable_also_lowers_to_an_assign() {
    let comp = comprehension_assign_value("y = {i: 1 for i in o.keys()}\n");
    assert!(matches!(comp.iter, CompIter::Iterable(_)));
    assert!(matches!(comp.elt, CompElt::Dict { .. }));
}

#[test]
fn an_iterable_source_keeps_the_loop_variables_source_name() {
    // The outermost iterable evaluates in the enclosing scope, so a read
    // of the loop variable's own source name there is not renamed.
    let comp = comprehension_assign_value("y = [i for i in i.keys()]\n");
    let CompIter::Iterable(iterable) = &comp.iter else {
        panic!("expected an iterable source, got {:?}", comp.iter);
    };
    assert_eq!(
        **iterable,
        HirExpr::MethodCall {
            base: Box::new(HirExpr::Name("i".to_string())),
            method: "keys".to_string(),
            args: vec![],
        }
    );
}

#[test]
fn an_iterable_source_that_fails_to_lower_reports_its_own_error() {
    assert_capability_error_message(
        "y = [i for i in g()()]\n",
        "only calling a bare name is supported so far, got a call whose callee is a call expression",
    );
}

#[test]
fn a_walrus_in_an_iterable_source_is_refused() {
    assert_capability_error_message(
        "y = [i for i in (o := p).keys()]\n",
        "a walrus assignment (`:=`) inside a comprehension is not supported yet",
    );
}

#[test]
fn a_comprehension_range_call_with_keyword_arguments_is_unsupported() {
    assert_capability_error_message(
        "y = [i for i in range(stop=3)]\n",
        "keyword arguments to range() are not supported yet",
    );
}

// The eight tests below each exercise one `?`-propagation region on its
// own `?` operator's specific call site (mirroring this file's existing
// "the five tests below exercise each new arm's own `?`-propagation
// path specifically" precedent above): `lower_set_comp` and
// `lower_dict_comp` are structurally near-identical to
// `lower_list_comp`, but each function's own `?` is a distinct
// coverage region, so an error test against one function's call site
// does not also cover its sibling's.

#[test]
fn a_set_comprehension_with_an_unsupported_header_propagates_the_header_error() {
    // Exercises both `Stmt::Assign`'s own `Expr::SetComp(comp) =>
    // lower_set_comp(...)?` call site and
    // `lower_set_comp`'s own internal
    // `lower_comprehension_header(&comp.generators)?` call site in one
    // test, since the header error propagates through both in the same
    // nested call.
    assert_capability_error_message(
        "y = {i for i in range(3) for j in range(3)}\n",
        "a comprehension with more than one `for` clause is not supported yet",
    );
}

#[test]
fn a_comprehension_iter_range_call_with_too_many_arguments_is_unsupported() {
    // Exercises `lower_comprehension_iter`'s own
    // `lower_range_call(call)?` call site specifically -- distinct from
    // `Stmt::For`'s own separate call site to the same shared helper
    // (already covered by `range_with_too_many_arguments_is_not_supported`
    // above).
    assert_capability_error_message(
        "y = [i for i in range(1, 2, 3, 4)]\n",
        "range() with 4 arguments is not supported",
    );
}

#[test]
fn a_comprehension_if_filter_with_an_unsupported_expression_propagates_the_filter_error() {
    assert_capability_error_message(
        "y = [i for i in range(3) if (lambda: 1)]\n",
        "expression kind not supported yet",
    );
}

#[test]
fn a_list_comprehension_element_that_fails_to_lower_propagates_the_element_error() {
    assert_capability_error_message(
        "y = [(lambda: 1) for i in range(3)]\n",
        "expression kind not supported yet",
    );
}

#[test]
fn a_set_comprehension_element_that_fails_to_lower_propagates_the_element_error() {
    assert_capability_error_message(
        "y = {(lambda: 1) for i in range(3)}\n",
        "expression kind not supported yet",
    );
}

#[test]
fn a_dict_comprehension_with_an_unsupported_header_propagates_the_header_error() {
    assert_capability_error_message(
        "y = {i: i for i in range(3) for j in range(3)}\n",
        "a comprehension with more than one `for` clause is not supported yet",
    );
}

#[test]
fn a_dict_comprehension_key_that_fails_to_lower_propagates_the_key_error() {
    assert_capability_error_message(
        "y = {(lambda: 1): i for i in range(3)}\n",
        "expression kind not supported yet",
    );
}

#[test]
fn a_dict_comprehension_value_that_fails_to_lower_propagates_the_value_error() {
    assert_capability_error_message(
        "y = {i: (lambda: 1) for i in range(3)}\n",
        "expression kind not supported yet",
    );
}

#[test]
fn dict_comp_key_unpacking_parses_successfully_and_is_rejected_at_lowering() {
    // The task-2 brief this test suite followed assumed
    // `{**x for k in y}`-shaped source could never actually parse (so
    // `comp.key == None` would be unreachable, modeled with an internal
    // panic). Verified false directly against the vendored
    // `ruff_python_parser`: it parses this successfully as
    // `ExprDictComp { key: None, value: Name("x"), .. }`, silently
    // dropping the `**` rather than erroring -- so `pycc_parser::parse`
    // itself succeeds here, and `lower_dict_comp` is the one
    // that must reject it, with an ordinary `C0001` capability
    // diagnostic instead of a panic.
    assert!(pycc_parser::parse("y = {**x for k in z}\n").is_ok());
    assert_capability_error_message(
        "y = {**x for k in z}\n",
        "dict-unpacking (`**expr`) inside a dict comprehension is not supported yet",
    );
}

#[test]
fn lower_comprehension_header_rejects_an_empty_generators_slice() {
    // Real parsed source can never produce a comprehension with zero
    // generators -- this is the only way to reach the `[generator] =
    // generators else { ... }` arm's failure branch and its own
    // `generators.first().map(|g| g.range).unwrap_or_default()`
    // span-fallback expression at all (D-014's region coverage gate
    // would otherwise flag that fallback as an uncoverable dead
    // branch).
    let err = lower_comprehension_header(&[], None, &[], &SignatureTable::default()).unwrap_err();
    assert_eq!(err.code, "C0001");
    assert!(
        err.message
            .contains("a comprehension with more than one `for` clause is not supported yet")
    );
    assert_eq!(err.span, Some(Span::new(0, 0)));
}
