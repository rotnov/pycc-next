//! Unit tests for `class/body.rs`'s class-body walk, moved out of that file
//! per AGENTS.md's file-decomposition rule when issue #1188 edited them.

use crate::class::tests::{assert_c0001, lower_ok};
use crate::{ContainerFallback, HirExpr, HirItem, HirStmt, lower_checked};

#[test]
fn a_non_def_class_body_statement_is_unsupported() {
    // #1266 made a value-less `x: int` an instance attribute declaration, so
    // this pins the catch-all with a statement kind that is still refused.
    assert_c0001("class C:\n    print(1)\n");
}

#[test]
fn a_value_less_annotation_without_an_init_is_an_unestablished_declaration() {
    // #1266: what this file's catch-all test used to spell is now refused
    // for a different reason -- nothing establishes the declared attribute.
    let message = crate::lower_checked(&crate::pycc_parser_test_helper::parse(
        "class C:\n    x: int\n",
    ))
    .unwrap_err()
    .message;
    assert!(
        message.starts_with("instance attribute `x` declared in class `C` is never assigned"),
        "{message}"
    );
}

#[test]
fn redefining_init_in_one_class_body_is_unsupported() {
    // #386: `__init__` redefinition stays C0001 -- the compile-time
    // attribute-slot pre-scan (`collect_init_attrs`) cannot reconcile
    // two different `__init__` bodies.
    assert_c0001(
        "class C:\n    def __init__(self) -> None:\n        return\n    def __init__(self) -> None:\n        return\n",
    );
}

#[test]
fn redefining_a_non_init_method_rebinds_to_the_latest_definition() {
    // #386: a non-`__init__` method redefinition is a rebind, not an
    // error. Both definitions lower into separate `HirItem::Function`s
    // with the same mangled name (`C.foo`), and the method table entry
    // is replaced (not duplicated) -- so `methods` has exactly one
    // `foo` entry, while `items` has two `C.foo` function items.
    let hir = lower_ok(
        "class C:\n    def __init__(self) -> None:\n        return\n    def foo(self) -> None:\n        return\n    def foo(self) -> None:\n        return\n",
    );
    assert_eq!(hir.class_defs.len(), 1);
    let (_, class_def) = &hir.class_defs[0];
    // The method table has exactly one `foo` entry (replaced, not
    // duplicated), plus the `__init__` entry.
    assert_eq!(
        class_def.methods,
        vec![
            ("__init__".to_string(), "C.__init__".to_string()),
            ("foo".to_string(), "C.foo".to_string()),
        ]
    );
    // Both definitions are lowered as separate `HirItem::Function`s
    // with the same mangled name -- PR #358's function-pointer slot
    // handles the rebind at the codegen level. Using `matches!` rather
    // than an `if let .. { true } else { false }` keeps the closure
    // branch-free under D-014's 100%-region coverage gate (every item
    // in this fixture is a `HirItem::Function`, so an `else { false }`
    // arm would be a permanently uncovered region).
    let foo_items: Vec<&HirItem> = hir
        .items
        .iter()
        .filter(|item| matches!(item, HirItem::Function { name, .. } if name == "C.foo"))
        .collect();
    assert_eq!(foo_items.len(), 2, "both foo definitions should be lowered");
}

#[test]
fn an_ordinary_class_with_a_docstring_lowers_successfully() {
    // #744: a class docstring (a bare string-literal expression
    // statement) is a no-op in an ordinary (non-dataclass) class body.
    let hir =
        lower_ok("class C:\n    \"A class.\"\n    def __init__(self) -> None:\n        return\n");
    assert_eq!(hir.class_defs.len(), 1);
}

#[test]
fn an_ordinary_class_with_a_non_leading_docstring_lowers_successfully() {
    // #744's guard has no position check: a bare string-literal
    // expression statement is a no-op anywhere in the body, not only
    // when it appears first. Place it after `__init__` to exercise
    // that non-leading position directly, rather than only inferring
    // it from the loop structure.
    let hir =
        lower_ok("class C:\n    def __init__(self) -> None:\n        return\n    \"A class.\"\n");
    assert_eq!(hir.class_defs.len(), 1);
}

#[test]
fn a_non_string_expression_statement_in_a_class_body_is_still_rejected() {
    // #744's docstring exemption covers only a bare string-literal
    // expression statement: a bare non-string expression statement in a
    // class body remains C0001, distinguishing it from the docstring
    // no-op added alongside it.
    assert_c0001("class C:\n    42\n    def __init__(self) -> None:\n        return\n");
}

/// The `HirExpr::ReceiverDispatchedCall` the module-level call statement
/// `index` of `hir` lowered to.
fn dispatched_call(hir: &crate::HirModule, index: usize) -> (&HirExpr, &ContainerFallback) {
    let top_level: Vec<&HirStmt> = hir
        .items
        .iter()
        .filter_map(|item| match item {
            HirItem::TopLevelStmt(stmt) => Some(stmt),
            _ => None,
        })
        .collect();
    let HirStmt::ExprStmt(HirExpr::ReceiverDispatchedCall { call, container }) = top_level[index]
    else {
        panic!(
            "statement {index} is not a receiver-dispatched call: {:?}",
            top_level[index]
        );
    };
    (call, container)
}

#[test]
fn a_method_named_get_lowers_and_its_call_keeps_both_readings() {
    // Issue #1188 retired the class-definition-time refusal of a method
    // named `get` (D-068 review finding on #385). The class lowers with the
    // method in its table, and `buf.get(5)` keeps the method reading next to
    // the container reading's own refusal, for `pycc_types` to choose from
    // by the receiver's type.
    let hir = lower_ok(
        "class Buf:\n    def __init__(self) -> None:\n        return\n    def get(self, k: int) -> int:\n        return k\n\nbuf = Buf()\nbuf.get(5)\n",
    );
    let (_, class_def) = &hir.class_defs[0];
    assert!(class_def.methods.iter().any(|(name, _)| name == "get"));
    let (call, container) = dispatched_call(&hir, 1);
    assert!(matches!(call, HirExpr::MethodCall { method, .. } if method == "get"));
    let ContainerFallback::Refused(diagnostic) = container else {
        panic!("expected the container reading to be refused: {container:?}");
    };
    assert_eq!(
        diagnostic.message,
        "`.get()` is only supported as `dict.get(key, default)` with exactly two arguments so far, got 1"
    );
}

#[test]
fn a_method_named_append_pop_or_add_also_lowers() {
    // The remaining three names the retired refusal guarded. Each call has an
    // arity the container reading refuses, so the carried diagnostic is that
    // reading's own message, unchanged.
    let cases = [
        (
            "append",
            "c.append()",
            "list.append() takes exactly one argument, got 0",
        ),
        ("pop", "c.pop(1)", "list.pop() takes no arguments, got 1"),
        (
            "add",
            "c.add()",
            "set.add() takes exactly one argument, got 0",
        ),
    ];
    for (name, call, container_message) in cases {
        let source = format!(
            "class C:\n    def __init__(self) -> None:\n        return\n    def {name}(self) -> None:\n        return\n\nc = C()\n{call}\n"
        );
        let hir = lower_ok(&source);
        let (_, class_def) = &hir.class_defs[0];
        assert!(
            class_def.methods.iter().any(|(method, _)| method == name),
            "name: {name}"
        );
        let (_, container) = dispatched_call(&hir, 1);
        let ContainerFallback::Refused(diagnostic) = container else {
            panic!("name: {name}, expected a refused container reading: {container:?}");
        };
        assert_eq!(diagnostic.message, container_message, "name: {name}");
    }
}

#[test]
fn an_async_method_is_unsupported() {
    assert_c0001("class C:\n    async def __init__(self) -> None:\n        return\n");
}

#[test]
fn a_decorated_method_is_unsupported() {
    assert_c0001("class C:\n    @staticmethod\n    def __init__(self) -> None:\n        return\n");
}

#[test]
fn a_generic_method_is_unsupported() {
    assert_c0001("class C:\n    def __init__[T](self) -> None:\n        return\n");
}

#[test]
fn a_property_getter_shadowing_a_method_is_rejected() {
    let module = crate::pycc_parser_test_helper::parse(
        "class C:\n    def __init__(self) -> None:\n        return\n    def x(self) -> int:\n        return 1\n    @property\n    def x(self) -> int:\n        return 2\n",
    );
    let diagnostic = lower_checked(&module).unwrap_err();
    assert_eq!(diagnostic.code, "C0001");
    assert!(
        diagnostic.message.contains("cannot shadow a method"),
        "unexpected message: {}",
        diagnostic.message
    );
}

#[test]
fn a_method_shadowing_a_property_getter_is_rejected() {
    let module = crate::pycc_parser_test_helper::parse(
        "class C:\n    def __init__(self) -> None:\n        return\n    @property\n    def x(self) -> int:\n        return 2\n    def x(self) -> int:\n        return 1\n",
    );
    let diagnostic = lower_checked(&module).unwrap_err();
    assert_eq!(diagnostic.code, "C0001");
    assert!(
        diagnostic.message.contains("cannot shadow a property"),
        "unexpected message: {}",
        diagnostic.message
    );
}

#[test]
fn a_method_shadowing_a_static_method_is_rejected() {
    let module = crate::pycc_parser_test_helper::parse(
        "class C:\n    def __init__(self) -> None:\n        return\n    @staticmethod\n    def foo(x: int) -> int:\n        return x\n    def foo(self, x: int) -> int:\n        return x + 1\n",
    );
    let diagnostic = lower_checked(&module).unwrap_err();
    assert_eq!(diagnostic.code, "C0001");
    assert!(
        diagnostic
            .message
            .contains("cannot share a name with a `@staticmethod`"),
        "unexpected message: {}",
        diagnostic.message
    );
}

#[test]
fn a_method_shadowing_a_class_method_is_rejected() {
    let module = crate::pycc_parser_test_helper::parse(
        "class C:\n    def __init__(self) -> None:\n        return\n    @classmethod\n    def foo(cls, x: int) -> int:\n        return x\n    def foo(self, x: int) -> int:\n        return x + 1\n",
    );
    let diagnostic = lower_checked(&module).unwrap_err();
    assert_eq!(diagnostic.code, "C0001");
    assert!(
        diagnostic
            .message
            .contains("cannot share a name with a `@classmethod`"),
        "unexpected message: {}",
        diagnostic.message
    );
}

#[test]
fn a_static_method_shadowing_a_method_is_rejected() {
    let module = crate::pycc_parser_test_helper::parse(
        "class C:\n    def __init__(self) -> None:\n        return\n    def foo(self, x: int) -> int:\n        return x\n    @staticmethod\n    def foo(x: int) -> int:\n        return x + 1\n",
    );
    let diagnostic = lower_checked(&module).unwrap_err();
    assert_eq!(diagnostic.code, "C0001");
    assert!(
        diagnostic
            .message
            .contains("cannot share a name with a regular method"),
        "unexpected message: {}",
        diagnostic.message
    );
}

#[test]
fn a_static_method_shadowing_a_property_is_rejected() {
    let module = crate::pycc_parser_test_helper::parse(
        "class C:\n    def __init__(self) -> None:\n        return\n    @property\n    def foo(self) -> int:\n        return 1\n    @staticmethod\n    def foo(x: int) -> int:\n        return x + 1\n",
    );
    let diagnostic = lower_checked(&module).unwrap_err();
    assert_eq!(diagnostic.code, "C0001");
    assert!(
        diagnostic
            .message
            .contains("cannot share a name with a property"),
        "unexpected message: {}",
        diagnostic.message
    );
}

#[test]
fn a_static_method_shadowing_a_class_method_is_rejected() {
    let module = crate::pycc_parser_test_helper::parse(
        "class C:\n    def __init__(self) -> None:\n        return\n    @classmethod\n    def foo(cls, x: int) -> int:\n        return x\n    @staticmethod\n    def foo(x: int) -> int:\n        return x + 1\n",
    );
    let diagnostic = lower_checked(&module).unwrap_err();
    assert_eq!(diagnostic.code, "C0001");
    assert!(
        diagnostic
            .message
            .contains("cannot share a name with a `@classmethod`"),
        "unexpected message: {}",
        diagnostic.message
    );
}

#[test]
fn a_class_method_shadowing_a_method_is_rejected() {
    let module = crate::pycc_parser_test_helper::parse(
        "class C:\n    def __init__(self) -> None:\n        return\n    def foo(self, x: int) -> int:\n        return x\n    @classmethod\n    def foo(cls, x: int) -> int:\n        return x + 1\n",
    );
    let diagnostic = lower_checked(&module).unwrap_err();
    assert_eq!(diagnostic.code, "C0001");
    assert!(
        diagnostic
            .message
            .contains("cannot share a name with a regular method"),
        "unexpected message: {}",
        diagnostic.message
    );
}

#[test]
fn a_class_method_shadowing_a_property_is_rejected() {
    let module = crate::pycc_parser_test_helper::parse(
        "class C:\n    def __init__(self) -> None:\n        return\n    @property\n    def foo(self) -> int:\n        return 1\n    @classmethod\n    def foo(cls, x: int) -> int:\n        return x + 1\n",
    );
    let diagnostic = lower_checked(&module).unwrap_err();
    assert_eq!(diagnostic.code, "C0001");
    assert!(
        diagnostic
            .message
            .contains("cannot share a name with a property"),
        "unexpected message: {}",
        diagnostic.message
    );
}

#[test]
fn a_class_method_shadowing_a_static_method_is_rejected() {
    let module = crate::pycc_parser_test_helper::parse(
        "class C:\n    def __init__(self) -> None:\n        return\n    @staticmethod\n    def foo(x: int) -> int:\n        return x\n    @classmethod\n    def foo(cls, x: int) -> int:\n        return x + 1\n",
    );
    let diagnostic = lower_checked(&module).unwrap_err();
    assert_eq!(diagnostic.code, "C0001");
    assert!(
        diagnostic
            .message
            .contains("cannot share a name with a `@staticmethod`"),
        "unexpected message: {}",
        diagnostic.message
    );
}

#[test]
fn staticmethod_on_init_is_rejected() {
    assert_c0001(
        "class C:\n    @staticmethod\n    def __init__(x: int) -> None:\n        return\n",
    );
}
