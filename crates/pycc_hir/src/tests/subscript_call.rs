//! Unit tests for Part 2a of #1371's subscript-callee rule
//! (`expr::subscript_call`): which `X[Y](args)` calls stay a generic class
//! instantiation and which lower to `HirExpr::ExprCall`.
//!
//! Its own child module so `tests.rs` does not keep growing (#663);
//! `assert_capability_error_message` and the lowering entry points are
//! reached through `use super::*`.
use super::*;

/// The last top-level expression statement of `source`, lowered with no
/// project import resolved.
fn last_expr(source: &str) -> HirExpr {
    let hir = lower_checked(&pycc_parser_test_helper::parse(source))
        .unwrap_or_else(|diagnostic| panic!("{source:?} must lower: {diagnostic:?}"));
    match hir.items.last() {
        Some(HirItem::TopLevelStmt(HirStmt::ExprStmt(expr))) => expr.clone(),
        other => panic!("{source:?}: expected a trailing expression, got {other:?}"),
    }
}

/// Lowers `importer` with every project import answered by `dependency` as
/// a resolved project module (`dep.py`), the way `src/modules.rs` does, or
/// as a CPython (`Foreign`) module when `dependency` is `None`.
fn lower_importing(dependency: Option<&str>, importer: &str) -> Result<HirModule, Diagnostic> {
    let dep = dependency.map(|source| {
        lower_module(
            &pycc_parser_test_helper::parse(source),
            &ResolvedImports::default(),
            None,
        )
        .expect("dependency lowers")
    });
    let parsed = pycc_parser_test_helper::parse(importer);
    let mut resolved = ResolvedImports::default();
    if let Some(dep) = &dep {
        resolved.add_module("dep.py".to_string(), &dep.hir, &dep.class_slots);
    }
    for request in project_import_requests(&parsed) {
        let answer = match &dep {
            Some(dep) => ResolvedImport::Module(ResolvedModule {
                display_path: "dep.py".to_string(),
                hir: &dep.hir,
                submodule_names: Vec::new(),
            }),
            None => ResolvedImport::Foreign,
        };
        resolved.insert(request.span, answer);
    }
    lower_module(&parsed, &resolved, None)
        .map(|lowered| lowered.hir)
        .map_err(|mut diagnostics| diagnostics.remove(0))
}

fn is_expr_call(expr: &HirExpr) -> bool {
    matches!(expr, HirExpr::ExprCall { callee, .. } if matches!(**callee, HirExpr::Subscript { .. }))
}

const GENERIC_C: &str = "class C[T]:\n    def __init__(self, x: T) -> None:\n        self.x = x\n";

/// A subscript callee whose base is not a bare name was refused with
/// "calling a subscript expression" before Part 2a; it is now an ordinary
/// call of a subscript result, whatever the key -- `pycc_types` decides.
#[test]
fn a_non_name_subscript_base_lowers_to_an_expression_call() {
    for source in [
        "(1 + 2)[int](1)\n",
        "t = [1]\n(t)[0](1)\n",
        "t = [[1]]\nt[0][0](1, 2)\n",
    ] {
        let expr = last_expr(source);
        assert!(is_expr_call(&expr), "{source:?}: {expr:?}");
    }
}

/// A bare-name base that is not class-like, keyed by anything other than a
/// scalar type name, is a call of a subscript result. The callee keeps the
/// subscript and the arguments stay in source order.
#[test]
fn a_non_class_name_base_with_a_value_key_lowers_to_an_expression_call() {
    let expr = last_expr("callbacks = {}\nk = 'a'\ncallbacks[k](1, 'x')\n");
    let HirExpr::ExprCall { callee, args } = &expr else {
        panic!("{expr:?}");
    };
    assert!(matches!(**callee, HirExpr::Subscript { .. }), "{callee:?}");
    assert_eq!(
        args,
        &vec![
            HirExpr::IntLiteral(1),
            HirExpr::StringLiteral("x".to_string())
        ]
    );
}

/// A bare-name base keyed by a scalar type name stays a generic class
/// instantiation even when the name is not a class: `handlers[int](x)` on
/// an object table is the documented residual `pycc_types` refuses.
#[test]
fn a_scalar_type_name_key_keeps_the_generic_instantiation_reading() {
    for key in ["int", "float", "bool", "str"] {
        let expr = last_expr(&format!("handlers = {{}}\nhandlers[{key}](1)\n"));
        assert!(
            matches!(&expr, HirExpr::GenericClassInstantiate { class, .. } if class == "handlers"),
            "{key}: {expr:?}"
        );
    }
}

/// A local class bound once keeps the located type-argument diagnostics:
/// its non-scalar key is still read as a type argument and refused here.
#[test]
fn a_local_class_keeps_its_located_type_argument_diagnostics() {
    assert_capability_error_message(
        &format!("{GENERIC_C}k = 1\nC[k](1)\n"),
        "a generic class type argument `k` is not supported yet",
    );
}

/// A class name that is rebound is not class-like (the binding-count rule
/// `SignatureTable` already uses), so its subscript call is an ordinary
/// call of a subscript result.
#[test]
fn a_rebound_class_name_is_not_class_like() {
    let expr = last_expr(&format!("{GENERIC_C}C = {{}}\nk = 1\nC[k](1)\n"));
    assert!(is_expr_call(&expr), "{expr:?}");
}

/// A name a project `from` import binds is class-like: an imported generic
/// class's bad type argument stays the located `C0001` it was before.
#[test]
fn a_project_imported_name_keeps_its_located_type_argument_diagnostics() {
    let diagnostic = lower_importing(
        Some(GENERIC_C),
        "from dep import C\n\n\nclass Foo:\n    pass\n\n\nC[Foo](1)\n",
    )
    .expect_err("an imported class's type argument is checked here");
    assert_eq!(diagnostic.code, "C0001");
    assert!(
        diagnostic
            .message
            .contains("a generic class type argument `Foo` is not supported yet"),
        "{diagnostic:?}"
    );
    assert!(diagnostic.span.is_some());
}

/// The class-like rule asks only whether the `from` import resolved to a
/// project module, not what the name is: a table a sibling module binds
/// (`from dep import TABLE`, here a dict; an `object` table under `--ext` is
/// the same name to this rule) is read as a class too, so `TABLE[k](x)`
/// takes the generic-instantiation path and is refused there -- the
/// documented residual, a refusal and never wrong code.
#[test]
fn a_project_imported_object_table_is_read_as_class_like() {
    let diagnostic = lower_importing(
        Some("TABLE = {\"len\": 1}\n"),
        "from dep import TABLE\n\nk = 'len'\nTABLE[k](1)\n",
    )
    .expect_err("a project-imported name is class-like");
    assert_eq!(diagnostic.code, "C0001");
    assert!(
        diagnostic
            .message
            .contains("a generic class type argument `k` is not supported yet"),
        "{diagnostic:?}"
    );
}

/// The same `from` import answered as a CPython module binds no class-like
/// name, so `Reg[k](x)` on it lowers to a call of a subscript result.
#[test]
fn a_foreign_imported_name_is_not_class_like() {
    let hir = lower_importing(None, "from lark import Reg\nk = 'a'\nReg[k](1)\n")
        .expect("a foreign import lowers");
    let Some(HirItem::TopLevelStmt(HirStmt::ExprStmt(expr))) = hir.items.last() else {
        panic!("{:?}", hir.items.last());
    };
    assert!(is_expr_call(expr), "{expr:?}");
}

/// A keyword argument keeps the subscript callee (since Part 8 of #1371 it
/// is wrapped in a `HirExpr::KeywordCall` that `pycc_types` admits only on
/// a CPython object), and a call of a call result is not a subscript callee
/// at all.
#[test]
fn a_keyword_call_is_deferred_and_a_call_result_callee_stays_refused() {
    let HirExpr::KeywordCall { call, .. } = last_expr("t = {}\nt['a'](x=1)\n") else {
        panic!("a keyword call of a subscript result is deferred to pycc_types");
    };
    assert!(is_expr_call(&call), "{call:?}");
    let module = pycc_parser_test_helper::parse("def g() -> int:\n    return 1\ng()()\n");
    let diagnostic = lower_checked(&module).expect_err("a call of a call result");
    assert_eq!(diagnostic.code, "C0001");
}

/// An argument that fails to lower propagates out of the expression-call
/// path just as it does out of the generic-instantiation path, and so does
/// a callee that fails to lower.
#[test]
fn a_lowering_error_in_the_callee_or_an_argument_propagates() {
    assert_capability_error_message(
        "t = {}\nt['a'](lambda: 1)\n",
        "expression kind not supported yet",
    );
    assert_capability_error_message(
        "t = {}\nt[lambda: 1](1)\n",
        "expression kind not supported yet",
    );
}

/// A walrus inside the callee or an argument is seen by
/// `contains_named_expr` through the new arm.
#[test]
fn contains_named_expr_reaches_the_callee_and_the_arguments() {
    let in_callee = last_expr("t = {}\nt[(k := 'a')](1)\n");
    let in_arg = last_expr("t = {}\nt['a']((y := 1))\n");
    let plain = last_expr("t = {}\nt['a'](1)\n");
    assert!(crate::expr::contains_named_expr(&in_callee));
    assert!(crate::expr::contains_named_expr(&in_arg));
    assert!(!crate::expr::contains_named_expr(&plain));
}

fn subscript_call(key: HirExpr, arg: HirExpr) -> HirExpr {
    HirExpr::ExprCall {
        callee: Box::new(HirExpr::Subscript {
            base: Box::new(HirExpr::Name("t".to_string())),
            index: Box::new(key),
        }),
        args: vec![arg],
    }
}

/// `killed_names` sees a walrus target inside the callee and inside an
/// argument of a call of a subscript result.
#[test]
fn killed_names_reaches_the_callee_and_the_arguments() {
    let walrus = |name: &str| HirExpr::NamedExpr {
        name: name.to_string(),
        value: Box::new(HirExpr::IntLiteral(1)),
    };
    let body = vec![HirStmt::ExprStmt(subscript_call(walrus("k"), walrus("y")))];
    let killed = crate::hir_module::killed_names(&body);
    assert!(killed.contains("k") && killed.contains("y"), "{killed:?}");
}

/// A comprehension's loop variable is renamed inside both the callee and
/// the arguments of a call of a subscript result.
#[test]
fn rename_name_in_expr_reaches_the_callee_and_the_arguments() {
    let name = |n: &str| HirExpr::Name(n.to_string());
    assert_eq!(
        rename_name_in_expr(subscript_call(name("x"), name("x")), "x", "_c"),
        subscript_call(name("_c"), name("_c"))
    );
}
