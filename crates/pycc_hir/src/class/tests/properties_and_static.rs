//! `@property`, `@staticmethod` and `@classmethod` lowering.

use super::*;

// -- @property lowering (#377) ------------------------------------------

#[test]
fn a_property_getter_lowers_into_the_property_table() {
    let hir = lower_ok(
        "class C:\n    def __init__(self) -> None:\n        self._x = 0\n    @property\n    def x(self) -> int:\n        return self._x\n",
    );
    let (_, class_def) = &hir.class_defs[0];
    assert_eq!(class_def.properties.len(), 1);
    assert_eq!(
        class_def.properties[0],
        crate::PropertyDef {
            name: "x".to_string(),
            getter: "C.x".to_string(),
            setter: None,
        }
    );
    // The getter is NOT in the methods table (accessed via attribute
    // syntax, not method-call syntax).
    assert!(!class_def.methods.iter().any(|(name, _)| name == "x"));
}

#[test]
fn a_property_getter_and_setter_lower_into_one_property_entry() {
    let hir = lower_ok(
        "class C:\n    def __init__(self) -> None:\n        self._x = 0\n    @property\n    def x(self) -> int:\n        return self._x\n    @x.setter\n    def x(self, v: int) -> None:\n        self._x = v\n",
    );
    let (_, class_def) = &hir.class_defs[0];
    assert_eq!(class_def.properties.len(), 1);
    assert_eq!(
        class_def.properties[0],
        crate::PropertyDef {
            name: "x".to_string(),
            getter: "C.x".to_string(),
            setter: Some("C.x.setter".to_string()),
        }
    );
}

#[test]
fn a_property_getter_function_is_emitted_with_the_mangled_name() {
    let hir = lower_ok(
        "class C:\n    def __init__(self) -> None:\n        self._x = 0\n    @property\n    def x(self) -> int:\n        return self._x\n",
    );
    assert!(
        hir.items
            .iter()
            .any(|item| matches!(item, HirItem::Function { name, .. } if name == "C.x")),
        "getter function `C.x` should be in items"
    );
}

#[test]
fn a_property_setter_function_is_emitted_with_the_setter_mangled_name() {
    let hir = lower_ok(
        "class C:\n    def __init__(self) -> None:\n        self._x = 0\n    @property\n    def x(self) -> int:\n        return self._x\n    @x.setter\n    def x(self, v: int) -> None:\n        self._x = v\n",
    );
    assert!(
        hir.items
            .iter()
            .any(|item| matches!(item, HirItem::Function { name, .. } if name == "C.x.setter")),
        "setter function `C.x.setter` should be in items"
    );
}

#[test]
fn an_unrecognized_method_decorator_is_still_rejected() {
    assert_c0001(
        "class C:\n    def __init__(self) -> None:\n        return\n    @foo\n    def f(self) -> None:\n        return\n",
    );
}

#[test]
fn multiple_decorators_on_a_method_are_rejected() {
    assert_c0001(
        "class C:\n    def __init__(self) -> None:\n        return\n    @property\n    @foo\n    def f(self) -> None:\n        return\n",
    );
}

#[test]
fn a_property_getter_with_extra_parameters_is_rejected() {
    assert_c0001(
        "class C:\n    def __init__(self) -> None:\n        return\n    @property\n    def x(self, extra: int) -> int:\n        return 0\n",
    );
}

#[test]
fn a_property_setter_with_no_value_parameter_is_rejected() {
    assert_c0001(
        "class C:\n    def __init__(self) -> None:\n        return\n    @property\n    def x(self) -> int:\n        return 0\n    @x.setter\n    def x(self) -> None:\n        return\n",
    );
}

#[test]
fn a_property_setter_with_two_value_parameters_is_rejected() {
    assert_c0001(
        "class C:\n    def __init__(self) -> None:\n        return\n    @property\n    def x(self) -> int:\n        return 0\n    @x.setter\n    def x(self, a: int, b: int) -> None:\n        return\n",
    );
}

#[test]
fn a_setter_without_a_preceding_getter_is_rejected() {
    let module = crate::pycc_parser_test_helper::parse(
        "class C:\n    def __init__(self) -> None:\n        return\n    @x.setter\n    def x(self, v: int) -> None:\n        return\n",
    );
    let diagnostic = lower_checked(&module).unwrap_err();
    assert_eq!(diagnostic.code, "C0001");
    assert!(
        diagnostic
            .message
            .contains("requires a preceding `@property` getter"),
        "unexpected message: {}",
        diagnostic.message
    );
}

#[test]
fn a_duplicate_property_getter_is_rejected() {
    let module = crate::pycc_parser_test_helper::parse(
        "class C:\n    def __init__(self) -> None:\n        return\n    @property\n    def x(self) -> int:\n        return 0\n    @property\n    def x(self) -> int:\n        return 1\n",
    );
    let diagnostic = lower_checked(&module).unwrap_err();
    assert_eq!(diagnostic.code, "C0001");
    assert!(
        diagnostic.message.contains("already defined"),
        "unexpected message: {}",
        diagnostic.message
    );
}

#[test]
fn a_duplicate_property_setter_is_rejected() {
    let module = crate::pycc_parser_test_helper::parse(
        "class C:\n    def __init__(self) -> None:\n        return\n    @property\n    def x(self) -> int:\n        return 0\n    @x.setter\n    def x(self, v: int) -> None:\n        return\n    @x.setter\n    def x(self, v: int) -> None:\n        return\n",
    );
    let diagnostic = lower_checked(&module).unwrap_err();
    assert_eq!(diagnostic.code, "C0001");
    assert!(
        diagnostic.message.contains("already defined"),
        "unexpected message: {}",
        diagnostic.message
    );
}

#[test]
fn a_setter_decorator_name_not_matching_the_method_name_is_rejected() {
    let module = crate::pycc_parser_test_helper::parse(
        "class C:\n    def __init__(self) -> None:\n        return\n    @property\n    def x(self) -> int:\n        return 0\n    @x.setter\n    def y(self, v: int) -> None:\n        return\n",
    );
    let diagnostic = lower_checked(&module).unwrap_err();
    assert_eq!(diagnostic.code, "C0001");
    assert!(
        diagnostic
            .message
            .contains("must decorate a method named `x`"),
        "unexpected message: {}",
        diagnostic.message
    );
}

#[test]
fn a_non_setter_attribute_decorator_is_rejected() {
    assert_c0001(
        "class C:\n    def __init__(self) -> None:\n        return\n    @x.deleter\n    def x(self) -> None:\n        return\n",
    );
}

#[test]
fn a_call_shaped_decorator_is_rejected() {
    assert_c0001(
        "class C:\n    def __init__(self) -> None:\n        return\n    @property()\n    def x(self) -> int:\n        return 0\n",
    );
}

#[test]
fn a_setter_decorator_with_a_chained_attribute_base_is_rejected() {
    // `@a.b.setter` -- the decorator expression is an attribute access
    // (`(a.b).setter`) whose own base is *not* a bare `Expr::Name`
    // (it's `Expr::Attribute(Name("a"), "b")`). This exercises
    // `classify_decorator`'s `let Expr::Name(base_name) =
    // attr.value.as_ref() else { ... }` rejection arm -- distinct from
    // `a_non_setter_attribute_decorator_is_rejected` above, which
    // exercises the `attr.attr != "setter"` arm, and from
    // `a_setter_decorator_name_not_matching_the_method_name_is_rejected`,
    // which exercises the name-mismatch arm.
    assert_c0001(
        "class C:\n    def __init__(self) -> None:\n        return\n    @property\n    def x(self) -> int:\n        return 0\n    @a.b.setter\n    def x(self, v: int) -> None:\n        return\n",
    );
}

// -- #436: @staticmethod / @classmethod lowering ------------------------

#[test]
fn a_static_method_lowers_into_the_static_methods_table() {
    let hir = lower_ok(
        "class C:\n    def __init__(self) -> None:\n        return\n    @staticmethod\n    def create(x: int) -> int:\n        return x\n",
    );
    let (_, class_def) = &hir.class_defs[0];
    assert_eq!(
        class_def.static_methods,
        vec![("create".to_string(), "C.create.static".to_string())]
    );
    // The static method is NOT in the methods table.
    assert!(!class_def.methods.iter().any(|(name, _)| name == "create"));
}

#[test]
fn a_class_method_lowers_into_the_class_methods_table() {
    let hir = lower_ok(
        "class C:\n    def __init__(self) -> None:\n        return\n    @classmethod\n    def greet(cls, x: int) -> int:\n        return x\n",
    );
    let (_, class_def) = &hir.class_defs[0];
    assert_eq!(
        class_def.class_methods,
        vec![("greet".to_string(), "C.greet.classmethod".to_string())]
    );
    // The class method is NOT in the methods table.
    assert!(!class_def.methods.iter().any(|(name, _)| name == "greet"));
}

#[test]
fn a_static_method_function_is_emitted_with_the_static_mangled_name() {
    let hir = lower_ok(
        "class C:\n    def __init__(self) -> None:\n        return\n    @staticmethod\n    def create(x: int) -> int:\n        return x\n",
    );
    assert!(
        hir.items.iter().any(|item| matches!(
            item,
            HirItem::Function { name, .. } if name == "C.create.static"
        )),
        "static method function `C.create.static` should be in items"
    );
}

#[test]
fn a_class_method_function_is_emitted_with_the_classmethod_mangled_name() {
    let hir = lower_ok(
        "class C:\n    def __init__(self) -> None:\n        return\n    @classmethod\n    def greet(cls, x: int) -> int:\n        return x\n",
    );
    assert!(
        hir.items.iter().any(|item| matches!(
            item,
            HirItem::Function { name, .. } if name == "C.greet.classmethod"
        )),
        "class method function `C.greet.classmethod` should be in items"
    );
}

#[test]
fn a_static_method_has_no_implicit_self_parameter() {
    let hir = lower_ok(
        "class C:\n    def __init__(self) -> None:\n        return\n    @staticmethod\n    def create(x: int) -> int:\n        return x\n",
    );
    let fn_item = hir
        .items
        .iter()
        .find_map(|item| match item {
            HirItem::Function { name, params, .. } if name == "C.create.static" => Some(params),
            _ => None,
        })
        .expect("C.create.static should be in items");
    // No `self` — the parameter list is exactly what the user wrote.
    assert_eq!(*fn_item, vec![("x".to_string(), Ty::Int)]);
}

#[test]
fn a_class_method_has_implicit_cls_typed_as_instance() {
    let hir = lower_ok(
        "class C:\n    def __init__(self) -> None:\n        return\n    @classmethod\n    def greet(cls, x: int) -> int:\n        return x\n",
    );
    let fn_item = hir
        .items
        .iter()
        .find_map(|item| match item {
            HirItem::Function { name, params, .. } if name == "C.greet.classmethod" => {
                Some(params)
            }
            _ => None,
        })
        .expect("C.greet.classmethod should be in items");
    assert_eq!(
        *fn_item,
        vec![
            ("cls".to_string(), Ty::Instance(Box::new("C".to_string()))),
            ("x".to_string(), Ty::Int),
        ]
    );
}

#[test]
fn classmethod_on_init_is_rejected() {
    assert_c0001(
        "class C:\n    @classmethod\n    def __init__(cls) -> None:\n        return\n",
    );
}

#[test]
fn a_class_method_without_cls_is_rejected() {
    let module = crate::pycc_parser_test_helper::parse(
        "class C:\n    def __init__(self) -> None:\n        return\n    @classmethod\n    def f() -> int:\n        return 1\n",
    );
    let diagnostic = lower_checked(&module).unwrap_err();
    assert_eq!(diagnostic.code, "C0001");
    assert!(
        diagnostic.message.contains("must take `cls`"),
        "unexpected message: {}",
        diagnostic.message
    );
}

#[test]
fn a_class_method_with_a_non_cls_first_parameter_is_rejected() {
    let module = crate::pycc_parser_test_helper::parse(
        "class C:\n    def __init__(self) -> None:\n        return\n    @classmethod\n    def f(self, x: int) -> int:\n        return x\n",
    );
    let diagnostic = lower_checked(&module).unwrap_err();
    assert_eq!(diagnostic.code, "C0001");
    assert!(
        diagnostic.message.contains("must be named `cls`"),
        "unexpected message: {}",
        diagnostic.message
    );
}

#[test]
fn a_class_method_with_annotated_cls_is_rejected() {
    assert_c0001(
        "class C:\n    def __init__(self) -> None:\n        return\n    @classmethod\n    def f(cls: int, x: int) -> int:\n        return x\n",
    );
}

#[test]
fn a_class_method_with_cls_default_is_rejected() {
    assert_c0001(
        "class C:\n    def __init__(self) -> None:\n        return\n    @classmethod\n    def f(cls = 1) -> int:\n        return 1\n",
    );
}

#[test]
fn a_class_method_with_an_unsupported_parameter_annotation_is_rejected() {
    // Exercises the `?` error propagation on `lower_arg_list` for the
    // non-`cls` parameters of a `@classmethod` (distinct from the `cls`
    // validation checks above, which run before `lower_arg_list`).
    assert_c0001(
        "class C:\n    def __init__(self) -> None:\n        return\n    @classmethod\n    def f(cls, x: Frobnicate) -> int:\n        return 1\n",
    );
}

#[test]
fn a_static_method_with_an_unsupported_parameter_annotation_is_rejected() {
    // Exercises the `?` error propagation on `lower_arg_list` for a
    // `@staticmethod`'s parameter list (which has no `self`/`cls`
    // preprocessing, so `lower_arg_list` is the only validation path).
    assert_c0001(
        "class C:\n    def __init__(self) -> None:\n        return\n    @staticmethod\n    def f(x: Frobnicate) -> int:\n        return 1\n",
    );
}

#[test]
fn a_static_method_redefinition_rebinds() {
    let hir = lower_ok(
        "class C:\n    def __init__(self) -> None:\n        return\n    @staticmethod\n    def f(x: int) -> int:\n        return x\n    @staticmethod\n    def f(x: int) -> int:\n        return x + 1\n",
    );
    let (_, class_def) = &hir.class_defs[0];
    // Rebind: one entry, not two.
    assert_eq!(class_def.static_methods.len(), 1);
    assert_eq!(
        class_def.static_methods[0],
        ("f".to_string(), "C.f.static".to_string())
    );
}

#[test]
fn a_class_method_redefinition_rebinds() {
    let hir = lower_ok(
        "class C:\n    def __init__(self) -> None:\n        return\n    @classmethod\n    def f(cls, x: int) -> int:\n        return x\n    @classmethod\n    def f(cls, x: int) -> int:\n        return x + 1\n",
    );
    let (_, class_def) = &hir.class_defs[0];
    assert_eq!(class_def.class_methods.len(), 1);
    assert_eq!(
        class_def.class_methods[0],
        ("f".to_string(), "C.f.classmethod".to_string())
    );
}
