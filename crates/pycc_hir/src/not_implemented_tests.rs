//! Unit tests for `not_implemented.rs` (#1418): which `NotImplemented`
//! spellings an `ext` module admits, the widened return type of a method
//! that uses the admitted one, and the unchanged `native` lowering.

use super::*;
use crate::{HirItem, ResolvedImports, lower_module};

/// Lowers `source` as an `ext` module (`ext == true`) or a `native` one.
fn lower(source: &str, ext: bool) -> Result<crate::HirModule, Vec<Diagnostic>> {
    let parsed = crate::pycc_parser_test_helper::parse(source);
    let mut resolved = ResolvedImports::default();
    resolved.set_ext_module(ext);
    lower_module(&parsed, &resolved, None).map(|lowered| lowered.hir)
}

/// The return type and body of the function item `name` in `source`.
fn function(source: &str, ext: bool, name: &str) -> (Ty, Vec<HirStmt>) {
    let module =
        lower(source, ext).unwrap_or_else(|diagnostics| panic!("{source:?}: {diagnostics:#?}"));
    module
        .items
        .into_iter()
        .find_map(|item| match item {
            HirItem::Function {
                name: held,
                return_ty,
                body,
                ..
            } if held == name => Some((return_ty, body)),
            _ => None,
        })
        .unwrap_or_else(|| panic!("no function `{name}` in {source:?}"))
}

/// The one diagnostic `source` is refused with as an `ext` module, which
/// must be this module's `C0001` at `snippet`'s position.
fn refused(source: &str, snippet: &str) {
    let diagnostics = lower(source, true).expect_err(source);
    assert_eq!(diagnostics.len(), 1, "{source:?}: {diagnostics:#?}");
    let diagnostic = &diagnostics[0];
    assert_eq!(diagnostic.code, "C0001", "{source:?}: {diagnostic:#?}");
    assert!(
        diagnostic
            .message
            .starts_with("`NotImplemented` is not supported here yet: an `--ext` module admits")
            && diagnostic.message.ends_with("method (#1418)"),
        "{source:?}: {diagnostic:#?}"
    );
    let start = u32::try_from(source.find(snippet).expect("snippet in source")).unwrap();
    let span = diagnostic.span.expect("the refusal carries a span");
    assert_eq!(span.start, start, "{source:?}: {diagnostic:#?}");
}

/// A class whose `method` has `body` (indented by eight spaces).
fn class_with(method: &str, body: &str) -> String {
    format!("class C:\n    def {method}(self, other: object) -> bool:\n{body}")
}

#[test]
fn each_comparison_method_returning_not_implemented_is_widened_to_the_object() {
    for dunder in COMPARISON_DUNDERS {
        let source = class_with(
            dunder,
            "        if other is None:\n            return NotImplemented\n        return False\n",
        );
        let (return_ty, body) = function(&source, true, &format!("C.{dunder}"));
        assert_eq!(return_ty, Ty::Object, "{source}");
        assert!(body_returns_not_implemented(&body), "{source}");
    }
}

#[test]
fn a_comparison_method_without_not_implemented_keeps_its_annotation() {
    let source = class_with("__eq__", "        return False\n");
    let (return_ty, body) = function(&source, true, "C.__eq__");
    assert_eq!(return_ty, Ty::Bool);
    assert!(!body_returns_not_implemented(&body));
}

#[test]
fn a_native_module_keeps_the_ordinary_name_lowering() {
    let source =
        "class C:\n    def __eq__(self, other: C) -> bool:\n        return NotImplemented\n";
    let (return_ty, body) = function(source, false, "C.__eq__");
    assert_eq!(return_ty, Ty::Bool);
    assert!(!body_returns_not_implemented(&body));
    assert!(
        matches!(&body[..], [HirStmt::Return(Some(HirExpr::Name(name)))] if name == "NotImplemented"),
        "{body:#?}"
    );
    // A native module refuses nothing here either; `pycc_types` reports its
    // `T0021` for the unbound name.
    lower("x = NotImplemented\n", false).unwrap();
}

#[test]
fn every_nested_block_is_searched() {
    for block in [
        "        if other is None:\n            pass\n        else:\n            return NotImplemented\n        return False\n",
        "        try:\n            return NotImplemented\n        except ValueError:\n            pass\n        return False\n",
        "        try:\n            pass\n        except ValueError:\n            return NotImplemented\n        return False\n",
        "        try:\n            pass\n        except ValueError:\n            pass\n        else:\n            return NotImplemented\n        return False\n",
        "        try:\n            pass\n        finally:\n            pass\n        return NotImplemented\n",
        "        try:\n            return NotImplemented\n        except* ValueError:\n            pass\n        return False\n",
        "        while other is None:\n            return NotImplemented\n        return False\n",
        "        for i in range(3):\n            return NotImplemented\n        return False\n",
        "        xs = [1, 2]\n        for i in xs:\n            return NotImplemented\n        return False\n",
        "        for i in other:\n            return NotImplemented\n        return False\n",
        "        match 1:\n            case 1:\n                return NotImplemented\n        return False\n",
    ] {
        let source = class_with("__lt__", block);
        let (return_ty, _) = function(&source, true, "C.__lt__");
        assert_eq!(return_ty, Ty::Object, "{source}");
    }
}

#[test]
fn a_body_of_other_statements_does_not_return_not_implemented() {
    let source = class_with(
        "__ne__",
        "        x = 1\n        y: int = 2\n        print(x, y)\n        if x:\n            return\n        raise ValueError()\n",
    );
    let (_, body) = function(&source, true, "C.__ne__");
    assert!(!body_returns_not_implemented(&body));
}

#[test]
fn every_other_position_is_refused() {
    let at = |source: &str| refused(source, "NotImplemented");
    // Outside a comparison method.
    at("x = NotImplemented\n");
    at("def f() -> object:\n    return NotImplemented\n");
    at(&class_with("eq", "        return NotImplemented\n"));
    at("class C:\n    x: object = NotImplemented\n");
    // Inside one, but not as the whole value of its own `return`.
    at(&class_with(
        "__eq__",
        "        x = NotImplemented\n        return x\n",
    ));
    at(&class_with(
        "__eq__",
        "        return (NotImplemented, 1)\n",
    ));
    at(&class_with(
        "__eq__",
        "        print(NotImplemented)\n        return False\n",
    ));
    // A nested scope admits nothing.
    at(&class_with(
        "__eq__",
        "        def g() -> object:\n            return NotImplemented\n        return False\n",
    ));
    at(&class_with(
        "__eq__",
        "        g = lambda: NotImplemented\n        return False\n",
    ));
    at(&class_with(
        "__eq__",
        "        class D:\n            def __eq__(self, other: object) -> bool:\n                return False\n            x = NotImplemented\n        return False\n",
    ));
    // A decorated or `async` comparison method is not admitted.
    at(
        "class C:\n    @staticmethod\n    def __eq__(a: object, b: object) -> bool:\n        return NotImplemented\n",
    );
    at(
        "class C:\n    async def __eq__(self, other: object) -> bool:\n        return NotImplemented\n",
    );
    // Nor is the method's own header.
    at(
        "class C:\n    def __eq__(self, other: object = NotImplemented) -> bool:\n        return False\n",
    );
    at("class C:\n    def __eq__(self, other: NotImplemented) -> bool:\n        return False\n");
    at("class C:\n    def __eq__(self, other: object) -> NotImplemented:\n        return False\n");
    at(
        "class C:\n    def __eq__[T: NotImplemented](self, other: T) -> bool:\n        return False\n",
    );
    at(
        "class C:\n    @NotImplemented\n    def __eq__(self, other: object) -> bool:\n        return False\n",
    );
    // Nor the class's own header.
    at("@NotImplemented\nclass C:\n    pass\n");
    at("class C(NotImplemented):\n    pass\n");
    at("class C[T: NotImplemented]:\n    pass\n");
}

#[test]
fn only_the_first_refused_occurrence_is_reported() {
    refused(
        "class C:\n    def eq(self) -> object:\n        print(NotImplemented, NotImplemented)\n        return NotImplemented\n",
        "NotImplemented",
    );
}
