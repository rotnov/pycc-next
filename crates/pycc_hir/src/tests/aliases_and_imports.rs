//! Type aliases, imports and import aliases, attribute access.

use super::*;

// -- D-135: `type` statement and legacy `TypeAlias` -------------------

#[test]
fn a_type_statement_resolves_the_alias_in_a_later_parameter_annotation() {
    let module = pycc_parser_test_helper::parse(
        "type IntAlias = int\n\
             def f(x: IntAlias) -> int:\n\
             \x20   return x\n",
    );
    let hir = lower_checked(&module).unwrap();

    // Zero HIR footprint: the `type` statement itself contributes no
    // `HirItem` -- the only item present is the function it fed, and its
    // `x` parameter resolved to `Ty::Int` through the alias.
    assert_eq!(
        hir.items,
        vec![HirItem::Function {
            name: "f".to_string(),
            params: vec![("x".to_string(), Ty::Int)],
            return_ty: Ty::Int,
            body: vec![HirStmt::Return(Some(HirExpr::Name("x".to_string())))],
        }]
    );
    assert_eq!(hir.type_aliases, vec![("IntAlias".to_string(), Ty::Int)]);
}

#[test]
fn a_legacy_type_alias_annotated_assignment_resolves_the_alias_the_same_way() {
    let module = pycc_parser_test_helper::parse(
        "IntAlias: TypeAlias = int\n\
             def f(x: IntAlias) -> int:\n\
             \x20   return x\n",
    );
    let hir = lower_checked(&module).unwrap();

    // Same zero-HIR-footprint contract as the `type` statement form:
    // the legacy annotated assignment contributes no `HirItem` either.
    assert_eq!(
        hir.items,
        vec![HirItem::Function {
            name: "f".to_string(),
            params: vec![("x".to_string(), Ty::Int)],
            return_ty: Ty::Int,
            body: vec![HirStmt::Return(Some(HirExpr::Name("x".to_string())))],
        }]
    );
    assert_eq!(hir.type_aliases, vec![("IntAlias".to_string(), Ty::Int)]);
}

#[test]
fn a_generic_type_alias_is_rejected_with_t0042() {
    let module = pycc_parser_test_helper::parse("type Alias[T] = list[T]\n");
    let diagnostic = lower_checked(&module).unwrap_err();

    assert_eq!(diagnostic.code, "T0042");
}

#[test]
fn a_generic_type_alias_t0042_span_points_at_the_type_statement_not_byte_zero() {
    // The `type` statement is deliberately not the first line, so a
    // regression back to a hardcoded `Span::new(0, 0)` would be caught:
    // byte 0 falls inside the preceding `def f() -> int:` line, not the
    // `type Alias[T] = int` statement this diagnostic is actually about.
    let source = "def f() -> int:\n    return 1\ntype Alias[T] = int\n";
    let type_stmt_start = source.find("type Alias").unwrap() as u32;
    let type_stmt_end = source.rfind('\n').unwrap() as u32;

    let module = pycc_parser_test_helper::parse(source);
    let diagnostic = lower_checked(&module).unwrap_err();

    assert_eq!(diagnostic.code, "T0042");
    assert_ne!(diagnostic.span, Some(Span::new(0, 0)));
    assert_eq!(
        diagnostic.span,
        Some(Span::new(type_stmt_start, type_stmt_end))
    );
}

#[test]
fn an_unresolvable_type_alias_rhs_falls_through_to_the_existing_c0001_diagnostic() {
    let module = pycc_parser_test_helper::parse("type Bad = NotARealType\n");
    let diagnostic = lower_checked(&module).unwrap_err();

    assert_eq!(diagnostic.code, "C0001");
}

#[test]
fn an_unresolvable_legacy_type_alias_rhs_also_falls_through_to_c0001() {
    let module = pycc_parser_test_helper::parse("Bad: TypeAlias = NotARealType\n");
    let diagnostic = lower_checked(&module).unwrap_err();

    assert_eq!(diagnostic.code, "C0001");
}

#[test]
fn a_bare_type_alias_annotation_with_no_value_is_not_treated_as_an_alias() {
    // `X: TypeAlias` with no assigned value can't define an alias (there
    // is no RHS to resolve) -- it falls through to the ordinary
    // `AnnAssign` lowering path, where `annotation_to_ty` rejects the
    // bare name `TypeAlias` itself with the same `C0001` catch-all as
    // any other unrecognized annotation name, and the alias table stays
    // empty.
    let module = pycc_parser_test_helper::parse("X: TypeAlias\n");
    let diagnostic = lower_checked(&module).unwrap_err();

    assert_eq!(diagnostic.code, "C0001");
}

#[test]
fn a_legacy_type_alias_annotation_on_a_non_name_target_is_not_treated_as_an_alias() {
    // Unlike a `type` statement's own target (always a bare name, see
    // `lower_type_alias_stmt`'s doc comment), a legacy `AnnAssign`
    // target can be an `Attribute`/`Subscript`, e.g. `obj.x: TypeAlias =
    // int`. `lower_legacy_type_alias_ann_assign` recognizes the `X:
    // TypeAlias = ...` shape only for a bare-name target and otherwise
    // falls through to the ordinary `AnnAssign` lowering path, which
    // rejects a non-name annotated-assignment target with the same
    // `C0001` catch-all it already uses for every other non-name
    // target -- the alias table stays empty either way.
    let module = pycc_parser_test_helper::parse("obj.x: TypeAlias = int\n");
    let diagnostic = lower_checked(&module).unwrap_err();

    assert_eq!(diagnostic.code, "C0001");
}

#[test]
fn import_math_binds_the_module_namespace() {
    let module = pycc_parser_test_helper::parse("import math\n");
    let hir = lower_checked(&module).expect("recognized stdlib import must lower");

    assert_eq!(
        hir.imports,
        vec![ImportBinding::Module {
            local_name: "math".to_string(),
            module: pycc_std::StdModule::Math,
        }]
    );
    assert!(hir.items.is_empty(), "a bare `import math` has no HirItem");
}

#[test]
fn import_cgi_is_c0001() {
    let module = pycc_parser_test_helper::parse("import cgi\n");
    let diagnostic = lower_checked(&module).unwrap_err();

    assert_eq!(diagnostic.code, "C0001");
}

#[test]
fn from_math_import_sqrt_and_pi_binds_both_names() {
    let module = pycc_parser_test_helper::parse("from math import sqrt, pi\n");
    let hir = lower_checked(&module).expect("both registered symbols must resolve");

    let sqrt_symbol = pycc_std::resolve_symbol(pycc_std::StdModule::Math, "sqrt")
        .expect("math.sqrt is registered");
    let pi_symbol =
        pycc_std::resolve_symbol(pycc_std::StdModule::Math, "pi").expect("math.pi is registered");
    assert_eq!(
        hir.imports,
        vec![
            ImportBinding::Symbol {
                local_name: "sqrt".to_string(),
                module: pycc_std::StdModule::Math,
                symbol: sqrt_symbol,
            },
            ImportBinding::Symbol {
                local_name: "pi".to_string(),
                module: pycc_std::StdModule::Math,
                symbol: pi_symbol,
            },
        ]
    );
}

#[test]
fn from_math_import_sqrt_and_unregistered_tan_is_c0002_not_a_partial_bind() {
    let module = pycc_parser_test_helper::parse("from math import sqrt, tan\n");
    let diagnostic = lower_checked(&module).unwrap_err();

    // Whole statement fails closed -- `sqrt` is not partially bound
    // even though it is itself registered.
    assert_eq!(diagnostic.code, "C0002");
}

#[test]
fn from_enum_import_enum_binds_enum_marker() {
    let module = pycc_parser_test_helper::parse("from enum import Enum\n");
    let hir = lower_checked(&module).expect("enum.Enum must resolve");

    let enum_symbol = pycc_std::resolve_symbol(pycc_std::StdModule::Enum, "Enum")
        .expect("enum.Enum is registered");
    assert_eq!(
        hir.imports,
        vec![ImportBinding::Symbol {
            local_name: "Enum".to_string(),
            module: pycc_std::StdModule::Enum,
            symbol: enum_symbol,
        }]
    );
}

// Part 1 of #883 (#962): `import <stdlib module> as <alias>` lowers and
// binds the alias; every `<alias>.<attr>` receiver resolves through that
// binding to the module's canonical spelling.

fn print_call(arg: HirExpr) -> Vec<HirItem> {
    vec![HirItem::TopLevelStmt(HirStmt::ExprStmt(HirExpr::Call {
        callee: "print".to_string(),
        args: vec![arg],
    }))]
}

#[test]
fn import_math_as_m_binds_the_alias_to_the_module() {
    let module = pycc_parser_test_helper::parse("import math as m\n");
    let hir = lower_checked(&module).expect("`import math as m` must lower");

    assert_eq!(
        hir.imports,
        vec![ImportBinding::Module {
            local_name: "m".to_string(),
            module: pycc_std::StdModule::Math,
        }]
    );
}

#[test]
fn aliased_math_sqrt_call_lowers_to_the_canonical_callee() {
    let module = pycc_parser_test_helper::parse("import math as m\nprint(m.sqrt(2.0))\n");
    let hir = lower_checked(&module).expect("m.sqrt(...) must lower");

    // The alias never reaches the HIR string: `pycc_mir`/`pycc_codegen`
    // match the literal `"math.sqrt"`.
    assert_eq!(
        hir.items,
        print_call(HirExpr::Call {
            callee: "math.sqrt".to_string(),
            args: vec![HirExpr::FloatLiteral(2.0)],
        })
    );
}

#[test]
fn aliased_math_pi_reference_lowers_to_the_canonical_name() {
    let module = pycc_parser_test_helper::parse("import math as m\nprint(m.pi)\n");
    let hir = lower_checked(&module).expect("m.pi must lower");

    assert_eq!(hir.items, print_call(HirExpr::Name("math.pi".to_string())));
}

#[test]
fn aliased_unregistered_call_names_the_alias_in_its_diagnostic() {
    let module = pycc_parser_test_helper::parse("import math as m\nm.tan(1.0)\n");
    let diagnostic = lower_checked(&module).unwrap_err();

    assert_eq!(diagnostic.code, "C0001");
    assert_eq!(
        diagnostic.message,
        "module `math` (imported as `m`) has no importable symbol named `tan`"
    );
}

#[test]
fn aliased_unregistered_attribute_names_the_alias_in_its_diagnostic() {
    let module = pycc_parser_test_helper::parse("import math as m\nprint(m.tau)\n");
    let diagnostic = lower_checked(&module).unwrap_err();

    assert_eq!(diagnostic.code, "C0001");
    assert_eq!(
        diagnostic.message,
        "module `math` (imported as `m`) has no attribute `tau`"
    );
}

#[test]
fn canonical_receiver_diagnostics_are_byte_stable() {
    // The "(imported as ...)" suffix is appended only when the spelling the
    // user wrote differs from the canonical one, so the pre-#962 wording
    // (and every `.expected.txt` fixture pinning it) is unchanged.
    let module = pycc_parser_test_helper::parse("import math\nmath.tan(1.0)\n");
    let diagnostic = lower_checked(&module).unwrap_err();
    assert_eq!(
        diagnostic.message,
        "module `math` has no importable symbol named `tan`"
    );

    let module = pycc_parser_test_helper::parse("import math\nprint(math.tau)\n");
    let diagnostic = lower_checked(&module).unwrap_err();
    assert_eq!(diagnostic.message, "module `math` has no attribute `tau`");
}

#[test]
fn the_last_binding_of_an_alias_wins() {
    // Python rebinding semantics: `m` is `enum` and then `math`.
    let module =
        pycc_parser_test_helper::parse("import enum as m\nimport math as m\nprint(m.sqrt(1.0))\n");
    let hir = lower_checked(&module).expect("the later `import math as m` wins");

    assert_eq!(
        hir.items,
        print_call(HirExpr::Call {
            callee: "math.sqrt".to_string(),
            args: vec![HirExpr::FloatLiteral(1.0)],
        })
    );
}

#[test]
fn an_alias_bound_in_the_import_table_beats_the_textual_spelling() {
    // `import enum as math`: the binding the user wrote wins over the
    // name's stdlib homonym, pinning `std_receiver`'s alias-first order.
    let module = pycc_parser_test_helper::parse("import enum as math\nprint(math.Enum)\n");
    let hir = lower_checked(&module).expect("math.Enum must resolve through the alias");

    assert_eq!(
        hir.items,
        print_call(HirExpr::Name("enum.Enum".to_string()))
    );
}

#[test]
fn an_aliased_non_math_module_resolves_through_the_alias() {
    let module = pycc_parser_test_helper::parse("import enum as e\nprint(e.Enum)\n");
    let hir = lower_checked(&module).expect("e.Enum must resolve");

    assert_eq!(
        hir.items,
        print_call(HirExpr::Name("enum.Enum".to_string()))
    );
}

#[test]
fn an_alias_is_visible_inside_a_function_body_lowered_after_the_import() {
    let module = pycc_parser_test_helper::parse(
        "import math as m\ndef f(x: float) -> float:\n    return m.sqrt(x)\n",
    );
    let hir = lower_checked(&module).expect("the alias is in scope for `f`");

    let HirItem::Function { body, .. } = &hir.items[0] else {
        panic!("expected a function item, got {:?}", hir.items[0]);
    };
    assert_eq!(
        *body,
        vec![HirStmt::Return(Some(HirExpr::Call {
            callee: "math.sqrt".to_string(),
            args: vec![HirExpr::Name("x".to_string())],
        }))]
    );
}

#[test]
fn an_alias_bound_after_the_using_function_is_not_visible_to_it() {
    // Source-order visibility, exactly like a class or a type alias: the
    // HIR lowers `m.sqrt(1.0)` as a generic `MethodCall` on an unbound
    // receiver (a recorded divergence from CPython's late binding), and it
    // is `pycc_types` that then reports the unbound name.
    let module = pycc_parser_test_helper::parse(
        "def f() -> float:\n    return m.sqrt(1.0)\nimport math as m\n",
    );
    let hir = lower_checked(&module).expect("the HIR itself lowers");

    let HirItem::Function { body, .. } = &hir.items[0] else {
        panic!("expected a function item, got {:?}", hir.items[0]);
    };
    assert_eq!(
        *body,
        vec![HirStmt::Return(Some(HirExpr::MethodCall {
            base: Box::new(HirExpr::Name("m".to_string())),
            method: "sqrt".to_string(),
            args: vec![HirExpr::FloatLiteral(1.0)],
        }))]
    );
}

#[test]
fn an_aliased_unregistered_module_is_still_c0001() {
    for source in ["import numpy as np\n", "import geometry as g\n"] {
        let module = pycc_parser_test_helper::parse(source);
        let diagnostic = lower_checked(&module).unwrap_err();
        assert_eq!(diagnostic.code, "C0001", "{source}");
        assert!(
            diagnostic.message.ends_with("is not supported yet"),
            "{source}: {}",
            diagnostic.message
        );
    }
    let module = pycc_parser_test_helper::parse("import numpy as np\n");
    assert_eq!(
        lower_checked(&module).unwrap_err().message,
        "import of module `numpy` is not supported yet"
    );
}

#[test]
fn an_aliased_unregistered_module_reports_exactly_one_diagnostic_for_its_uses() {
    // D-219 cascade suppression: the failed `import numpy as np` poisons
    // `np`, and `np.array(1)` lowers as a generic `MethodCall` on the
    // poisoned name, so the only diagnostic is the import's own `C0001`.
    let module = pycc_parser_test_helper::parse("import numpy as np\nprint(np.array(1))\n");
    let diagnostics = lower_all(&module).unwrap_err();
    assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
    assert_eq!(
        diagnostics[0].message,
        "import of module `numpy` is not supported yet"
    );
}

#[test]
fn folds_a_type_checking_guard_through_a_typing_alias() {
    // Part 1 of #883 (#962): `import typing as t` + `if t.TYPE_CHECKING:`
    // folds exactly like the bare `typing.TYPE_CHECKING` spelling.
    let module = pycc_parser_test_helper::parse(
        "import typing as t\nif t.TYPE_CHECKING:\n    import some_module_that_does_not_exist_at_runtime_or_compile_time\n",
    );
    let hir = lower_checked(&module).unwrap();
    assert_eq!(
        hir.items,
        vec![HirItem::TopLevelStmt(HirStmt::If {
            test: HirExpr::BoolLiteral(false),
            body: vec![],
            orelse: vec![],
        })]
    );
}

#[test]
fn folds_an_elif_type_checking_guard_through_a_typing_alias() {
    let module = pycc_parser_test_helper::parse(
        "import typing as t\nif False:\n    print(1)\nelif t.TYPE_CHECKING:\n    import some_module_that_does_not_exist_at_runtime_or_compile_time\nelse:\n    print(2)\n",
    );
    let hir = lower_checked(&module).unwrap();
    assert_eq!(
        hir.items,
        vec![HirItem::TopLevelStmt(HirStmt::If {
            test: HirExpr::BoolLiteral(false),
            body: vec![HirStmt::ExprStmt(HirExpr::Call {
                callee: "print".to_string(),
                args: vec![HirExpr::IntLiteral(1)],
            })],
            orelse: vec![HirStmt::If {
                test: HirExpr::BoolLiteral(false),
                body: vec![],
                orelse: vec![HirStmt::ExprStmt(HirExpr::Call {
                    callee: "print".to_string(),
                    args: vec![HirExpr::IntLiteral(2)],
                })],
            }],
        })]
    );
}

#[test]
fn a_type_checking_attribute_on_an_alias_of_another_module_is_not_folded() {
    // The guard folds only through an alias of `typing`; an alias of
    // `math` reaches the ordinary attribute path and its diagnostic.
    let module =
        pycc_parser_test_helper::parse("import math as t\nif t.TYPE_CHECKING:\n    print(1)\n");
    let diagnostic = lower_checked(&module).unwrap_err();
    assert_eq!(diagnostic.code, "C0001");
    assert_eq!(
        diagnostic.message,
        "module `math` (imported as `t`) has no attribute `TYPE_CHECKING`"
    );
}

#[test]
fn a_type_checking_attribute_on_a_rebound_typing_alias_is_not_folded() {
    // D-068 review of #962: the fold must honour last-binding-wins exactly
    // like every other stdlib receiver. After `import typing as t` is
    // rebound by `import enum as t`, the live `t` is `enum`, CPython raises
    // `AttributeError` on `t.TYPE_CHECKING`, and folding the guard would
    // silently discard that -- so the test must reach the ordinary
    // attribute path and its diagnostic instead.
    let module = pycc_parser_test_helper::parse(
        "import typing as t\nimport enum as t\nif t.TYPE_CHECKING:\n    print(1)\n",
    );
    let diagnostic = lower_checked(&module).unwrap_err();
    assert_eq!(diagnostic.code, "C0001");
    assert_eq!(
        diagnostic.message,
        "module `enum` (imported as `t`) has no attribute `TYPE_CHECKING`"
    );
}

#[test]
fn a_type_checking_attribute_on_typing_rebound_to_another_module_is_not_folded() {
    // The textual `typing.` spelling folds through `std_receiver`'s
    // fallback only while no alias rebinds `typing` itself.
    let module = pycc_parser_test_helper::parse(
        "import math as typing\nif typing.TYPE_CHECKING:\n    print(1)\n",
    );
    let diagnostic = lower_checked(&module).unwrap_err();
    assert_eq!(diagnostic.code, "C0001");
    assert_eq!(
        diagnostic.message,
        "module `math` (imported as `typing`) has no attribute `TYPE_CHECKING`"
    );
}

#[test]
fn a_class_named_like_an_import_alias_collides_with_it() {
    let module = pycc_parser_test_helper::parse(
        "import math as m\nclass m:\n    def __init__(self) -> None:\n        self.v = 1\n",
    );
    let diagnostic = lower_checked(&module).unwrap_err();
    assert_eq!(diagnostic.code, "C0001");
    assert!(
        diagnostic
            .message
            .contains("class `m` collides with an import of the same name"),
        "{}",
        diagnostic.message
    );
}

#[test]
fn from_math_import_sqrt_as_s_is_c0001() {
    let module = pycc_parser_test_helper::parse("from math import sqrt as s\n");
    let diagnostic = lower_checked(&module).unwrap_err();

    assert_eq!(diagnostic.code, "C0001");
}

#[test]
fn from_unregistered_module_import_is_c0001() {
    let module = pycc_parser_test_helper::parse("from os import path\n");
    let diagnostic = lower_checked(&module).unwrap_err();

    assert_eq!(diagnostic.code, "C0001");
}

#[test]
fn from_dot_import_x_is_c0001() {
    let module = pycc_parser_test_helper::parse("from . import x\n");
    let diagnostic = lower_checked(&module).unwrap_err();

    assert_eq!(diagnostic.code, "C0001");
}

#[test]
fn from_math_import_star_is_c0001() {
    let module = pycc_parser_test_helper::parse("from math import *\n");
    let diagnostic = lower_checked(&module).unwrap_err();

    assert_eq!(diagnostic.code, "C0001");
}

#[test]
fn import_two_modules_in_one_statement_is_c0001() {
    let module = pycc_parser_test_helper::parse("import math, os\n");
    let diagnostic = lower_checked(&module).unwrap_err();

    assert_eq!(diagnostic.code, "C0001");
}

#[test]
fn math_sqrt_call_lowers_to_a_qualified_callee() {
    let module = pycc_parser_test_helper::parse("import math\nprint(math.sqrt(2.0))\n");
    let hir = lower_checked(&module).expect("math.sqrt(...) must lower");

    assert_eq!(
        hir.items,
        vec![HirItem::TopLevelStmt(HirStmt::ExprStmt(HirExpr::Call {
            callee: "print".to_string(),
            args: vec![HirExpr::Call {
                callee: "math.sqrt".to_string(),
                args: vec![HirExpr::FloatLiteral(2.0)],
            }],
        }))]
    );
}

#[test]
fn math_pi_bare_reference_lowers_to_a_qualified_name() {
    let module = pycc_parser_test_helper::parse("import math\nprint(math.pi)\n");
    let hir = lower_checked(&module).expect("math.pi must lower");

    assert_eq!(
        hir.items,
        vec![HirItem::TopLevelStmt(HirStmt::ExprStmt(HirExpr::Call {
            callee: "print".to_string(),
            args: vec![HirExpr::Name("math.pi".to_string())],
        }))]
    );
}

#[test]
fn math_tan_call_is_unsupported_since_it_is_not_registered() {
    let module = pycc_parser_test_helper::parse("import math\nmath.tan(1.0)\n");
    let diagnostic = lower_checked(&module).unwrap_err();

    assert_eq!(diagnostic.code, "C0001");
}

#[test]
fn math_tan_bare_reference_is_unsupported_since_it_is_not_registered() {
    let module = pycc_parser_test_helper::parse("import math\nprint(math.tan)\n");
    let diagnostic = lower_checked(&module).unwrap_err();

    assert_eq!(diagnostic.code, "C0001");
}

#[test]
fn os_path_bare_attribute_access_lowers_to_a_generic_attr_get() {
    // No `import os` here on purpose -- `os` isn't a registered
    // `pycc_std` module at all, so this exercises the "receiver name
    // does not resolve to a stdlib module" branch directly, distinct
    // from `math_tan_bare_reference_is_unsupported_since_it_is_not_registered`
    // above (recognized module, unregistered attribute -- which stays
    // `C0001`, see that test and `math_tan_call_is_unsupported_since_it_is_not_registered`'s
    // own updated comment). Before D-154 (Part 1 of #375), a receiver
    // that didn't resolve to a stdlib module made *any* attribute
    // access `C0001` unconditionally; D-154 adds a generic
    // instance-attribute-read shape (`HirExpr::AttrGet`) that this now
    // falls through to instead, deferring real rejection (`os` is
    // never assigned, so it isn't a class instance either) to
    // `pycc_types`.
    let module = pycc_parser_test_helper::parse("print(os.path)\n");
    let hir = lower_checked(&module).unwrap();
    assert_eq!(
        hir.items,
        vec![HirItem::TopLevelStmt(HirStmt::ExprStmt(HirExpr::Call {
            callee: "print".to_string(),
            args: vec![HirExpr::AttrGet {
                base: Box::new(HirExpr::Name("os".to_string())),
                attr: "path".to_string(),
            }],
        }))]
    );
}

#[test]
fn attribute_access_on_a_non_name_receiver_lowers_to_a_generic_attr_get() {
    // Same D-154 widening as the `os.path` test above, exercised on a
    // receiver that isn't even a bare name (a list literal) -- `base`
    // is lowered generically regardless of its own shape.
    let module = pycc_parser_test_helper::parse("print([1, 2].sqrt)\n");
    let hir = lower_checked(&module).unwrap();
    assert_eq!(
        hir.items,
        vec![HirItem::TopLevelStmt(HirStmt::ExprStmt(HirExpr::Call {
            callee: "print".to_string(),
            args: vec![HirExpr::AttrGet {
                base: Box::new(HirExpr::ListLiteral(vec![
                    HirExpr::IntLiteral(1),
                    HirExpr::IntLiteral(2)
                ])),
                attr: "sqrt".to_string(),
            }],
        }))]
    );
}

#[test]
fn math_sqrt_call_propagates_an_unsupported_argument_expression() {
    // Exercises the `?` inside the stdlib-call arm's own argument
    // lowering (`call.arguments.args.iter().map(lower_expr).collect()`)
    // taking its error path, as opposed to every other stdlib-call test
    // above, which only exercises the success path.
    let module = pycc_parser_test_helper::parse("import math\nmath.sqrt(1j)\n");
    let diagnostic = lower_checked(&module).unwrap_err();

    assert_eq!(diagnostic.code, "C0001");
}

#[test]
fn method_call_propagates_an_unsupported_base_expression() {
    // Exercises the `?` inside `MethodCall`'s own `base` lowering
    // (D-154), as opposed to `method_call_on_a_non_name_receiver_lowers_to_a_generic_method_call`
    // below, which only exercises the success path for a non-name base.
    let module = pycc_parser_test_helper::parse("(1j).bump()\n");
    let diagnostic = lower_checked(&module).unwrap_err();

    assert_eq!(diagnostic.code, "C0001");
}

#[test]
fn method_call_propagates_an_unsupported_argument_expression() {
    // Exercises the `?` inside `MethodCall`'s own argument lowering
    // (D-154), mirroring `math_sqrt_call_propagates_an_unsupported_argument_expression`
    // above.
    let module = pycc_parser_test_helper::parse("p.bump(1j)\n");
    let diagnostic = lower_checked(&module).unwrap_err();

    assert_eq!(diagnostic.code, "C0001");
}

#[test]
fn attr_get_propagates_an_unsupported_base_expression() {
    // Exercises the `?` inside `AttrGet`'s own `base` lowering (D-154).
    let module = pycc_parser_test_helper::parse("(1j).x\n");
    let diagnostic = lower_checked(&module).unwrap_err();

    assert_eq!(diagnostic.code, "C0001");
}

#[test]
fn method_call_on_a_non_name_receiver_lowers_to_a_generic_method_call() {
    // Exercises the call-position stdlib-intrinsic branch's own
    // `Expr::Name(receiver)` guard failing (as opposed to the bare
    // attribute-access arm's analogous guard above) -- before D-154
    // (Part 1 of #375) this was unconditionally `C0001`; now it falls
    // through to the generic `HirExpr::MethodCall` shape instead, same
    // as `os_path_bare_attribute_access_lowers_to_a_generic_attr_get`
    // above.
    let module = pycc_parser_test_helper::parse("[1, 2].sqrt()\n");
    let hir = lower_checked(&module).unwrap();
    assert_eq!(
        hir.items,
        vec![HirItem::TopLevelStmt(HirStmt::ExprStmt(
            HirExpr::MethodCall {
                base: Box::new(HirExpr::ListLiteral(vec![
                    HirExpr::IntLiteral(1),
                    HirExpr::IntLiteral(2)
                ])),
                method: "sqrt".to_string(),
                args: vec![],
            }
        ))]
    );
}

#[test]
fn import_inside_a_function_body_is_c0001() {
    // The module-level side-table is populated only by `module::lower_all`'s
    // top-level loop (mirroring `type_aliases`); a nested import still
    // reaches plain `lower_stmt`, which has no arm for `Stmt::Import`.
    let module =
        pycc_parser_test_helper::parse("def f() -> None:\n    import math\n    return None\n");
    let diagnostic = lower_checked(&module).unwrap_err();

    assert_eq!(diagnostic.code, "C0001");
}
