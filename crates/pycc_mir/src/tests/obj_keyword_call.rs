//! Part 8 of #1371: a keyword call on a CPython object
//! (`HirExpr::KeywordCall`) lowers to `MirExpr::ObjKeywordCall`.
//!
//! The fixtures are copied from `tests/obj_call.rs` rather than shared; they
//! are small and private there.

use crate::*;
use pycc_diag::Span;
use pycc_hir::{HirExpr, HirModule, HirStmt, ImportBinding};

/// A module whose body is `items`, below one foreign import of `product`.
fn module_with_items(items: Vec<HirItem>) -> HirModule {
    HirModule {
        seeded_builtin_exception_classes: false,
        items,
        type_aliases: Vec::new(),
        imports: vec![ImportBinding::Foreign {
            local_name: "product".to_string(),
            module_path: "product".to_string(),
            from: None,
            site: pycc_hir::ForeignImportSite::Item(0),
            span: Span::new(0, 0),
        }],
        class_defs: Vec::new(),
    }
}

/// The lowered form of every top-level expression statement in `hir`; the
/// lowering runs `verify_receiver::verify` over the module too.
fn discarded_exprs(hir: &HirModule) -> Vec<MirExpr> {
    build(hir)
        .items
        .into_iter()
        .filter_map(|item| match item {
            MirItem::TopLevelStmt(MirStmt::ExprStmt(expr)) => Some(expr),
            _ => None,
        })
        .collect()
}

fn keyword_call(call: HirExpr, keywords: Vec<(&str, HirExpr)>) -> HirExpr {
    HirExpr::KeywordCall {
        call: Box::new(call),
        keywords: keywords
            .into_iter()
            .map(|(name, value)| (name.to_string(), value))
            .collect(),
        span: Span::new(0, 0),
    }
}

fn product() -> HirExpr {
    HirExpr::Name("product".to_string())
}

fn walrus(name: &str) -> HirExpr {
    HirExpr::NamedExpr {
        name: name.to_string(),
        value: Box::new(HirExpr::IntLiteral(1)),
    }
}

/// A direct call and a method call each lower their positional half to the
/// keyword-free node and keep the keywords in source order; the node is
/// typed `object`.
#[test]
fn a_keyword_call_lowers_to_obj_keyword_call() {
    let hir = module_with_items(vec![
        HirItem::TopLevelStmt(HirStmt::ExprStmt(keyword_call(
            HirExpr::Call {
                callee: "product".to_string(),
                args: vec![HirExpr::StringLiteral("ab".to_string())],
            },
            vec![
                ("repeat", HirExpr::IntLiteral(2)),
                ("x", HirExpr::BoolLiteral(true)),
            ],
        ))),
        HirItem::TopLevelStmt(HirStmt::ExprStmt(keyword_call(
            HirExpr::MethodCall {
                base: Box::new(product()),
                method: "m".to_string(),
                args: Vec::new(),
            },
            vec![("k", product())],
        ))),
    ]);
    let exprs = discarded_exprs(&hir);
    let [
        MirExpr::ObjKeywordCall(direct),
        MirExpr::ObjKeywordCall(method),
    ] = exprs.as_slice()
    else {
        panic!("expected two `ObjKeywordCall`s, got {exprs:?}");
    };
    assert_eq!(exprs[0].ty(), Ty::Object);
    assert!(
        matches!(&direct.call, MirExpr::ObjCall { args, .. }
            if matches!(args.as_slice(), [MirExpr::StringLiteral(s)] if s == "ab")),
        "{direct:?}"
    );
    assert_eq!(direct.names, vec!["repeat".to_string(), "x".to_string()]);
    assert!(
        matches!(
            direct.values.as_slice(),
            [MirExpr::IntLiteral(2), MirExpr::BoolLiteral(true)]
        ),
        "{direct:?}"
    );
    assert!(
        matches!(&method.call, MirExpr::ObjMethodCall { method, args, .. }
            if method == "m" && args.is_empty()),
        "{method:?}"
    );
    assert_eq!(method.names, vec!["k".to_string()]);
    assert!(
        matches!(&method.values[0], MirExpr::Name { ty: Ty::Object, .. }),
        "{method:?}"
    );
}

/// A walrus in the positional half or in a keyword value binds for the
/// next statement, through both the statement's pre-bind walk and
/// `collect_named_expr_bindings`.
#[test]
fn a_walrus_in_a_keyword_call_binds_for_the_next_statement() {
    let hir = module_with_items(vec![
        HirItem::TopLevelStmt(HirStmt::ExprStmt(keyword_call(
            HirExpr::Call {
                callee: "product".to_string(),
                args: vec![walrus("a")],
            },
            vec![("k", walrus("b"))],
        ))),
        HirItem::TopLevelStmt(HirStmt::ExprStmt(HirExpr::Name("a".to_string()))),
        HirItem::TopLevelStmt(HirStmt::ExprStmt(HirExpr::Name("b".to_string()))),
    ]);
    let exprs = discarded_exprs(&hir);
    let mut out = Vec::new();
    exprs[0].collect_named_expr_bindings(&mut out);
    assert_eq!(
        out,
        vec![("a".to_string(), Ty::Int), ("b".to_string(), Ty::Int)]
    );
    assert!(matches!(&exprs[1], MirExpr::Name { name, ty: Ty::Int } if name == "a"));
    assert!(matches!(&exprs[2], MirExpr::Name { name, ty: Ty::Int } if name == "b"));
}

/// The same walrus inside a function body, where the pre-bind walk
/// allocates the local before the call is lowered.
#[test]
fn a_walrus_in_a_keyword_value_binds_in_a_function_body() {
    let hir = module_with_items(vec![HirItem::Function {
        name: "f".to_string(),
        params: Vec::new(),
        return_ty: Ty::Int,
        body: vec![
            HirStmt::ExprStmt(keyword_call(
                HirExpr::MethodCall {
                    base: Box::new(product()),
                    method: "m".to_string(),
                    args: Vec::new(),
                },
                vec![("k", walrus("n"))],
            )),
            HirStmt::Return(Some(HirExpr::Name("n".to_string()))),
        ],
    }]);
    let mir = build(&hir);
    let body = mir
        .items
        .iter()
        .find_map(|item| match item {
            MirItem::Function { body, .. } => Some(body),
            _ => None,
        })
        .expect("the function item lowers");
    let rendered = format!("{body:?}");
    assert!(rendered.contains("ObjKeywordCall"), "{rendered}");
    assert!(
        matches!(body.last(), Some(MirStmt::Return(Some(MirExpr::Name { name, ty: Ty::Int }))) if name == "n"),
        "{rendered}"
    );
}
