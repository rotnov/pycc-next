//! Unit tests for the method part of #1140: a default parameter value on a
//! method is admitted under the module-level rules and recorded in
//! `HirClassDef::method_defaults` for the `--ext` host boundary.

use pycc_diag::Diagnostic;

use crate::{HirClassDef, HirExpr, ResolvedImports, lower_module};

/// Lowers `source` as an `ext` module (`ext == true`) or a `native` one.
fn lower(source: &str, ext: bool) -> Result<crate::HirModule, Vec<Diagnostic>> {
    let parsed = crate::pycc_parser_test_helper::parse(source);
    let mut resolved = ResolvedImports::default();
    resolved.set_ext_module(ext);
    lower_module(&parsed, &resolved, None).map(|lowered| lowered.hir)
}

fn lowered(source: &str, ext: bool) -> crate::HirModule {
    lower(source, ext).unwrap_or_else(|diagnostics| panic!("{source:?}: {diagnostics:#?}"))
}

fn only_error(source: &str, ext: bool) -> Diagnostic {
    let diagnostics = lower(source, ext).expect_err("the fixture must be rejected");
    assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
    diagnostics.into_iter().next().expect("one diagnostic")
}

fn class<'m>(module: &'m crate::HirModule, name: &str) -> &'m HirClassDef {
    module
        .class_defs
        .iter()
        .find_map(|(held, def)| (held == name).then_some(def))
        .unwrap_or_else(|| panic!("no class `{name}`"))
}

/// `name`'s recorded defaults in class `C`, `None` when it has no entry.
fn defaults_of(module: &crate::HirModule, name: &str) -> Option<Vec<Option<HirExpr>>> {
    class(module, "C")
        .method_defaults
        .iter()
        .find_map(|(held, defaults)| (held == name).then(|| defaults.clone()))
}

#[test]
fn each_method_kind_records_its_defaults_parallel_to_its_full_parameters() {
    let module = lowered(
        "class C:\n\
         \x20   def __init__(self, a: int, b: int = 2, c: str = 'x') -> None:\n        return\n\
         \x20   def m(self, a: bool = True, /, b: float = -1.5) -> None:\n        return\n\
         \x20   @staticmethod\n    def s(a: int, b: int = -3) -> int:\n        return a\n\
         \x20   @classmethod\n    def k(cls, a: int | None = None) -> None:\n        return\n\
         \x20   def plain(self, a: int) -> None:\n        return\n",
        false,
    );
    assert_eq!(
        defaults_of(&module, "C.__init__"),
        Some(vec![
            None,
            None,
            Some(HirExpr::IntLiteral(2)),
            Some(HirExpr::StringLiteral("x".to_string())),
        ])
    );
    // A positional-only defaulted parameter is recorded exactly as an
    // ordinary one: the host passes both positionally.
    assert_eq!(
        defaults_of(&module, "C.m"),
        Some(vec![
            None,
            Some(HirExpr::BoolLiteral(true)),
            Some(HirExpr::FloatLiteral(-1.5)),
        ])
    );
    // A `@staticmethod` has no receiver entry.
    assert_eq!(
        defaults_of(&module, "C.s.static"),
        Some(vec![None, Some(HirExpr::IntLiteral(-3))])
    );
    assert_eq!(
        defaults_of(&module, "C.k.classmethod"),
        Some(vec![None, Some(HirExpr::NoneLiteral)])
    );
    // A method without a default records nothing.
    assert_eq!(defaults_of(&module, "C.plain"), None);
}

#[test]
fn a_redefinition_replaces_or_removes_the_recorded_defaults() {
    let module = lowered(
        "class C:\n\
         \x20   def m(self, a: int = 1) -> None:\n        return\n\
         \x20   def m(self, a: int = 5) -> None:\n        return\n\
         \x20   def n(self, a: int = 1) -> None:\n        return\n\
         \x20   def n(self, a: int) -> None:\n        return\n",
        false,
    );
    assert_eq!(
        defaults_of(&module, "C.m"),
        Some(vec![None, Some(HirExpr::IntLiteral(5))])
    );
    assert_eq!(defaults_of(&module, "C.n"), None);
}

#[test]
fn an_unadmitted_method_default_keeps_the_module_level_capability_error() {
    for default in ["[]", "{}", "f()", "1 + 2"] {
        let source =
            format!("class C:\n    def m(self, a: int = {default}) -> None:\n        return\n");
        let diagnostic = only_error(&source, false);
        assert_eq!(diagnostic.code, "C0001", "{source}");
        assert_eq!(
            diagnostic.message,
            crate::func::params::UNADMITTED_DEFAULT,
            "{source}"
        );
    }
}

#[test]
fn an_unassignable_method_default_is_a_type_error() {
    let diagnostic = only_error(
        "class C:\n    def m(self, a: int = 'x') -> None:\n        return\n",
        false,
    );
    assert_eq!(diagnostic.code, "T0021");
    assert_eq!(
        diagnostic.message,
        "default value of parameter `a` of `m` expects `int`, got `str`"
    );
}

#[test]
fn none_defaults_an_object_parameter_of_a_method_but_not_of_a_function() {
    let module = lowered(
        "from typing import Any\n\
         class C:\n\
         \x20   def __init__(self, a: Any = None, b: object = None) -> None:\n        return\n",
        true,
    );
    assert_eq!(
        defaults_of(&module, "C.__init__"),
        Some(vec![
            None,
            Some(HirExpr::NoneLiteral),
            Some(HirExpr::NoneLiteral),
        ])
    );
    // A module-level default is spliced into a native call, so it keeps the
    // strict rule.
    let diagnostic = only_error(
        "from typing import Any\ndef f(a: Any = None) -> None:\n    return\n",
        true,
    );
    assert_eq!(diagnostic.code, "T0021");
    assert_eq!(
        diagnostic.message,
        "default value of parameter `a` of `f` expects `object`, got `None`"
    );
}

#[test]
fn only_none_is_admitted_at_an_object_method_parameter() {
    let diagnostic = only_error(
        "from typing import Any\n\
         class C:\n    def m(self, a: Any = 1) -> None:\n        return\n",
        true,
    );
    assert_eq!(diagnostic.code, "T0021");
    assert_eq!(
        diagnostic.message,
        "default value of parameter `a` of `m` expects `object`, got `int`"
    );
}

// --- #1409: an unannotated defaulted parameter in an `--ext` module -----

/// The `(name, type)` parameter list of the lowered function `name`.
fn params_of(module: &crate::HirModule, name: &str) -> Vec<(String, crate::Ty)> {
    module
        .items
        .iter()
        .find_map(|item| match item {
            crate::HirItem::Function {
                name: held, params, ..
            } if held == name => Some(params.clone()),
            _ => None,
        })
        .unwrap_or_else(|| panic!("no function `{name}`"))
}

/// The #1207 subject's two signatures (lark `ParserState.__init__` and
/// `ParserState.copy`) and every other method kind: `None` gives the opaque
/// object, a literal its scalar type, and the default is recorded exactly as
/// its annotated twin's is.
#[test]
fn an_ext_methods_unannotated_default_gives_its_parameter_a_type() {
    use crate::Ty;
    let module = lowered(
        "class C:\n\
         \x20   def __init__(self, n: int, state_stack=None, value_stack=None) -> None:\n\
         \x20       return\n\
         \x20   def copy(self, deepcopy_values=True) -> bool:\n\
         \x20       if deepcopy_values:\n            return True\n        return False\n\
         \x20   @staticmethod\n    def s(a=-3, b=2.5, c='x') -> int:\n        return a\n\
         \x20   @classmethod\n    def k(cls, a=None, /, b=False) -> None:\n        return\n",
        true,
    );
    let receiver = || ("self".to_string(), Ty::Instance(Box::new("C".to_string())));
    let named = |name: &str, ty: Ty| (name.to_string(), ty);
    assert_eq!(
        params_of(&module, "C.__init__"),
        vec![
            receiver(),
            named("n", Ty::Int),
            named("state_stack", Ty::Object),
            named("value_stack", Ty::Object),
        ]
    );
    assert_eq!(
        defaults_of(&module, "C.__init__"),
        Some(vec![
            None,
            None,
            Some(HirExpr::NoneLiteral),
            Some(HirExpr::NoneLiteral),
        ])
    );
    assert_eq!(
        params_of(&module, "C.copy"),
        vec![receiver(), named("deepcopy_values", Ty::Bool)]
    );
    assert_eq!(
        params_of(&module, "C.s.static"),
        vec![
            named("a", Ty::Int),
            named("b", Ty::Float),
            named("c", Ty::Str)
        ]
    );
    assert_eq!(
        defaults_of(&module, "C.s.static"),
        Some(vec![
            Some(HirExpr::IntLiteral(-3)),
            Some(HirExpr::FloatLiteral(2.5)),
            Some(HirExpr::StringLiteral("x".to_string())),
        ])
    );
    assert_eq!(
        params_of(&module, "C.k.classmethod"),
        vec![
            ("cls".to_string(), Ty::Instance(Box::new("C".to_string()))),
            named("a", Ty::Object),
            named("b", Ty::Bool),
        ]
    );
}

/// A module-level `def` takes a literal default's scalar type too, but a
/// `None` default implies no type there (its default is spliced into
/// in-module calls, where `None` at an object slot is refused, #1387), so it
/// keeps `T0001`.
#[test]
fn an_ext_functions_literal_default_types_it_but_none_does_not() {
    use crate::Ty;
    let module = lowered("def f(a=1, b='s', c=True) -> int:\n    return a\n", true);
    assert_eq!(
        params_of(&module, "f"),
        vec![
            ("a".to_string(), Ty::Int),
            ("b".to_string(), Ty::Str),
            ("c".to_string(), Ty::Bool),
        ]
    );
    let diagnostic = only_error("def f(a=None) -> None:\n    return\n", true);
    assert_eq!(diagnostic.code, "T0001");
    assert_eq!(
        diagnostic.message,
        "parameter `a` of public function `f` needs a type annotation"
    );
}

/// Each case the rule leaves alone keeps its own unchanged diagnostic: no
/// default, a default outside the literal subset, a `native` build, and a
/// `Protocol` member (which refuses any default first).
#[test]
fn an_unannotated_parameter_the_rule_does_not_cover_keeps_its_diagnostic() {
    for (source, ext, code, message) in [
        (
            "class C:\n    def m(self, a) -> None:\n        return\n",
            true,
            "T0001",
            "parameter `a` of public function `m` needs a type annotation",
        ),
        (
            "class C:\n    def m(self, a=[]) -> None:\n        return\n",
            true,
            "T0001",
            "parameter `a` of public function `m` needs a type annotation",
        ),
        (
            "class C:\n    def m(self, a=1 + 2) -> None:\n        return\n",
            true,
            "T0001",
            "parameter `a` of public function `m` needs a type annotation",
        ),
        (
            "class C:\n    def m(self, a=True) -> None:\n        return\n",
            false,
            "T0001",
            "parameter `a` of public function `m` needs a type annotation",
        ),
        (
            "class C:\n    def __init__(self, a=None) -> None:\n        return\n",
            false,
            "T0001",
            "parameter `a` of public function `__init__` needs a type annotation",
        ),
        (
            "def f(a=1) -> None:\n    return\n",
            false,
            "T0001",
            "parameter `a` of public function `f` needs a type annotation",
        ),
        (
            "from typing import Protocol\n\
             class P(Protocol):\n    def m(self, a=True) -> None:\n        ...\n",
            true,
            "C0001",
            "default parameter values are not supported yet",
        ),
    ] {
        let diagnostic = only_error(source, ext);
        assert_eq!(diagnostic.code, code, "{source}");
        assert_eq!(diagnostic.message, message, "{source}");
    }
}

/// A private method's or function's unannotated parameter keeps `Ty::Infer`
/// in an `--ext` module: only the case that was `T0001` changes.
#[test]
fn a_private_unannotated_defaulted_parameter_still_infers() {
    let module = lowered(
        "class C:\n    def _m(self, a=True) -> None:\n        return\n\
         def _f(a=None) -> None:\n    return\n",
        true,
    );
    assert_eq!(params_of(&module, "C._m")[1].1, crate::Ty::Infer);
    assert_eq!(params_of(&module, "_f")[0].1, crate::Ty::Infer);
}

/// #1387: `class C` with `body` beside `__init__` and one public method.
fn class_with(body: &str) -> String {
    format!(
        "class C:\n    def __init__(self, n: int) -> None:\n        self.n = n\n\n{body}\n    def size(self) -> int:\n        return self.n\n"
    )
}

#[test]
fn an_unannotated_equality_operand_is_an_object_in_an_ext_module() {
    // lark's `ParserState.__eq__(self, other) -> bool` (line 51), and its
    // `__ne__` counterpart.
    for name in ["__eq__", "__ne__"] {
        let source = class_with(&format!(
            "    def {name}(self, other) -> bool:\n        return other is None\n"
        ));
        let mangled = format!("C.{name}");
        assert_eq!(
            params_of(&lowered(&source, true), &mangled)[1],
            ("other".to_string(), crate::Ty::Object),
            "{source}"
        );
        // A `native` module has no `object` to give it: the operand stays
        // an inference variable, which `pycc_types` reports as `T0021`.
        assert_eq!(
            params_of(&lowered(&source, false), &mangled)[1].1,
            crate::Ty::Infer,
            "{source}"
        );
    }
}

#[test]
fn only_the_bare_operand_of_eq_and_ne_becomes_an_object() {
    for (body, mangled, expected) in [
        // Another rich comparison: its operand is not this rule's.
        (
            "    def __lt__(self, other) -> bool:\n        return False\n",
            "C.__lt__",
            vec![crate::Ty::Infer],
        ),
        // An annotated operand keeps its annotation.
        (
            "    def __eq__(self, other: int) -> bool:\n        return other == 1\n",
            "C.__eq__",
            vec![crate::Ty::Int],
        ),
        // A defaulted operand is not the data model's operand shape, and a
        // private method's defaulted parameter keeps inferring (#1409).
        (
            "    def __eq__(self, other=1) -> bool:\n        return other == 1\n",
            "C.__eq__",
            vec![crate::Ty::Infer],
        ),
        // A second parameter: not the data model's binary shape.
        (
            "    def __eq__(self, other, extra) -> bool:\n        return False\n",
            "C.__eq__",
            vec![crate::Ty::Infer, crate::Ty::Infer],
        ),
    ] {
        let source = class_with(body);
        let params = params_of(&lowered(&source, true), mangled);
        let tail: Vec<crate::Ty> = params[1..].iter().map(|(_, ty)| ty.clone()).collect();
        assert_eq!(tail, expected, "{source}");
    }
    // A static method has no receiver, so its one parameter is no operand.
    let source =
        class_with("    @staticmethod\n    def __eq__(other) -> bool:\n        return False\n");
    assert_eq!(
        params_of(&lowered(&source, true), "C.__eq__.static"),
        vec![("other".to_string(), crate::Ty::Infer)],
        "{source}"
    );
}
