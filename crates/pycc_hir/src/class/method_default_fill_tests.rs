//! Unit tests for `HirClassDef::omitted_method_defaults` (Part 1 of #1191,
//! issue #1438).

use crate::{HirClassDef, HirExpr, ResolvedImports, lower_module};

fn lowered(source: &str) -> crate::HirModule {
    let parsed = crate::pycc_parser_test_helper::parse(source);
    lower_module(&parsed, &ResolvedImports::default(), None)
        .map(|lowered| lowered.hir)
        .unwrap_or_else(|diagnostics| panic!("{source:?}: {diagnostics:#?}"))
}

fn class_c(module: &crate::HirModule) -> &HirClassDef {
    module
        .class_defs
        .iter()
        .find_map(|(held, def)| (held == "C").then_some(def))
        .expect("class `C`")
}

const SOURCE: &str = "class C:\n\
    \x20   def m(self, a: int, b: int = 2, c: str = 'x') -> None:\n        return\n\
    \x20   def plain(self, a: int) -> None:\n        return\n";

#[test]
fn fills_exactly_the_omitted_trailing_defaults() {
    let module = lowered(SOURCE);
    let class = class_c(&module);
    assert_eq!(
        class.omitted_method_defaults("C.m", 1),
        Some(vec![
            &HirExpr::IntLiteral(2),
            &HirExpr::StringLiteral("x".to_string())
        ])
    );
    assert_eq!(
        class.omitted_method_defaults("C.m", 2),
        Some(vec![&HirExpr::StringLiteral("x".to_string())])
    );
}

#[test]
fn leaves_every_other_call_to_the_arity_check() {
    let module = lowered(SOURCE);
    let class = class_c(&module);
    // A required parameter is omitted.
    assert_eq!(class.omitted_method_defaults("C.m", 0), None);
    // Every parameter is supplied, or more than the method takes.
    assert_eq!(class.omitted_method_defaults("C.m", 3), None);
    assert_eq!(class.omitted_method_defaults("C.m", 4), None);
    // The method records no defaults at all.
    assert_eq!(class.omitted_method_defaults("C.plain", 0), None);
}
