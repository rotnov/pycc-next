//! Walrus targets on the constraint solver path.

use super::*;

#[test]
fn walrus_with_solver_path_succeeds() {
    // PEP 572 (#774): force the solver path with a private helper that
    // contains a walrus in its body, exercising `constraints::
    // collect_expr_constraints`'s own `HirExpr::NamedExpr` arm -- the
    // solver's own private-helper type-inference path never otherwise
    // recurses into that arm anywhere else in this test suite.
    let src = "\
def _helper(x) -> int:
    if (n := 1) > 0:
        pass
    return x
_helper(1)
";
    parse_check_resolve(src).expect("check should succeed");
}

#[test]
fn walrus_target_read_after_an_if_test_on_the_solver_path_succeeds() {
    // PEP 572 (#774), deep-review follow-up (round 4): unlike
    // `walrus_with_solver_path_succeeds` above (which never reads `n` after
    // the `if`, so it exercises `collect_expr_constraints`'s own
    // `NamedExpr` arm for coverage without observing whether the binding
    // it produces is actually usable), this test reads `n` in the `return`
    // that follows -- a walrus in an `if`/`while` test always executes
    // whenever the statement itself is reached, so `n` is definitely bound
    // there, not merely maybe-bound the way a name assigned in only one
    // branch would be. Before `bind_named_expr_targets`, the solver path's
    // `HirExpr::Name` arm found `n` in `local_names` (via `collect_named_
    // expr_names_in_expr`) but never in `env.bindings`, producing a false
    // T0021 "not bound" diagnostic for this legal, T0050-conformant code.
    let src = "\
def _helper(x) -> int:
    if (n := 1) > 0:
        return n
    return x
_helper(1)
";
    parse_check_resolve(src).expect("check should succeed");
}

#[test]
fn walrus_target_read_after_a_while_test_on_the_solver_path_succeeds() {
    // PEP 572 (#774), deep-review follow-up (round 4): the `While` arm's
    // own call to `bind_named_expr_targets` is a separate line from the
    // `If` arm's -- this repository's 100% line-coverage gate (D-014)
    // needs a solver-path (unannotated private helper) `while` test with a
    // walrus to exercise it, since module-scope code (the existing
    // `while (n := 1) > 0` tests elsewhere in this file) never reaches
    // `constraints::collect_block_constraints` at all -- that solver only
    // runs for a private helper whose signature the annotation-driven fast
    // path cannot resolve.
    let src = "\
def _helper(x) -> int:
    while (n := x) > 0:
        return n
    return x
_helper(1)
";
    parse_check_resolve(src).expect("check should succeed");
}

#[test]
fn bare_expr_stmt_walrus_on_the_solver_path_propagates_a_forward_reference_error() {
    // PEP 572 (#774), deep-review follow-up (round 4): the three
    // tests above all place the walrus as an `if`/`while` test, never
    // as a standalone `ExprStmt` -- that leaves `bind_named_expr_
    // targets`'s own call from the `HirStmt::ExprStmt` arm (as opposed
    // to the `If`/`While` arms) unexercised, and none of them exercise
    // its `?`'s error-propagation branch at all (`m` in every prior
    // test's walrus value already has a term by the time it is read,
    // so `collect_expr_constraints` inside `bind_named_expr_targets`
    // never returns `Err` there). `m` here is local to `_helper` (it
    // is assigned two lines below) but not yet bound at the point the
    // walrus reads it, so the recursive `collect_expr_constraints`
    // call inside `bind_named_expr_targets` fails with T0021 before
    // the walrus's own binding step, or the outer `collect_expr_
    // constraints` call on the `ExprStmt` itself, is ever reached.
    let src = "\
def _helper(x) -> int:
    (n := m)
    m = 1
    return x
_helper(1)
";
    let err = parse_check_resolve(src).unwrap_err();
    assert_eq!(err.code, "T0021");
}

#[test]
fn walrus_inside_an_if_test_on_the_solver_path_propagates_a_forward_reference_error() {
    // PEP 572 (#774), deep-review follow-up (round 4): the `If` arm's
    // own error-propagation sibling of the test above -- `m` is local
    // but unbound at the point the walrus inside the `if` test reads
    // it, so `bind_named_expr_targets`'s own recursive call fails
    // before `collect_expr_constraints` is reached on `test` itself.
    let src = "\
def _helper(x) -> int:
    if (n := m) > 0:
        pass
    m = 1
    return x
_helper(1)
";
    let err = parse_check_resolve(src).unwrap_err();
    assert_eq!(err.code, "T0021");
}

#[test]
fn walrus_inside_a_while_test_on_the_solver_path_propagates_a_forward_reference_error() {
    // PEP 572 (#774), deep-review follow-up (round 4): the `While`
    // arm's own error-propagation sibling of the two tests above.
    let src = "\
def _helper(x) -> int:
    while (n := m) > 0:
        pass
    m = 1
    return x
_helper(1)
";
    let err = parse_check_resolve(src).unwrap_err();
    assert_eq!(err.code, "T0021");
}

#[test]
fn walrus_nested_inside_a_unary_op_in_an_if_test_on_the_solver_path_succeeds() {
    // PEP 572 (#774), deep-review follow-up (round 4): every prior
    // `bind_named_expr_targets` test above places the walrus directly
    // as the `Compare`'s own left operand, never inside another
    // recursive shape -- `bind_named_expr_targets`'s `UnaryOp` arm is
    // otherwise unexercised by this whole test suite. `-(n := 1)`
    // nests the walrus inside a `UnaryOp` within the `if` test, and
    // `n` is read afterward to confirm the binding the pre-pass
    // produces is actually usable (not merely exercised for coverage).
    let src = "\
def _helper(x) -> int:
    if -(n := 1) > 0:
        return n
    return x
_helper(1)
";
    parse_check_resolve(src).expect("check should succeed");
}

// PEP 572 (#774), deep-review follow-up (round 4), coverage closure: the
// tests above close every *line* in `bind_named_expr_targets`, but 13 of
// its recursive call sites still show an uncovered `?`-error-propagation
// *region* -- every arm besides the top-level `NamedExpr` arm itself only
// ever originates an `Err` by recursing into a nested `NamedExpr`, and no
// existing test places a forward-reference-erroring walrus in most of
// these nested positions. Each test below goes through
// `collect_block_constraints` directly (this repository's established
// low-level solver-path pattern, e.g.
// `collect_block_constraints_propagates_an_error_from_the_initializer_expression`
// above) rather than `parse_check_resolve`, because several of the target
// HIR shapes (`DictGetOrDefault`, `GenericClassInstantiate`) have no
// parseable Python source that constructs them directly. `bind_named_expr_
// targets` itself has no `pub` qualifier, so these tests cannot call it in
// isolation -- `collect_block_constraints`'s `ExprStmt` arm is the only
// reachable entry point, exactly like every other test in this file.
// `m` is always declared local (so `is_local` is true) but never bound, so
// reading it inside a walrus's own value fails with T0021 (`unbound_local`)
// -- an undeclared/global name would instead resolve silently via the
// `Name` arm's `None => Ok(None)` fallback and never error at all.

#[test]
fn walrus_nested_inside_another_walrus_value_on_the_solver_path_propagates_a_forward_reference_error()
 {
    // Exercises `NamedExpr`'s own recursive `bind_named_expr_targets(value)?`
    // call (source line 974): the outer walrus's value is itself a walrus
    // whose own value forward-references `m`.
    let signatures = HashMap::new();
    let mut parents = Vec::new();
    let mut concrete = Vec::new();
    let mut constraints = SolverConstraints::default();
    let mut env = ConstraintEnvironment::empty(&["n", "m"]);
    let body = vec![HirStmt::ExprStmt(HirExpr::NamedExpr {
        name: "n".to_string(),
        value: Box::new(HirExpr::NamedExpr {
            name: "m".to_string(),
            value: Box::new(HirExpr::Name("m".to_string())),
        }),
    })];

    let err = collect_block_constraints(
        &signatures,
        &mut parents,
        &mut concrete,
        &mut constraints,
        &mut env,
        &body,
        None,
    )
    .unwrap_err();

    assert_eq!(err.code, "T0021");
}

#[test]
fn walrus_nested_inside_a_call_arg_on_the_solver_path_propagates_a_forward_reference_error() {
    // Exercises `Call`'s own `bind_named_expr_targets(arg)?` call inside its
    // argument loop (source line 994).
    let signatures = HashMap::new();
    let mut parents = Vec::new();
    let mut concrete = Vec::new();
    let mut constraints = SolverConstraints::default();
    let mut env = ConstraintEnvironment::empty(&["m"]);
    let body = vec![HirStmt::ExprStmt(HirExpr::Call {
        callee: "f".to_string(),
        args: vec![HirExpr::NamedExpr {
            name: "m".to_string(),
            value: Box::new(HirExpr::Name("m".to_string())),
        }],
    })];

    let err = collect_block_constraints(
        &signatures,
        &mut parents,
        &mut concrete,
        &mut constraints,
        &mut env,
        &body,
        None,
    )
    .unwrap_err();

    assert_eq!(err.code, "T0021");
}

#[test]
fn walrus_nested_inside_an_fstring_interpolation_on_the_solver_path_propagates_a_forward_reference_error()
 {
    // Exercises `FString`'s own `bind_named_expr_targets(inner)?` call
    // inside its interpolation-part loop (source line 1008).
    let signatures = HashMap::new();
    let mut parents = Vec::new();
    let mut concrete = Vec::new();
    let mut constraints = SolverConstraints::default();
    let mut env = ConstraintEnvironment::empty(&["m"]);
    let body = vec![HirStmt::ExprStmt(HirExpr::FString(vec![
        FStringPart::Interpolation(Box::new(HirExpr::NamedExpr {
            name: "m".to_string(),
            value: Box::new(HirExpr::Name("m".to_string())),
        })),
    ]))];

    let err = collect_block_constraints(
        &signatures,
        &mut parents,
        &mut concrete,
        &mut constraints,
        &mut env,
        &body,
        None,
    )
    .unwrap_err();

    assert_eq!(err.code, "T0021");
}

#[test]
fn walrus_nested_inside_a_list_literal_element_on_the_solver_path_propagates_a_forward_reference_error()
 {
    // Exercises the shared `ListLiteral`/`SetLiteral`/`TupleLiteral` arm's
    // own `bind_named_expr_targets(e)?` call inside its element loop
    // (source line 1015).
    let signatures = HashMap::new();
    let mut parents = Vec::new();
    let mut concrete = Vec::new();
    let mut constraints = SolverConstraints::default();
    let mut env = ConstraintEnvironment::empty(&["m"]);
    let body = vec![HirStmt::ExprStmt(HirExpr::ListLiteral(vec![
        HirExpr::NamedExpr {
            name: "m".to_string(),
            value: Box::new(HirExpr::Name("m".to_string())),
        },
    ]))];

    let err = collect_block_constraints(
        &signatures,
        &mut parents,
        &mut concrete,
        &mut constraints,
        &mut env,
        &body,
        None,
    )
    .unwrap_err();

    assert_eq!(err.code, "T0021");
}

#[test]
fn walrus_nested_inside_an_object_list_element_on_the_solver_path_propagates_a_forward_reference_error()
 {
    // Part 2d of #1371: the same shared element-loop arm, entered through
    // its `ObjectList` alternative (an object-slot list display).
    let signatures = HashMap::new();
    let mut parents = Vec::new();
    let mut concrete = Vec::new();
    let mut constraints = SolverConstraints::default();
    let mut env = ConstraintEnvironment::empty(&["m"]);
    let body = vec![HirStmt::ExprStmt(HirExpr::ObjectList(vec![
        HirExpr::NamedExpr {
            name: "m".to_string(),
            value: Box::new(HirExpr::Name("m".to_string())),
        },
    ]))];

    let err = collect_block_constraints(
        &signatures,
        &mut parents,
        &mut concrete,
        &mut constraints,
        &mut env,
        &body,
        None,
    )
    .unwrap_err();

    assert_eq!(err.code, "T0021");
}

#[test]
fn walrus_nested_inside_a_subscript_base_on_the_solver_path_propagates_a_forward_reference_error() {
    // Exercises `Subscript`'s own `bind_named_expr_targets(base)?` call
    // (source line 1020). The index is a harmless literal that is never
    // reached because the base's error propagates first.
    let signatures = HashMap::new();
    let mut parents = Vec::new();
    let mut concrete = Vec::new();
    let mut constraints = SolverConstraints::default();
    let mut env = ConstraintEnvironment::empty(&["m"]);
    let body = vec![HirStmt::ExprStmt(HirExpr::Subscript {
        base: Box::new(HirExpr::NamedExpr {
            name: "m".to_string(),
            value: Box::new(HirExpr::Name("m".to_string())),
        }),
        index: Box::new(HirExpr::IntLiteral(0)),
    })];

    let err = collect_block_constraints(
        &signatures,
        &mut parents,
        &mut concrete,
        &mut constraints,
        &mut env,
        &body,
        None,
    )
    .unwrap_err();

    assert_eq!(err.code, "T0021");
}

#[test]
fn walrus_nested_inside_a_slice_base_on_the_solver_path_propagates_a_forward_reference_error() {
    // Exercises `Slice`'s own `bind_named_expr_targets(base)?` call (source
    // line 1029).
    let signatures = HashMap::new();
    let mut parents = Vec::new();
    let mut concrete = Vec::new();
    let mut constraints = SolverConstraints::default();
    let mut env = ConstraintEnvironment::empty(&["m"]);
    let body = vec![HirStmt::ExprStmt(HirExpr::Slice {
        base: Box::new(HirExpr::NamedExpr {
            name: "m".to_string(),
            value: Box::new(HirExpr::Name("m".to_string())),
        }),
        start: None,
        stop: None,
        step: None,
    })];

    let err = collect_block_constraints(
        &signatures,
        &mut parents,
        &mut concrete,
        &mut constraints,
        &mut env,
        &body,
        None,
    )
    .unwrap_err();

    assert_eq!(err.code, "T0021");
}

#[test]
fn walrus_nested_inside_a_slice_bound_on_the_solver_path_propagates_a_forward_reference_error() {
    // Exercises `Slice`'s own `bind_named_expr_targets(bound)?` call inside
    // its `[start, stop, step].into_iter().flatten()` loop (source line
    // 1031). The base is a harmless bare name -- `bind_named_expr_targets`'s
    // `Name` arm never looks the name up, so it always succeeds regardless
    // of binding state -- so the base's own recursion (line 1029) succeeds
    // and the loop is reached.
    let signatures = HashMap::new();
    let mut parents = Vec::new();
    let mut concrete = Vec::new();
    let mut constraints = SolverConstraints::default();
    let mut env = ConstraintEnvironment::empty(&["m"]);
    let body = vec![HirStmt::ExprStmt(HirExpr::Slice {
        base: Box::new(HirExpr::Name("xs".to_string())),
        start: Some(Box::new(HirExpr::NamedExpr {
            name: "m".to_string(),
            value: Box::new(HirExpr::Name("m".to_string())),
        })),
        stop: None,
        step: None,
    })];

    let err = collect_block_constraints(
        &signatures,
        &mut parents,
        &mut concrete,
        &mut constraints,
        &mut env,
        &body,
        None,
    )
    .unwrap_err();

    assert_eq!(err.code, "T0021");
}

#[test]
fn walrus_nested_inside_a_dict_literal_key_on_the_solver_path_propagates_a_forward_reference_error()
{
    // Exercises `DictLiteral`'s own `bind_named_expr_targets(k)?` call
    // (source line 1040).
    let signatures = HashMap::new();
    let mut parents = Vec::new();
    let mut concrete = Vec::new();
    let mut constraints = SolverConstraints::default();
    let mut env = ConstraintEnvironment::empty(&["m"]);
    let body = vec![HirStmt::ExprStmt(HirExpr::DictLiteral(vec![(
        HirExpr::NamedExpr {
            name: "m".to_string(),
            value: Box::new(HirExpr::Name("m".to_string())),
        },
        HirExpr::Name("v".to_string()),
    )]))];

    let err = collect_block_constraints(
        &signatures,
        &mut parents,
        &mut concrete,
        &mut constraints,
        &mut env,
        &body,
        None,
    )
    .unwrap_err();

    assert_eq!(err.code, "T0021");
}

#[test]
fn walrus_nested_inside_a_dict_literal_value_on_the_solver_path_propagates_a_forward_reference_error()
 {
    // Exercises `DictLiteral`'s own `bind_named_expr_targets(v)?` call
    // (source line 1041). The key is a harmless bare name so the key's own
    // recursion (line 1040) succeeds and the value is reached.
    let signatures = HashMap::new();
    let mut parents = Vec::new();
    let mut concrete = Vec::new();
    let mut constraints = SolverConstraints::default();
    let mut env = ConstraintEnvironment::empty(&["m"]);
    let body = vec![HirStmt::ExprStmt(HirExpr::DictLiteral(vec![(
        HirExpr::Name("k".to_string()),
        HirExpr::NamedExpr {
            name: "m".to_string(),
            value: Box::new(HirExpr::Name("m".to_string())),
        },
    )]))];

    let err = collect_block_constraints(
        &signatures,
        &mut parents,
        &mut concrete,
        &mut constraints,
        &mut env,
        &body,
        None,
    )
    .unwrap_err();

    assert_eq!(err.code, "T0021");
}

#[test]
fn walrus_nested_inside_a_dict_get_or_default_key_on_the_solver_path_propagates_a_forward_reference_error()
 {
    // Exercises `DictGetOrDefault`'s own `bind_named_expr_targets(key)?`
    // call (source line 1046).
    let signatures = HashMap::new();
    let mut parents = Vec::new();
    let mut concrete = Vec::new();
    let mut constraints = SolverConstraints::default();
    let mut env = ConstraintEnvironment::empty(&["m"]);
    let body = vec![HirStmt::ExprStmt(HirExpr::DictGetOrDefault {
        dict: pycc_hir::ContainerReceiver::Name("d".to_string()),
        key: Box::new(HirExpr::NamedExpr {
            name: "m".to_string(),
            value: Box::new(HirExpr::Name("m".to_string())),
        }),
        default: Box::new(HirExpr::Name("v".to_string())),
    })];

    let err = collect_block_constraints(
        &signatures,
        &mut parents,
        &mut concrete,
        &mut constraints,
        &mut env,
        &body,
        None,
    )
    .unwrap_err();

    assert_eq!(err.code, "T0021");
}

#[test]
fn walrus_nested_inside_a_method_call_base_on_the_solver_path_propagates_a_forward_reference_error()
{
    // Exercises `MethodCall`'s own `bind_named_expr_targets(base)?` call
    // (source line 1053).
    let signatures = HashMap::new();
    let mut parents = Vec::new();
    let mut concrete = Vec::new();
    let mut constraints = SolverConstraints::default();
    let mut env = ConstraintEnvironment::empty(&["m"]);
    let body = vec![HirStmt::ExprStmt(HirExpr::MethodCall {
        base: Box::new(HirExpr::NamedExpr {
            name: "m".to_string(),
            value: Box::new(HirExpr::Name("m".to_string())),
        }),
        method: "foo".to_string(),
        args: vec![],
    })];

    let err = collect_block_constraints(
        &signatures,
        &mut parents,
        &mut concrete,
        &mut constraints,
        &mut env,
        &body,
        None,
    )
    .unwrap_err();

    assert_eq!(err.code, "T0021");
}

#[test]
fn walrus_nested_inside_a_method_call_arg_on_the_solver_path_propagates_a_forward_reference_error()
{
    // Exercises `MethodCall`'s own `bind_named_expr_targets(arg)?` call
    // inside its argument loop (source line 1055). The base is a harmless
    // bare name so the base's own recursion (line 1053) succeeds and the
    // argument loop is reached.
    let signatures = HashMap::new();
    let mut parents = Vec::new();
    let mut concrete = Vec::new();
    let mut constraints = SolverConstraints::default();
    let mut env = ConstraintEnvironment::empty(&["m"]);
    let body = vec![HirStmt::ExprStmt(HirExpr::MethodCall {
        base: Box::new(HirExpr::Name("xs".to_string())),
        method: "foo".to_string(),
        args: vec![HirExpr::NamedExpr {
            name: "m".to_string(),
            value: Box::new(HirExpr::Name("m".to_string())),
        }],
    })];

    let err = collect_block_constraints(
        &signatures,
        &mut parents,
        &mut concrete,
        &mut constraints,
        &mut env,
        &body,
        None,
    )
    .unwrap_err();

    assert_eq!(err.code, "T0021");
}

#[test]
fn walrus_nested_inside_a_generic_class_instantiate_arg_on_the_solver_path_propagates_a_forward_reference_error()
 {
    // Exercises `GenericClassInstantiate`'s own `bind_named_expr_targets(arg)?`
    // call inside its argument loop (source line 1061).
    let signatures = HashMap::new();
    let mut parents = Vec::new();
    let mut concrete = Vec::new();
    let mut constraints = SolverConstraints::default();
    let mut env = ConstraintEnvironment::empty(&["m"]);
    let body = vec![HirStmt::ExprStmt(HirExpr::GenericClassInstantiate {
        class: "C".to_string(),
        type_arg: Ty::Int,
        args: vec![HirExpr::NamedExpr {
            name: "m".to_string(),
            value: Box::new(HirExpr::Name("m".to_string())),
        }],
    })];

    let err = collect_block_constraints(
        &signatures,
        &mut parents,
        &mut concrete,
        &mut constraints,
        &mut env,
        &body,
        None,
    )
    .unwrap_err();

    assert_eq!(err.code, "T0021");
}
