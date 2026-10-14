//! Walrus (`NamedExpr`) binding, scoping and error propagation.

use super::*;

// PEP 572 (#774): `function_local_names`'s own `collect_named_expr_names_in_
// expr` walk records a walrus target as a function-local name wherever it is
// nested -- including inside a unary operand and a slice bound, which no
// other test in this suite reaches (every other walrus fixture is
// module-level, never inside a `HirItem::Function` body). Hand-built HIR,
// bypassing the parser, to pin the exact nesting shape.
#[test]
fn a_walrus_inside_a_unary_operand_in_a_function_body_is_a_local_name() {
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![HirItem::Function {
            name: "f".to_string(),
            params: vec![],
            return_ty: Ty::Int,
            body: vec![
                HirStmt::If {
                    test: HirExpr::Compare {
                        op: CmpOpKind::Lt,
                        left: Box::new(HirExpr::UnaryOp {
                            op: UnaryOpKind::USub,
                            operand: Box::new(HirExpr::NamedExpr {
                                name: "n".to_string(),
                                value: Box::new(HirExpr::IntLiteral(3)),
                            }),
                        }),
                        right: Box::new(HirExpr::IntLiteral(0)),
                    },
                    body: vec![HirStmt::Return(Some(HirExpr::Name("n".to_string())))],
                    orelse: vec![],
                },
                HirStmt::Return(Some(HirExpr::IntLiteral(0))),
            ],
        }],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    assert!(check(&hir).is_ok(), "{:?}", check(&hir));
}

#[test]
fn a_walrus_inside_a_slice_bound_in_a_function_body_is_a_local_name() {
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![HirItem::Function {
            name: "g".to_string(),
            params: vec![("lst".to_string(), Ty::List(Box::new(Ty::Int)))],
            return_ty: Ty::Int,
            body: vec![
                HirStmt::ExprStmt(HirExpr::Slice {
                    base: Box::new(HirExpr::Name("lst".to_string())),
                    start: Some(Box::new(HirExpr::NamedExpr {
                        name: "m".to_string(),
                        value: Box::new(HirExpr::IntLiteral(0)),
                    })),
                    stop: None,
                    step: None,
                }),
                HirStmt::Return(Some(HirExpr::Name("m".to_string()))),
            ],
        }],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    assert!(check(&hir).is_ok(), "{:?}", check(&hir));
}

// -- PEP 572 (#774): the walrus assignment (`:=`) value-type restriction
// (T0050) -----------------------------------------------------------

#[test]
fn a_walrus_with_an_int_value_type_checks() {
    let result = check_source("if (n := 7) > 5:\n    print(n)\n");
    assert!(
        result.is_ok(),
        "an `int`-valued walrus must type-check: {result:?}"
    );
}

#[test]
fn a_walrus_with_an_optional_int_value_type_checks() {
    // `is_walrus_value_ty_supported`'s `Ty::Optional(inner)` arm recurses
    // into `inner` -- here an `Optional[int]` walrus value, reached via a
    // `Name` already declared `int | None`.
    let result = check_source("x: int | None = 5\nif (y := x) is not None:\n    z = y\n");
    assert!(
        result.is_ok(),
        "an `Optional[int]`-valued walrus must type-check: {result:?}"
    );
}

#[test]
fn a_walrus_with_a_str_value_is_rejected_with_t0050() {
    let result = check_source("s = \"hi\"\nif (y := s) == \"hi\":\n    print(y)\n");
    let err = result.expect_err("a `str`-valued walrus must be rejected");
    assert_eq!(err.code, "T0050");
}

#[test]
fn a_walrus_with_an_optional_str_value_is_rejected_with_t0050() {
    // `is_walrus_value_ty_supported`'s `Ty::Optional(inner)` arm recurses
    // into `inner` and, for an unsupported inner type such as `str`,
    // reports unsupported -- the region gap this closes is that recursive
    // `false` return specifically (the sibling `Optional[int]` test above
    // only exercises the recursive `true` return). A real parsed program
    // can never reach this: `annotation_to_ty` (`pycc_hir/src/func.rs`) is
    // the one place a `Ty::Optional` is ever constructed from source, and
    // it rejects every inner type but `int`/`float`/`bool` at lowering
    // time with T0049,
    // before `pycc_types` ever runs -- so `Ty::Optional(Ty::Str)` only
    // exists as a hand-built `Environment` binding here, bypassing
    // `check_source`/lowering entirely to reach the type-checker's own
    // `is_walrus_value_ty_supported` directly.
    let mut env = Environment::new();
    env.bind("x".to_string(), Ty::Optional(Box::new(Ty::Str)));
    let expr = HirExpr::NamedExpr {
        name: "y".to_string(),
        value: Box::new(HirExpr::Name("x".to_string())),
    };
    let err =
        infer_expr(&env, &expr).expect_err("an `Optional[str]`-valued walrus must be rejected");
    assert_eq!(err.code, "T0050");
}

#[test]
fn a_walrus_whose_value_fails_to_infer_propagates_the_error_before_the_t0050_check() {
    // `infer_expr_in`'s own `NamedExpr` arm (`crates/pycc_types/src/expr.rs`)
    // computes `value`'s type via its own `infer_expr_in(..)?` call before
    // ever reaching `is_walrus_value_ty_supported` -- that `?`'s error
    // branch is distinct from `collect_named_expr_bindings`'s identically-
    // shaped call in `lib.rs` (already covered elsewhere in this file):
    // `collect_named_expr_bindings` always runs first, over the same
    // `value`, so any error reachable through an ordinary `check_source`
    // program is always caught there first, never here. Reaching this
    // arm's own failure requires calling `infer_expr` directly (as the
    // sibling `Optional[str]` test above does) on a `NamedExpr` whose
    // `value` is a plain undefined-name read.
    let env = Environment::new();
    let expr = HirExpr::NamedExpr {
        name: "y".to_string(),
        value: Box::new(HirExpr::Name("undefined".to_string())),
    };
    let err = infer_expr(&env, &expr).expect_err("an undefined-name walrus value must be rejected");
    assert_eq!(err.code, "T0021");
}

#[test]
fn a_walrus_in_a_generic_functions_body_does_not_trip_the_t0042_self_call_scan() {
    // `reject_generic_calls_in_expr`'s own `HirExpr::NamedExpr` arm (#774):
    // a generic function's body is pre-scanned for illegal self-calls
    // (D-134/T0042) before it is type-checked at all, and that scan walks
    // every `HirExpr` including a bare walrus expression statement.
    let result = check_source("def identity[T](value: T) -> T:\n    (z := 1)\n    return value\n");
    assert!(
        result.is_ok(),
        "a walrus with no generic self-call must not trip T0042: {result:?}"
    );
}

#[test]
fn a_walrus_nested_inside_a_list_slice_and_dict_literal_binds_every_name() {
    // Module-level `collect_named_expr_bindings` walk (lib.rs), exercising
    // its `ListLiteral`/`SetLiteral`/`TupleLiteral`, `Slice`, and
    // `DictLiteral` arms in one bare expression statement -- mirroring
    // `pycc_mir`'s own equivalent walk test for the same #774 nesting shape.
    let result = check_source(
        "lst = [1, 2, 3]\n[a := 1]\nlst[(b := 0):]\n{\"k\": (c := 3)}\nprint(a)\nprint(b)\nprint(c)\n",
    );
    assert!(
        result.is_ok(),
        "every nested walrus target must be bound and visible afterward: {result:?}"
    );
}

// The test above only exercises `collect_named_expr_bindings`'s
// `ListLiteral`/`SetLiteral`/`TupleLiteral`, `Slice`, and `DictLiteral`
// arms. This one covers every other container-shaped arm at module scope:
// `Call` args, `BinOp`, `Compare`, an `FString` interpolation, a
// non-`Slice` `Subscript`, `ListAppend`, `SetAdd`, `AttrGet` (via a `Call`
// base), `MethodCall` args, `GenericClassInstantiate` args, and
// `DictGetOrDefault`.
//
// The fixture deliberately includes a generic class (`Box[T]`) so that
// `monomorphize`'s top-level rewrite pass actually runs (its early-return
// fast path is skipped whenever the module has a generic function/class or
// a protocol-typed parameter): `rewrite_generic_calls_in_expr`'s own
// `NamedExpr` arm calls `env.bind`, so a module-level walrus name bound by
// an earlier bare statement stays visible to a later top-level statement's
// name lookup inside that pass, exactly like the ordinary
// `check`/`check_and_resolve` path. Goes through `check_source` (full
// `check_and_resolve`, `monomorphize` included) rather than `check` alone,
// so a regression in either pass's env bookkeeping fails this test.
#[test]
fn a_walrus_nested_inside_every_remaining_container_shape_binds_every_name_at_module_scope() {
    let result = check_source(
        "\
class Box[T]:
    def __init__(self, x: T):
        self.x = x
class C:
    def __init__(self, x: int):
        self.x = x
    def m(self, y: int) -> int:
        return y
def f(v: int) -> int:
    return v
def make_c(v: int) -> C:
    return C(v)
c = C(3)
lst = [1, 2, 3]
s = {1, 2}
dct = {\"k\": 2}
print(f((a := 1)))
print((b := 2) + 1)
print((dd := 3) == 3)
print(f\"{(e := 4)}\")
print(lst[(g := 0)])
lst.append((h := 5))
s.add((i := 6))
print(make_c((j := 0)).x)
print(c.m((k := 7)))
print(Box[int]((l := 8)).x)
print(dct.get(f\"k{(m := 1)}\", 0))
{(p := 9)}
((q := 10),)
print(a, b, dd, e, g, h, i, j, k, l, m, p, q)
",
    );
    assert!(
        result.is_ok(),
        "every nested walrus target across every container shape must be bound and visible \
         afterward: {result:?}"
    );
}

// PEP 572 (#774): `collect_named_expr_bindings` is called from six call
// sites -- module-scope `check_stmt`'s `ExprStmt`/`If`/`While` arms and
// function-scope `check_stmt_in_function`'s same three arms -- each behind
// a `?`. These six tests exercise that error-propagation branch at each
// site: `n` is first bound `str`, then a nested walrus tries to rebind it
// `int` (a `T0050`-supported value type), which `check_assignment` rejects
// as an incompatible redefinition (`T0023`).
#[test]
fn a_failing_walrus_binding_in_a_module_bare_expr_stmt_propagates_t0023() {
    let err = check_source("n = \"hi\"\n(n := 1)\n").expect_err("incompatible redefinition");
    assert_eq!(err.code, "T0023");
}

#[test]
fn a_failing_walrus_binding_in_a_module_if_test_propagates_t0023() {
    let err = check_source("n = \"hi\"\nif (n := 1) > 0:\n    pass\n")
        .expect_err("incompatible redefinition");
    assert_eq!(err.code, "T0023");
}

#[test]
fn a_failing_walrus_binding_in_a_module_while_test_propagates_t0023() {
    let err = check_source("n = \"hi\"\nwhile (n := 1) > 0:\n    pass\n")
        .expect_err("incompatible redefinition");
    assert_eq!(err.code, "T0023");
}

#[test]
fn a_failing_walrus_binding_in_a_function_bare_expr_stmt_propagates_t0023() {
    let err = check_source("def f() -> int:\n    n = \"hi\"\n    (n := 1)\n    return 0\n")
        .expect_err("incompatible redefinition");
    assert_eq!(err.code, "T0023");
}

#[test]
fn a_failing_walrus_binding_in_a_function_if_test_propagates_t0023() {
    let err = check_source(
        "def f() -> int:\n    n = \"hi\"\n    if (n := 1) > 0:\n        pass\n    return 0\n",
    )
    .expect_err("incompatible redefinition");
    assert_eq!(err.code, "T0023");
}

#[test]
fn a_failing_walrus_binding_in_a_function_while_test_propagates_t0023() {
    let err = check_source(
        "def f() -> int:\n    n = \"hi\"\n    while (n := 1) > 0:\n        pass\n    return 0\n",
    )
    .expect_err("incompatible redefinition");
    assert_eq!(err.code, "T0023");
}

// PEP 572 (#774): the comprehensive
// `a_walrus_nested_inside_every_remaining_container_shape_binds_every_name_at_module_scope`
// test above only exercises `collect_named_expr_bindings`'s recursive `?`
// propagation on its *success* path for every container shape -- each
// recursive `collect_named_expr_bindings(env, local_names, ..)?` call is its
// own coverage region distinct from the call's success path, and none of
// them is exercised when every nested walrus binds cleanly. These tests
// each nest a *failing* walrus (`n` already bound `str`, rebound `int`) one
// level inside a specific container shape so that shape's own recursive `?`
// takes its error branch instead. Every fixture here is expected to fail
// during `check`, before `check_and_resolve` ever reaches `monomorphize`.
#[test]
fn a_failing_walrus_nested_inside_a_named_exprs_own_value_propagates_t0023() {
    // Exercises the `NamedExpr` arm's own recursive call into `value`
    // (line ~996) failing, as opposed to `check_assignment` on the outer
    // name failing -- the outer name `m` binds cleanly; it's the *inner*
    // `n` rebind nested inside `m`'s value that fails.
    let err = check_source("n = \"hi\"\n(m := (n := 1))\n").expect_err("incompatible redefinition");
    assert_eq!(err.code, "T0023");
}

#[test]
fn a_type_error_inside_a_named_exprs_own_value_propagates_through_infer_expr_in() {
    // Exercises the `NamedExpr` arm's own `infer_expr_in(env, local_names,
    // value)` call (line ~997) failing on a `value` that contains no
    // nested walrus at all (so the preceding `collect_named_expr_bindings`
    // recursion succeeds trivially) but is otherwise ill-typed.
    let err = check_source("(m := 1 + \"x\")\n").expect_err("adding int and str is a type error");
    assert_ne!(err.code, "T0023");
}

#[test]
fn a_failing_walrus_nested_inside_a_call_argument_propagates_t0023() {
    let err = check_source("def f(v: int) -> int:\n    return v\nn = \"hi\"\nf((n := 1))\n")
        .expect_err("incompatible redefinition");
    assert_eq!(err.code, "T0023");
}

#[test]
fn a_failing_walrus_nested_inside_an_fstring_interpolation_propagates_t0023() {
    let err = check_source("n = \"hi\"\nf\"{(n := 1)}\"\n").expect_err("incompatible redefinition");
    assert_eq!(err.code, "T0023");
}

#[test]
fn a_failing_walrus_nested_inside_a_list_literal_element_propagates_t0023() {
    let err = check_source("n = \"hi\"\n[(n := 1)]\n").expect_err("incompatible redefinition");
    assert_eq!(err.code, "T0023");
}

#[test]
fn a_failing_walrus_nested_inside_a_subscript_base_propagates_t0023() {
    let err = check_source("n = \"hi\"\n(n := 1)[0]\n").expect_err("incompatible redefinition");
    assert_eq!(err.code, "T0023");
}

#[test]
fn a_failing_walrus_nested_inside_a_slice_base_propagates_t0023() {
    let err = check_source("n = \"hi\"\n(n := 1)[0:1]\n").expect_err("incompatible redefinition");
    assert_eq!(err.code, "T0023");
}

#[test]
fn a_failing_walrus_nested_inside_a_slice_bound_propagates_t0023() {
    let err = check_source("n = \"hi\"\nlst = [1, 2, 3]\nlst[(n := 1):]\n")
        .expect_err("incompatible redefinition");
    assert_eq!(err.code, "T0023");
}

#[test]
fn a_failing_walrus_nested_inside_a_dict_literal_key_propagates_t0023() {
    let err = check_source("n = \"hi\"\n{(n := 1): 2}\n").expect_err("incompatible redefinition");
    assert_eq!(err.code, "T0023");
}

#[test]
fn a_failing_walrus_nested_inside_a_dict_literal_value_propagates_t0023() {
    let err = check_source("n = \"hi\"\n{1: (n := 1)}\n").expect_err("incompatible redefinition");
    assert_eq!(err.code, "T0023");
}

#[test]
fn a_failing_walrus_nested_inside_a_dict_get_or_default_key_propagates_t0023() {
    let err = check_source("n = \"hi\"\ndct = {\"k\": 2}\ndct.get((n := 1), 0)\n")
        .expect_err("incompatible redefinition");
    assert_eq!(err.code, "T0023");
}

#[test]
fn a_failing_walrus_nested_inside_a_method_call_base_propagates_t0023() {
    let err = check_source(
        "class C:\n    def __init__(self, x: int):\n        self.x = x\n    def m(self) -> int:\n        return self.x\nn = \"hi\"\n(n := C(1)).m()\n",
    )
    .expect_err("incompatible redefinition");
    assert_eq!(err.code, "T0023");
}

#[test]
fn a_failing_walrus_nested_inside_a_method_call_argument_propagates_t0023() {
    let err = check_source(
        "class C:\n    def __init__(self, x: int):\n        self.x = x\n    def m(self, y: int) -> int:\n        return y\nn = \"hi\"\nc = C(1)\nc.m((n := 1))\n",
    )
    .expect_err("incompatible redefinition");
    assert_eq!(err.code, "T0023");
}

#[test]
fn a_failing_walrus_nested_inside_a_generic_class_instantiate_argument_propagates_t0023() {
    let err = check_source(
        "class Box[T]:\n    def __init__(self, x: T):\n        self.x = x\nn = \"hi\"\nBox[int]((n := 1))\n",
    )
    .expect_err("incompatible redefinition");
    assert_eq!(err.code, "T0023");
}

// PEP 572 (#774): `collect_named_expr_names_in_expr` (function-local-name
// detection, distinct from `collect_named_expr_bindings` above) has two
// arms not reached by the two existing function-body fixtures
// (`a_walrus_inside_a_unary_operand_..`/`a_walrus_inside_a_slice_bound_..`
// above): the `is_local` *true* branch (a second walrus reusing an
// already-registered name skips the push) and the
// `ListLiteral`/`SetLiteral`/`TupleLiteral` arm. Hand-built HIR, mirroring
// this file's other function-body walrus fixtures.
#[test]
fn a_walrus_reusing_an_already_local_name_in_a_function_body_skips_the_duplicate_push() {
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![HirItem::Function {
            name: "f".to_string(),
            params: vec![],
            return_ty: Ty::Int,
            body: vec![
                HirStmt::If {
                    test: HirExpr::Compare {
                        op: CmpOpKind::Lt,
                        left: Box::new(HirExpr::NamedExpr {
                            name: "x".to_string(),
                            value: Box::new(HirExpr::IntLiteral(1)),
                        }),
                        right: Box::new(HirExpr::NamedExpr {
                            name: "x".to_string(),
                            value: Box::new(HirExpr::IntLiteral(2)),
                        }),
                    },
                    body: vec![HirStmt::Return(Some(HirExpr::Name("x".to_string())))],
                    orelse: vec![],
                },
                HirStmt::Return(Some(HirExpr::IntLiteral(0))),
            ],
        }],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    assert!(check(&hir).is_ok(), "{:?}", check(&hir));
}

#[test]
fn a_walrus_inside_a_list_literal_test_in_a_function_body_is_a_local_name() {
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![HirItem::Function {
            name: "f".to_string(),
            params: vec![],
            return_ty: Ty::Int,
            body: vec![
                HirStmt::If {
                    test: HirExpr::ListLiteral(vec![HirExpr::NamedExpr {
                        name: "a".to_string(),
                        value: Box::new(HirExpr::IntLiteral(1)),
                    }]),
                    body: vec![HirStmt::Return(Some(HirExpr::Name("a".to_string())))],
                    orelse: vec![],
                },
                HirStmt::Return(Some(HirExpr::IntLiteral(0))),
            ],
        }],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    assert!(check(&hir).is_ok(), "{:?}", check(&hir));
}

// The `ListLiteral`/`SetLiteral`/`TupleLiteral` match arm above is one
// source line shared by all three variants, but each `|`-alternative still
// carries its own coverage region -- the test above only ever exercises
// the `ListLiteral` alternative. These two exercise the other two
// alternatives directly (not nested inside one another: a
// `set[..]`-typed tuple element is independently rejected by D-116 as
// "not compiled yet", so `TupleLiteral(vec![SetLiteral(..)])` is not a
// legal fixture here -- each container shape gets its own flat walrus).
#[test]
fn a_walrus_inside_a_set_literal_test_in_a_function_body_is_a_local_name() {
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![HirItem::Function {
            name: "f".to_string(),
            params: vec![],
            return_ty: Ty::Int,
            body: vec![
                HirStmt::If {
                    test: HirExpr::SetLiteral(vec![HirExpr::NamedExpr {
                        name: "b".to_string(),
                        value: Box::new(HirExpr::IntLiteral(1)),
                    }]),
                    body: vec![HirStmt::Return(Some(HirExpr::Name("b".to_string())))],
                    orelse: vec![],
                },
                HirStmt::Return(Some(HirExpr::IntLiteral(0))),
            ],
        }],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    assert!(check(&hir).is_ok(), "{:?}", check(&hir));
}

#[test]
fn a_walrus_inside_a_tuple_literal_test_in_a_function_body_is_a_local_name() {
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![HirItem::Function {
            name: "f".to_string(),
            params: vec![],
            return_ty: Ty::Int,
            body: vec![
                HirStmt::If {
                    test: HirExpr::TupleLiteral(vec![HirExpr::NamedExpr {
                        name: "c".to_string(),
                        value: Box::new(HirExpr::IntLiteral(1)),
                    }]),
                    body: vec![HirStmt::Return(Some(HirExpr::Name("c".to_string())))],
                    orelse: vec![],
                },
                HirStmt::Return(Some(HirExpr::IntLiteral(0))),
            ],
        }],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    assert!(check(&hir).is_ok(), "{:?}", check(&hir));
}

/// Part 2d of #1371: the same walk through an object-slot list display's
/// `|`-alternative. Only the empty-container pre-pass builds one from
/// source, never in a test position, so the fixture is hand-built.
#[test]
fn a_walrus_inside_an_object_list_test_in_a_function_body_is_a_local_name() {
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![HirItem::Function {
            name: "f".to_string(),
            params: vec![],
            return_ty: Ty::Int,
            body: vec![
                HirStmt::If {
                    test: HirExpr::ObjectList(vec![HirExpr::NamedExpr {
                        name: "d".to_string(),
                        value: Box::new(HirExpr::IntLiteral(1)),
                    }]),
                    body: vec![HirStmt::Return(Some(HirExpr::Name("d".to_string())))],
                    orelse: vec![],
                },
                HirStmt::Return(Some(HirExpr::IntLiteral(0))),
            ],
        }],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    assert!(check(&hir).is_ok(), "{:?}", check(&hir));
}

/// Part 2d of #1371: `rewrite_generic_calls_in_expr` walks an object-slot
/// list display's elements and types the display as the object.
#[test]
fn rewrite_generic_calls_in_expr_walks_an_object_list_display() {
    let mut env = Environment::new();
    let mut expr = HirExpr::ObjectList(vec![HirExpr::IntLiteral(1)]);
    let mut instantiations = Vec::new();
    let mut seen = HashSet::new();
    let ty =
        rewrite_generic_calls_in_expr(&mut env, &[], &mut expr, &mut instantiations, &mut seen)
            .unwrap();
    assert_eq!(ty, Ty::Object);
    assert!(instantiations.is_empty());
}
