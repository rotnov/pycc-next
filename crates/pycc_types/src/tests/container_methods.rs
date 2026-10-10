//! `append`, `pop`, `dict.get` and `set.add` checks.

use super::*;

#[test]
fn appending_a_matching_value_to_a_list_infers_none() {
    let mut env = Environment::new();
    env.bind("x".to_string(), Ty::List(Box::new(Ty::Int)));
    let expr = HirExpr::ListAppend {
        list: pycc_hir::ContainerReceiver::Name("x".to_string()),
        value: Box::new(HirExpr::IntLiteral(1)),
    };
    assert_eq!(infer_expr(&env, &expr), Ok(Ty::None));
}

#[test]
fn appending_a_str_to_a_list_of_str_infers_none_proving_append_is_not_int_specific() {
    let mut env = Environment::new();
    env.bind("x".to_string(), Ty::List(Box::new(Ty::Str)));
    let expr = HirExpr::ListAppend {
        list: pycc_hir::ContainerReceiver::Name("x".to_string()),
        value: Box::new(HirExpr::StringLiteral("a".to_string())),
    };
    assert_eq!(infer_expr(&env, &expr), Ok(Ty::None));
}

#[test]
fn appending_to_an_undefined_name_is_rejected_as_not_defined() {
    let env = Environment::new();
    let expr = HirExpr::ListAppend {
        list: pycc_hir::ContainerReceiver::Name("undefined".to_string()),
        value: Box::new(HirExpr::IntLiteral(1)),
    };
    let err = infer_expr(&env, &expr).unwrap_err();
    assert_eq!(err.code, "T0021");
    assert!(err.message.contains("not defined"));
}

#[test]
fn appending_to_a_local_name_read_before_assignment_is_unbound_local() {
    // `ListAppend`'s `list` field is a plain `String`, not an
    // `HirExpr::Name`, so it must replicate `HirExpr::Name`'s own
    // "unbound local" vs. "not defined" distinction by hand
    // (`lookup_bound_name`) rather than going through `infer_expr_in`'s
    // `Name` arm.
    let env = Environment::new();
    let expr = HirExpr::ListAppend {
        list: pycc_hir::ContainerReceiver::Name("x".to_string()),
        value: Box::new(HirExpr::IntLiteral(1)),
    };
    let err = infer_expr_in(&env, &["x"], &expr).unwrap_err();
    assert_eq!(err.code, "T0021");
    assert!(err.message.contains("not bound before this use"));
}

#[test]
fn appending_to_a_non_list_value_is_rejected_as_t0033() {
    let mut env = Environment::new();
    env.bind("x".to_string(), Ty::Int);
    let expr = HirExpr::ListAppend {
        list: pycc_hir::ContainerReceiver::Name("x".to_string()),
        value: Box::new(HirExpr::IntLiteral(1)),
    };
    let err = infer_expr(&env, &expr).unwrap_err();
    assert_eq!(err.code, "T0033");
    assert!(err.message.contains("does not support `.append()`"));
}

#[test]
fn appending_a_mismatched_value_type_is_rejected_as_t0021() {
    let mut env = Environment::new();
    env.bind("x".to_string(), Ty::List(Box::new(Ty::Int)));
    let expr = HirExpr::ListAppend {
        list: pycc_hir::ContainerReceiver::Name("x".to_string()),
        value: Box::new(HirExpr::StringLiteral("nope".to_string())),
    };
    let err = infer_expr(&env, &expr).unwrap_err();
    assert_eq!(err.code, "T0021");
    assert!(err.message.contains("cannot append"));
}

#[test]
fn appending_a_bool_to_a_list_of_int_is_accepted_since_bool_is_an_int_subtype() {
    // D-086, matching `Subscript`'s own index check in `lib.rs`:
    // `x.append(True)` on an already-typed `list[int]` is ordinary,
    // CPython-valid Python (`bool` is an `int` subtype) --
    // `is_assignable` applies here, not
    // ListLiteral's stricter exact-equality homogeneity rule (which
    // answers a different question: inferring an as-yet-unknown element
    // type, not checking a value against an already-known one). Found
    // by an automated whole-branch review (PR #236) and confirmed
    // against a real `pycc build` before this fix.
    let mut env = Environment::new();
    env.bind("x".to_string(), Ty::List(Box::new(Ty::Int)));
    let expr = HirExpr::ListAppend {
        list: pycc_hir::ContainerReceiver::Name("x".to_string()),
        value: Box::new(HirExpr::BoolLiteral(true)),
    };
    assert_eq!(infer_expr(&env, &expr), Ok(Ty::None));
}

#[test]
fn appending_propagates_an_ill_typed_value_s_error() {
    let mut env = Environment::new();
    env.bind("x".to_string(), Ty::List(Box::new(Ty::Int)));
    let expr = HirExpr::ListAppend {
        list: pycc_hir::ContainerReceiver::Name("x".to_string()),
        value: Box::new(HirExpr::Name("undefined".to_string())),
    };
    assert_eq!(infer_expr(&env, &expr).unwrap_err().code, "T0021");
}

// -- PR-12 Task 10 (D-119): remaining container methods depth --------
// `list.pop()`, `dict.get(key, default)`, `set.add(value)` -- each
// mirrors `ListAppend`'s own `infer_expr_in` test coverage exactly
// (base-type gate, value/key/default-type gate, `lookup_bound_name`'s
// own "not defined"/"unbound local" distinction, propagation, D-086
// bool-subtypes-int leniency, genericity).

#[test]
fn popping_the_last_element_of_a_list_of_int_infers_int() {
    let mut env = Environment::new();
    env.bind("x".to_string(), Ty::List(Box::new(Ty::Int)));
    let expr = HirExpr::ListPop {
        list: pycc_hir::ContainerReceiver::Name("x".to_string()),
    };
    assert_eq!(infer_expr(&env, &expr), Ok(Ty::Int));
}

#[test]
fn popping_from_a_list_of_str_infers_str_proving_pop_is_not_int_specific() {
    let mut env = Environment::new();
    env.bind("x".to_string(), Ty::List(Box::new(Ty::Str)));
    let expr = HirExpr::ListPop {
        list: pycc_hir::ContainerReceiver::Name("x".to_string()),
    };
    assert_eq!(infer_expr(&env, &expr), Ok(Ty::Str));
}

#[test]
fn popping_from_an_undefined_name_is_rejected_as_not_defined() {
    let env = Environment::new();
    let expr = HirExpr::ListPop {
        list: pycc_hir::ContainerReceiver::Name("undefined".to_string()),
    };
    let err = infer_expr(&env, &expr).unwrap_err();
    assert_eq!(err.code, "T0021");
    assert!(err.message.contains("not defined"));
}

#[test]
fn popping_from_a_local_name_read_before_assignment_is_unbound_local() {
    let env = Environment::new();
    let expr = HirExpr::ListPop {
        list: pycc_hir::ContainerReceiver::Name("x".to_string()),
    };
    let err = infer_expr_in(&env, &["x"], &expr).unwrap_err();
    assert_eq!(err.code, "T0021");
    assert!(err.message.contains("not bound before this use"));
}

#[test]
fn popping_from_a_non_list_value_is_rejected_as_t0033() {
    let mut env = Environment::new();
    env.bind("x".to_string(), Ty::Int);
    let expr = HirExpr::ListPop {
        list: pycc_hir::ContainerReceiver::Name("x".to_string()),
    };
    let err = infer_expr(&env, &expr).unwrap_err();
    assert_eq!(err.code, "T0033");
    assert!(err.message.contains("does not support `.pop()`"));
}

#[test]
fn getting_a_str_key_with_a_matching_int_default_infers_int() {
    let mut env = Environment::new();
    env.bind("d".to_string(), Ty::Dict(Box::new((Ty::Str, Ty::Int))));
    let expr = HirExpr::DictGetOrDefault {
        dict: pycc_hir::ContainerReceiver::Name("d".to_string()),
        key: Box::new(HirExpr::StringLiteral("a".to_string())),
        default: Box::new(HirExpr::IntLiteral(0)),
    };
    assert_eq!(infer_expr(&env, &expr), Ok(Ty::Int));
}

#[test]
fn getting_a_bool_default_for_a_dict_of_int_values_is_accepted_since_bool_is_an_int_subtype() {
    // D-086, matching `ListAppend`'s own leniency: `bool` is an `int`
    // subtype, so a `bool` default for a `dict[str, int]` is ordinary,
    // CPython-valid Python, not a type error.
    let mut env = Environment::new();
    env.bind("d".to_string(), Ty::Dict(Box::new((Ty::Str, Ty::Int))));
    let expr = HirExpr::DictGetOrDefault {
        dict: pycc_hir::ContainerReceiver::Name("d".to_string()),
        key: Box::new(HirExpr::StringLiteral("a".to_string())),
        default: Box::new(HirExpr::BoolLiteral(true)),
    };
    assert_eq!(infer_expr(&env, &expr), Ok(Ty::Int));
}

#[test]
fn getting_from_an_undefined_dict_is_rejected_as_not_defined() {
    let env = Environment::new();
    let expr = HirExpr::DictGetOrDefault {
        dict: pycc_hir::ContainerReceiver::Name("undefined".to_string()),
        key: Box::new(HirExpr::StringLiteral("a".to_string())),
        default: Box::new(HirExpr::IntLiteral(0)),
    };
    let err = infer_expr(&env, &expr).unwrap_err();
    assert_eq!(err.code, "T0021");
    assert!(err.message.contains("not defined"));
}

#[test]
fn getting_from_a_local_dict_read_before_assignment_is_unbound_local() {
    let env = Environment::new();
    let expr = HirExpr::DictGetOrDefault {
        dict: pycc_hir::ContainerReceiver::Name("d".to_string()),
        key: Box::new(HirExpr::StringLiteral("a".to_string())),
        default: Box::new(HirExpr::IntLiteral(0)),
    };
    let err = infer_expr_in(&env, &["d"], &expr).unwrap_err();
    assert_eq!(err.code, "T0021");
    assert!(err.message.contains("not bound before this use"));
}

#[test]
fn getting_from_a_non_dict_value_is_rejected_as_t0033() {
    let mut env = Environment::new();
    env.bind("d".to_string(), Ty::Int);
    let expr = HirExpr::DictGetOrDefault {
        dict: pycc_hir::ContainerReceiver::Name("d".to_string()),
        key: Box::new(HirExpr::StringLiteral("a".to_string())),
        default: Box::new(HirExpr::IntLiteral(0)),
    };
    let err = infer_expr(&env, &expr).unwrap_err();
    assert_eq!(err.code, "T0033");
    assert!(err.message.contains("does not support `.get()`"));
}

#[test]
fn getting_with_a_mismatched_key_type_is_rejected_as_t0021() {
    let mut env = Environment::new();
    env.bind("d".to_string(), Ty::Dict(Box::new((Ty::Str, Ty::Int))));
    let expr = HirExpr::DictGetOrDefault {
        dict: pycc_hir::ContainerReceiver::Name("d".to_string()),
        key: Box::new(HirExpr::IntLiteral(1)),
        default: Box::new(HirExpr::IntLiteral(0)),
    };
    let err = infer_expr(&env, &expr).unwrap_err();
    assert_eq!(err.code, "T0021");
    assert!(err.message.contains("cannot look up"));
}

#[test]
fn getting_with_a_mismatched_default_type_is_rejected_as_t0021() {
    let mut env = Environment::new();
    env.bind("d".to_string(), Ty::Dict(Box::new((Ty::Str, Ty::Int))));
    let expr = HirExpr::DictGetOrDefault {
        dict: pycc_hir::ContainerReceiver::Name("d".to_string()),
        key: Box::new(HirExpr::StringLiteral("a".to_string())),
        default: Box::new(HirExpr::StringLiteral("nope".to_string())),
    };
    let err = infer_expr(&env, &expr).unwrap_err();
    assert_eq!(err.code, "T0021");
    assert!(err.message.contains("cannot use"));
}

#[test]
fn getting_propagates_an_ill_typed_key_s_error() {
    let mut env = Environment::new();
    env.bind("d".to_string(), Ty::Dict(Box::new((Ty::Str, Ty::Int))));
    let expr = HirExpr::DictGetOrDefault {
        dict: pycc_hir::ContainerReceiver::Name("d".to_string()),
        key: Box::new(HirExpr::Name("undefined".to_string())),
        default: Box::new(HirExpr::IntLiteral(0)),
    };
    assert_eq!(infer_expr(&env, &expr).unwrap_err().code, "T0021");
}

#[test]
fn getting_propagates_an_ill_typed_default_s_error() {
    let mut env = Environment::new();
    env.bind("d".to_string(), Ty::Dict(Box::new((Ty::Str, Ty::Int))));
    let expr = HirExpr::DictGetOrDefault {
        dict: pycc_hir::ContainerReceiver::Name("d".to_string()),
        key: Box::new(HirExpr::StringLiteral("a".to_string())),
        default: Box::new(HirExpr::Name("undefined".to_string())),
    };
    assert_eq!(infer_expr(&env, &expr).unwrap_err().code, "T0021");
}

#[test]
fn adding_an_int_value_to_a_set_of_int_infers_none() {
    let mut env = Environment::new();
    env.bind("s".to_string(), Ty::Set(Box::new(Ty::Int)));
    let expr = HirExpr::SetAdd {
        set: "s".to_string(),
        value: Box::new(HirExpr::IntLiteral(1)),
    };
    assert_eq!(infer_expr(&env, &expr), Ok(Ty::None));
}

#[test]
fn adding_a_bool_value_to_a_set_of_int_is_accepted_since_bool_is_an_int_subtype() {
    let mut env = Environment::new();
    env.bind("s".to_string(), Ty::Set(Box::new(Ty::Int)));
    let expr = HirExpr::SetAdd {
        set: "s".to_string(),
        value: Box::new(HirExpr::BoolLiteral(true)),
    };
    assert_eq!(infer_expr(&env, &expr), Ok(Ty::None));
}

#[test]
fn adding_to_a_set_of_an_unhashable_element_is_refused_by_the_set_element_check() {
    // #1343: `.add` checks its element (`set_element.rs`) because a `set[R]`
    // parameter reaches it with no literal checked. `set[list[int]]` never gets
    // past the annotation gate, so this pins the defensive arm.
    let mut env = Environment::new();
    env.bind(
        "s".to_string(),
        Ty::Set(Box::new(Ty::List(Box::new(Ty::Int)))),
    );
    let expr = HirExpr::SetAdd {
        set: "s".to_string(),
        value: Box::new(HirExpr::ListLiteral(vec![HirExpr::IntLiteral(1)])),
    };
    assert_eq!(infer_expr(&env, &expr).unwrap_err().code, "T0038");
}

#[test]
fn adding_to_an_undefined_set_is_rejected_as_not_defined() {
    let env = Environment::new();
    let expr = HirExpr::SetAdd {
        set: "undefined".to_string(),
        value: Box::new(HirExpr::IntLiteral(1)),
    };
    let err = infer_expr(&env, &expr).unwrap_err();
    assert_eq!(err.code, "T0021");
    assert!(err.message.contains("not defined"));
}

#[test]
fn adding_to_a_local_set_read_before_assignment_is_unbound_local() {
    let env = Environment::new();
    let expr = HirExpr::SetAdd {
        set: "s".to_string(),
        value: Box::new(HirExpr::IntLiteral(1)),
    };
    let err = infer_expr_in(&env, &["s"], &expr).unwrap_err();
    assert_eq!(err.code, "T0021");
    assert!(err.message.contains("not bound before this use"));
}

#[test]
fn adding_to_a_non_set_value_is_rejected_as_t0033() {
    let mut env = Environment::new();
    env.bind("s".to_string(), Ty::Int);
    let expr = HirExpr::SetAdd {
        set: "s".to_string(),
        value: Box::new(HirExpr::IntLiteral(1)),
    };
    let err = infer_expr(&env, &expr).unwrap_err();
    assert_eq!(err.code, "T0033");
    assert!(err.message.contains("does not support `.add()`"));
}

#[test]
fn adding_to_a_frozenset_is_rejected_as_t0033_naming_frozenset() {
    // Part 1 of #1319: a `frozenset[int]` is immutable, so `.add` is
    // refused by the same arm as any other non-set receiver, and the
    // message names the receiver's own type.
    let mut env = Environment::new();
    env.bind("fs".to_string(), Ty::FrozenSet(Box::new(Ty::Int)));
    let expr = HirExpr::SetAdd {
        set: "fs".to_string(),
        value: Box::new(HirExpr::IntLiteral(1)),
    };
    let err = infer_expr(&env, &expr).unwrap_err();
    assert_eq!(err.code, "T0033");
    assert_eq!(err.message, "`frozenset[int]` does not support `.add()`");
}

#[test]
fn adding_a_mismatched_value_type_is_rejected_as_t0021() {
    let mut env = Environment::new();
    env.bind("s".to_string(), Ty::Set(Box::new(Ty::Int)));
    let expr = HirExpr::SetAdd {
        set: "s".to_string(),
        value: Box::new(HirExpr::StringLiteral("nope".to_string())),
    };
    let err = infer_expr(&env, &expr).unwrap_err();
    assert_eq!(err.code, "T0021");
    assert!(err.message.contains("cannot add"));
}

#[test]
fn adding_propagates_an_ill_typed_value_s_error() {
    let mut env = Environment::new();
    env.bind("s".to_string(), Ty::Set(Box::new(Ty::Int)));
    let expr = HirExpr::SetAdd {
        set: "s".to_string(),
        value: Box::new(HirExpr::Name("undefined".to_string())),
    };
    assert_eq!(infer_expr(&env, &expr).unwrap_err().code, "T0021");
}

// Whole-module `check()` tests, mirroring `HirExpr::Slice`'s own pair
// (PR-12 Task 7, D-118) exactly: Task 3's own ledger records a real
// regression of exactly this shape (a solver arm that looked correct in
// isolation but was never actually reached by the block walker) -- these
// confirm each new arm is genuinely wired into both the fast
// (`check_with_environment_all`) and solver (`collect_block_constraints`)
// paths, not just correct when `infer_expr`/`collect_expr_constraints`
// are called directly on a hand-built expression.

#[test]
fn list_pop_type_checks_correctly_when_an_unrelated_private_helper_forces_the_solver_path() {
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            HirItem::TopLevelStmt(HirStmt::Assign {
                target: "xs".to_string(),
                value: HirExpr::ListLiteral(vec![HirExpr::IntLiteral(1)]),
            }),
            HirItem::TopLevelStmt(HirStmt::Assign {
                target: "y".to_string(),
                value: HirExpr::ListPop {
                    list: pycc_hir::ContainerReceiver::Name("xs".to_string()),
                },
            }),
            HirItem::Function {
                name: "_constant".to_string(),
                params: vec![],
                return_ty: Ty::Infer,
                body: vec![HirStmt::Return(Some(HirExpr::IntLiteral(1)))],
            },
        ],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    assert!(check(&hir).is_ok());
}

#[test]
fn dict_get_or_default_type_checks_correctly_when_an_unrelated_private_helper_forces_the_solver_path()
 {
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            HirItem::TopLevelStmt(HirStmt::Assign {
                target: "d".to_string(),
                value: HirExpr::DictLiteral(vec![(
                    HirExpr::StringLiteral("a".to_string()),
                    HirExpr::IntLiteral(1),
                )]),
            }),
            HirItem::TopLevelStmt(HirStmt::Assign {
                target: "y".to_string(),
                value: HirExpr::DictGetOrDefault {
                    dict: pycc_hir::ContainerReceiver::Name("d".to_string()),
                    key: Box::new(HirExpr::StringLiteral("a".to_string())),
                    default: Box::new(HirExpr::IntLiteral(0)),
                },
            }),
            HirItem::Function {
                name: "_constant".to_string(),
                params: vec![],
                return_ty: Ty::Infer,
                body: vec![HirStmt::Return(Some(HirExpr::IntLiteral(1)))],
            },
        ],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    assert!(check(&hir).is_ok());
}

#[test]
fn set_add_type_checks_correctly_when_an_unrelated_private_helper_forces_the_solver_path() {
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            HirItem::TopLevelStmt(HirStmt::Assign {
                target: "s".to_string(),
                value: HirExpr::SetLiteral(vec![HirExpr::IntLiteral(1)]),
            }),
            HirItem::TopLevelStmt(HirStmt::ExprStmt(HirExpr::SetAdd {
                set: "s".to_string(),
                value: Box::new(HirExpr::IntLiteral(2)),
            })),
            HirItem::Function {
                name: "_constant".to_string(),
                params: vec![],
                return_ty: Ty::Infer,
                body: vec![HirStmt::Return(Some(HirExpr::IntLiteral(1)))],
            },
        ],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    assert!(check(&hir).is_ok());
}

#[test]
fn list_pop_on_a_list_typed_parameter_infers_the_scalar_element_type_inside_an_unannotated_private_helper()
 {
    // D-146 (#239): `collect_expr_constraints`'s `ListPop` arm now looks
    // up `list` in `env.bindings` and, when bound to a `Ty::List` element-
    // type carrier, extracts the scalar element type -- the carrier is
    // destructured, never unified. Here `xs` is a parameter typed
    // `list[int]`, so `xs.pop()` produces `Some(Ok(Ty::Int))`, `y` is
    // bound to `Ok(Ty::Int)`, and `return y` resolves the unannotated
    // return to `Ty::Int`. This was the pre-existing D-116 solver binding
    // gap: before this fix the `ListPop` arm returned `Ok(None)`, so `y`
    // was never bound and the later `return y` failed with the actively
    // misleading "not bound before this use" even though the assignment
    // was textually right above it.
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![HirItem::Function {
            name: "_h".to_string(),
            params: vec![("xs".to_string(), Ty::List(Box::new(Ty::Int)))],
            return_ty: Ty::Infer,
            body: vec![
                HirStmt::Assign {
                    target: "y".to_string(),
                    value: HirExpr::ListPop {
                        list: pycc_hir::ContainerReceiver::Name("xs".to_string()),
                    },
                },
                HirStmt::Return(Some(HirExpr::Name("y".to_string()))),
            ],
        }],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    let local_names = module_function_local_names(&hir);
    let signatures = infer_function_signatures_with_solver(&hir, &local_names).unwrap();
    assert_eq!(
        signatures["_h"],
        (vec![Ty::List(Box::new(Ty::Int))], Ty::Int)
    );
    assert!(check(&hir).is_ok());
}

#[test]
fn dict_get_or_default_assigned_inside_an_unannotated_private_helper_reaches_the_genuine_cannot_infer_return_type_diagnostic()
 {
    // Same pre-existing D-116 gap as the `ListPop` test immediately
    // above, exercised through `DictGetOrDefault`'s own
    // `collect_expr_constraints` arm (also `Ok(None)`) instead. Issue
    // #771/D-199: `y` is now tracked as definitely-but-opaquely bound
    // (`opaque_bindings`), so `return y` no longer misfires as an unbound
    // local ("not bound before this use") -- it correctly falls through
    // to the genuine "cannot infer return type" diagnostic instead, since
    // the solver still has no type term for `y` (that half of the gap is
    // unchanged; only the misleading consequence is fixed).
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![HirItem::Function {
            name: "_h".to_string(),
            params: vec![("d".to_string(), Ty::Dict(Box::new((Ty::Str, Ty::Int))))],
            return_ty: Ty::Infer,
            body: vec![
                HirStmt::Assign {
                    target: "y".to_string(),
                    value: HirExpr::DictGetOrDefault {
                        dict: pycc_hir::ContainerReceiver::Name("d".to_string()),
                        key: Box::new(HirExpr::StringLiteral("a".to_string())),
                        default: Box::new(HirExpr::IntLiteral(0)),
                    },
                },
                HirStmt::Return(Some(HirExpr::Name("y".to_string()))),
            ],
        }],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    let err = check(&hir).unwrap_err();
    assert_eq!(err.code, "T0021");
    assert_eq!(
        err.message,
        "cannot infer return type of private helper `_h`; add an annotation"
    );
}
