//! Method signatures, receivers and `Self`/class-name annotations.

use super::*;

#[test]
fn a_class_without_init_gets_a_synthesized_zero_argument_constructor() {
    // #912 (D-225): a class that declares no `__init__` and inherits
    // none is no longer `C0001` -- `ensure_init` synthesizes
    // `def __init__(self) -> None: pass` for it.
    let module = crate::pycc_parser_test_helper::parse(
        "class C:\n    def foo(self) -> None:\n        return\n",
    );
    let hir = lower_checked(&module).unwrap();
    let (_, class_def) = hir
        .class_defs
        .iter()
        .find(|(name, _)| name == "C")
        .expect("class `C` is lowered");
    assert_eq!(
        class_def
            .methods
            .iter()
            .find(|(mn, _)| mn == "__init__")
            .map(|(_, mangled)| mangled.as_str()),
        Some("C.__init__"),
        "the synthesized constructor is registered under the class's own name"
    );
    let init = hir
        .items
        .iter()
        .find_map(|item| match item {
            HirItem::Function {
                name, params, body, ..
            } if name == "C.__init__" => Some((params.clone(), body.clone())),
            _ => None,
        })
        .expect("the synthesized `C.__init__` item is emitted");
    assert_eq!(init.0.len(), 1, "only `self` is a parameter: {:?}", init.0);
    assert_eq!(init.0[0].0, "self");
    assert!(init.1.is_empty(), "the body is empty: {:?}", init.1);
}

// -- lower_method: method-shape checks ----------------------------------

#[test]
fn a_positional_only_method_parameter_lowers_successfully() {
    // PEP 570 (#383): positional-only parameters on methods are now
    // lowered. `self` remains the first parameter; posonlyargs follow
    // it, before ordinary `args`. The class also has a regular method
    // `foo` to exercise the `find_map` None branch (items before the
    // target function that don't match).
    let module = crate::pycc_parser_test_helper::parse(
        "class C:\n    def foo(self) -> None:\n        return\n    def __init__(self, x: int, /, y: int) -> None:\n        return\n",
    );
    let hir = lower_checked(&module).unwrap();
    // Find the __init__ function item and verify its params.
    let init = hir
        .items
        .iter()
        .filter_map(|item| match item {
            HirItem::Function { name, params, .. } if name == "C.__init__" => {
                Some(params.clone())
            }
            _ => None,
        })
        .next()
        .expect("C.__init__ must be lowered");
    assert_eq!(
        init,
        vec![
            ("self".to_string(), Ty::Instance(Box::new("C".to_string()))),
            ("x".to_string(), Ty::Int),
            ("y".to_string(), Ty::Int),
        ]
    );
}

#[test]
fn a_positional_only_classmethod_parameter_lowers_successfully() {
    // PEP 570 (#383): positional-only parameters on `@classmethod` are
    // now lowered. `cls` remains the first parameter; posonlyargs follow
    // it, before ordinary `args`. The mangled name uses a `.classmethod`
    // suffix (#436).
    let module = crate::pycc_parser_test_helper::parse(
        "class C:\n    def __init__(self) -> None:\n        return\n    @classmethod\n    def m(cls, x: int, /, y: int) -> None:\n        return\n",
    );
    let hir = lower_checked(&module).unwrap();
    let m = hir
        .items
        .iter()
        .filter_map(|item| match item {
            HirItem::Function { name, params, .. } if name == "C.m.classmethod" => {
                Some(params.clone())
            }
            _ => None,
        })
        .next()
        .expect("C.m.classmethod must be lowered");
    assert_eq!(
        m,
        vec![
            ("cls".to_string(), Ty::Instance(Box::new("C".to_string()))),
            ("x".to_string(), Ty::Int),
            ("y".to_string(), Ty::Int),
        ]
    );
}

#[test]
fn a_positional_only_staticmethod_parameter_lowers_successfully() {
    // PEP 570 (#383): positional-only parameters on `@staticmethod` are
    // now lowered. For static methods, posonlyargs come first (no
    // implicit `self`/`cls`), then ordinary `args`. The mangled name
    // uses a `.static` suffix (#436).
    let module = crate::pycc_parser_test_helper::parse(
        "class C:\n    def __init__(self) -> None:\n        return\n    @staticmethod\n    def m(x: int, /, y: int) -> None:\n        return\n",
    );
    let hir = lower_checked(&module).unwrap();
    let m = hir
        .items
        .iter()
        .filter_map(|item| match item {
            HirItem::Function { name, params, .. } if name == "C.m.static" => {
                Some(params.clone())
            }
            _ => None,
        })
        .next()
        .expect("C.m.static must be lowered");
    assert_eq!(
        m,
        vec![("x".to_string(), Ty::Int), ("y".to_string(), Ty::Int),]
    );
}

#[test]
fn a_positional_only_method_parameter_may_have_a_default() {
    // PEP 570 (#383) with the method part of #1140: a defaulted
    // positional-only parameter lowers on every method kind, exactly as
    // on a module-level `def` (`class::method_tests` pins what is
    // recorded).
    for source in [
        "class C:\n    def __init__(self, x: int = 0, /) -> None:\n        return\n",
        "class C:\n    def __init__(self) -> None:\n        return\n    @classmethod\n    def m(cls, x: int = 0, /) -> None:\n        return\n",
        "class C:\n    def __init__(self) -> None:\n        return\n    @staticmethod\n    def m(x: int = 0, /) -> None:\n        return\n",
    ] {
        let module = lower_ok(source);
        assert_eq!(module.class_defs[0].1.method_defaults.len(), 1, "{source}");
    }
}

#[test]
fn a_vararg_method_parameter_is_unsupported() {
    assert_c0001("class C:\n    def __init__(self, *args) -> None:\n        return\n");
}

#[test]
fn a_keyword_only_method_parameter_is_unsupported() {
    assert_c0001("class C:\n    def __init__(self, *, x: int) -> None:\n        return\n");
}

#[test]
fn a_kwarg_method_parameter_is_unsupported() {
    assert_c0001("class C:\n    def __init__(self, **kwargs) -> None:\n        return\n");
}

#[test]
fn a_method_with_no_parameters_at_all_is_unsupported() {
    assert_c0001("class C:\n    def __init__() -> None:\n        return\n");
}

#[test]
fn a_method_whose_receiver_is_not_spelled_self_is_accepted_and_aliased() {
    // #1181 removed the rule this test used to assert. The receiver is
    // lowered under the canonical name `self`, and the body opens with
    // the `this = self` alias -- the two facts the whole change rests on,
    // asserted here on the HIR directly rather than only through the CLI
    // (`tests/issue_1181_receiver_spelling.rs` covers the behavior).
    let module = lower_ok("class C:\n    def __init__(this) -> None:\n        return\n");
    assert_eq!(
        module.items,
        vec![crate::HirItem::Function {
            name: "C.__init__".to_string(),
            params: vec![("self".to_string(), Ty::Instance(Box::new("C".to_string())))],
            return_ty: Ty::None,
            body: vec![
                HirStmt::Assign {
                    target: "this".to_string(),
                    value: crate::HirExpr::Name("self".to_string()),
                },
                HirStmt::Return(None),
            ],
        }],
        "the receiver is lowered canonically and the body opens with its alias"
    );
}

#[test]
fn a_method_with_no_receiver_at_all_is_unsupported() {
    assert_c0001("class C:\n    def __init__() -> None:\n        return\n");
}

#[test]
fn a_self_parameter_with_a_default_value_is_unsupported() {
    assert_c0001("class C:\n    def __init__(self=None) -> None:\n        return\n");
}

#[test]
fn an_annotated_self_parameter_is_unsupported() {
    assert_c0001("class C:\n    def __init__(self: C) -> None:\n        return\n");
}

#[test]
fn an_init_parameter_without_an_annotation_is_inferred_in_a_native_module() {
    // A native module types an unannotated `__init__` parameter from the
    // program's constructor calls (`pycc_types`' `init_params`); an
    // `--ext` module keeps the `T0001` (`method_tests`).
    let module = crate::pycc_parser_test_helper::parse(
        "class C:\n    def __init__(self, x) -> None:\n        return\n",
    );
    let hir = lower_checked(&module).unwrap();
    let crate::HirItem::Function { params, .. } = &hir.items[0] else {
        panic!("the `__init__` function")
    };
    assert_eq!(params[1].1, crate::Ty::Infer);
}

#[test]
fn a_private_method_parameter_without_an_annotation_is_still_permitted() {
    // Contrast with the test above: an ordinary (non-`__init__`)
    // private method still follows the plain D-038 rule (an unannotated
    // parameter is only rejected for a *public* name).
    let hir = lower_ok(
        "class C:\n    def __init__(self, x: int) -> None:\n        self.x = x\n    def _helper(self, y) -> None:\n        return\n",
    );
    assert_eq!(hir.class_defs.len(), 1);
}

#[test]
fn a_method_with_an_unsupported_return_annotation_propagates_the_error() {
    // Exercises `lower_method`'s own `?` on
    // `crate::lower_return_annotation`'s error path -- distinct from
    // every other method-shape test above, which only ever exercise a
    // parameter-side rejection.
    assert_c0001(
        "class C:\n    def __init__(self) -> None:\n        return\n    def bump(self) -> Frobnicate:\n        return\n",
    );
}

#[test]
fn a_method_signature_and_self_ty_lower_correctly() {
    let hir = lower_ok(
        "class Point:\n    def __init__(self, x: int, y: int) -> None:\n        self.x = x\n        self.y = y\n",
    );
    assert_eq!(hir.class_defs.len(), 1);
    let (name, class_def) = &hir.class_defs[0];
    assert_eq!(name, "Point");
    assert_eq!(
        *class_def,
        HirClassDef {
            class_attrs: Vec::new(),
            exception_type_tag: None,
            name: "Point".to_string(),
            bases: Vec::new(),
            mro: vec!["Point".to_string()],
            attrs: vec![("x".to_string(), Ty::Int), ("y".to_string(), Ty::Int),],
            methods: vec![("__init__".to_string(), "Point.__init__".to_string())],
            properties: Vec::new(),
            type_param: None,
            static_methods: Vec::new(),
            class_methods: Vec::new(),
            is_enum: false,
            implicit_object_init: false,
            method_defaults: Vec::new(),
            enum_members: Vec::new(),
            is_dataclass: false,
            dataclass_fields: Vec::new(),
            is_protocol: false,
            runtime_checkable: false,
            protocol_members: Vec::new(),
            abstract_methods: Vec::new(),
            is_abstract: false,
        }
    );
    // Direct value comparison, not a `let PATTERN = .. else { panic!(..) }`
    // destructure -- this crate's own established coverage-gate
    // convention (see `pycc_hir::lib.rs`'s
    // `re_exported_grammar_types_resolve_and_have_the_expected_shape`-
    // adjacent precedent): a hand-written panic arm never taken on the
    // happy path is a permanently uncovered region under D-014's
    // 100%-region gate.
    assert_eq!(
        hir.items[0],
        HirItem::Function {
            name: "Point.__init__".to_string(),
            params: vec![
                (
                    "self".to_string(),
                    Ty::Instance(Box::new("Point".to_string()))
                ),
                ("x".to_string(), Ty::Int),
                ("y".to_string(), Ty::Int),
            ],
            return_ty: Ty::None,
            body: vec![
                HirStmt::AttrSet {
                    base: HirExpr::Name("self".to_string()),
                    attr: "x".to_string(),
                    value: HirExpr::Name("x".to_string()),
                },
                HirStmt::AttrSet {
                    base: HirExpr::Name("self".to_string()),
                    attr: "y".to_string(),
                    value: HirExpr::Name("y".to_string()),
                },
            ],
        }
    );
}

// -- Self / generic / override / init_subclass -------------------------

// PEP 673 (#387 Part 1): `Self` as a method return-type annotation
// resolves to the class's own instance type at HIR-lowering time.
#[test]
fn self_return_annotation_resolves_to_class_instance() {
    let hir = lower_ok(
        "class C:\n    def __init__(self) -> None:\n        return\n    def clone(self) -> Self:\n        return self\n",
    );
    assert_eq!(hir.class_defs.len(), 1);
    // The `clone` method should be lowered as a function with return
    // type `Ty::Instance("C")`.
    let clone = hir.items.iter().find_map(|item| match item {
        HirItem::Function {
            name, return_ty, ..
        } if name == "C.clone" => Some(return_ty),
        _ => None,
    });
    assert_eq!(
        clone,
        Some(&Ty::Instance(Box::new("C".to_string()))),
        "Self return annotation should resolve to Ty::Instance(\"C\")"
    );
}

// PEP 673 (#387 Part 1): `Self` as a method parameter annotation also
// resolves to the class's own instance type.
#[test]
fn self_param_annotation_resolves_to_class_instance() {
    let hir = lower_ok(
        "class C:\n    def __init__(self, value: int) -> None:\n        self.value = value\n    def merge(self, other: Self) -> None:\n        self.value = other.value\n",
    );
    // The `merge` method's `other` parameter should be typed
    // `Ty::Instance("C")`.
    let merge = hir.items.iter().find_map(|item| match item {
        HirItem::Function { name, params, .. } if name == "C.merge" => Some(params.clone()),
        _ => None,
    });
    let merge = merge.expect("C.merge should exist");
    // params[0] is `self`, params[1] is `other`
    assert_eq!(merge[1].1, Ty::Instance(Box::new("C".to_string())));
}

// PEP 649/749 (#387 Part 2): self-referential deferred annotations. A
// class's method may use the class's own name as a parameter type
// annotation, even though the class is not fully defined at that point
// in source.
#[test]
fn class_name_in_own_method_annotation_resolves_to_instance() {
    let hir = lower_ok(
        "class Node:\n    def __init__(self, value: int) -> None:\n        self.value = value\n    def update(self, other: Node) -> None:\n        self.value = other.value\n",
    );
    let update = hir.items.iter().find_map(|item| match item {
        HirItem::Function { name, params, .. } if name == "Node.update" => Some(params.clone()),
        _ => None,
    });
    let update = update.expect("Node.update should exist");
    // params[1] is `other` with type `Ty::Instance("Node")`
    assert_eq!(update[1].1, Ty::Instance(Box::new("Node".to_string())));
}

// PEP 649/749 (#387 Part 2): the class name also works as a return
// type annotation in its own methods.
#[test]
fn class_name_as_own_return_annotation_resolves_to_instance() {
    let hir = lower_ok(
        "class Builder:\n    def __init__(self) -> None:\n        self.count = 0\n    def clone(self) -> Builder:\n        return self\n",
    );
    let clone = hir.items.iter().find_map(|item| match item {
        HirItem::Function {
            name, return_ty, ..
        } if name == "Builder.clone" => Some(return_ty.clone()),
        _ => None,
    });
    assert_eq!(clone, Some(Ty::Instance(Box::new("Builder".to_string()))),);
}

// PEP 649/749 (#387 Part 2, Bug 4 fix): a local `AnnAssign` *inside* a
// method body (e.g. `other: Node = self`) must also resolve the class
// name to `Ty::Instance`. Before the fix, `lower_body` was called with
// `class_name=None` from `lower_method`, so the class name was
// unresolvable in statement-body annotations (C0001).
#[test]
fn class_name_in_method_body_local_annotation_resolves_to_instance() {
    let hir = lower_ok(
        "class Node:\n    def __init__(self) -> None:\n        self.x = 0\n    def next(self) -> Node:\n        other: Node = self\n        return other\n",
    );
    let next = hir.items.iter().find_map(|item| match item {
        HirItem::Function { name, body, .. } if name == "Node.next" => Some(body.clone()),
        _ => None,
    });
    let next = next.expect("Node.next should exist");
    // body[0] is the `other: Node = self` AnnAssign. Use `matches!` with
    // a guard (same pattern as `check_and_resolve_monomorphizes_a_generic_
    // class_with_self_typed_method` in pycc_types) to avoid an uncovered
    // `panic!`/`unreachable!` arm under the 100%-region coverage gate.
    assert!(matches!(
        &next[0],
        HirStmt::AnnAssign { annotation, .. }
        if annotation == &Ty::Instance(Box::new("Node".to_string()))
    ));
}
