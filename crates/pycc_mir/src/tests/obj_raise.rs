//! Part 9 of #1371: `raise o` with a CPython object `o` lowers to
//! `MirStmt::ObjRaise`, and a native `raise` keeps its `MirStmt::Raise`.
//!
//! The fixtures are copied from `tests/obj_call.rs` rather than shared; they
//! are small and private there.

use crate::*;
use pycc_diag::Span;
use pycc_hir::{HirExpr, HirModule, HirStmt, ImportBinding};

fn foreign(local_name: &str) -> ImportBinding {
    ImportBinding::Foreign {
        local_name: local_name.to_string(),
        module_path: "errs".to_string(),
        from: None,
        site: pycc_hir::ForeignImportSite::Item(0),
        span: Span::new(0, 0),
    }
}

fn module(items: Vec<HirItem>, foreign_names: &[&str]) -> HirModule {
    HirModule {
        seeded_builtin_exception_classes: false,
        items,
        type_aliases: Vec::new(),
        imports: foreign_names.iter().map(|name| foreign(name)).collect(),
        class_defs: Vec::new(),
    }
}

fn raise(exc: HirExpr) -> HirStmt {
    HirStmt::Raise {
        exc: Some(exc),
        cause: None,
    }
}

fn call(callee: &str, args: Vec<HirExpr>) -> HirExpr {
    HirExpr::Call {
        callee: callee.to_string(),
        args,
    }
}

fn top_level_stmts(hir: &HirModule) -> Vec<MirStmt> {
    build(hir)
        .items
        .into_iter()
        .filter_map(|item| match item {
            MirItem::TopLevelStmt(stmt) => Some(stmt),
            _ => None,
        })
        .collect()
}

/// lark's `raise UnexpectedToken(token, expected)` shape: a call of a
/// foreign binding raises the call's `object` result.
#[test]
fn raising_a_foreign_call_lowers_to_obj_raise_of_the_obj_call() {
    let hir = module(
        vec![HirItem::TopLevelStmt(raise(call(
            "Boom",
            vec![HirExpr::IntLiteral(1)],
        )))],
        &["Boom"],
    );
    let stmts = top_level_stmts(&hir);
    let [MirStmt::ObjRaise { value }] = stmts.as_slice() else {
        panic!("expected one `ObjRaise`, got {stmts:?}");
    };
    assert!(matches!(value, MirExpr::ObjCall { .. }), "{value:?}");
}

/// A bare foreign class name is raised as the object itself; CPython
/// instantiates it at run time.
#[test]
fn raising_a_foreign_name_lowers_to_obj_raise_of_the_name() {
    let hir = module(
        vec![HirItem::TopLevelStmt(raise(HirExpr::Name(
            "Boom".to_string(),
        )))],
        &["Boom"],
    );
    let stmts = top_level_stmts(&hir);
    let [MirStmt::ObjRaise { value }] = stmts.as_slice() else {
        panic!("expected one `ObjRaise`, got {stmts:?}");
    };
    assert!(
        matches!(value, MirExpr::Name { name, ty: Ty::Object } if name == "Boom"),
        "{value:?}"
    );
}

/// A foreign binding that shadows a builtin exception name is the CPython
/// object, not the builtin pycc class: the scope decides, not the spelling.
#[test]
fn a_foreign_binding_shadowing_a_builtin_exception_is_raised_as_an_object() {
    let hir = module(
        vec![HirItem::TopLevelStmt(raise(call(
            "ValueError",
            vec![HirExpr::StringLiteral("bad".to_string())],
        )))],
        &["ValueError"],
    );
    let stmts = top_level_stmts(&hir);
    assert!(
        matches!(stmts.as_slice(), [MirStmt::ObjRaise { .. }]),
        "{stmts:?}"
    );
}

/// Without a foreign binding, the same call is the native constructor.
#[test]
fn a_native_constructor_call_still_lowers_to_raise() {
    let hir = module(
        vec![HirItem::TopLevelStmt(raise(call(
            "ValueError",
            vec![HirExpr::StringLiteral("bad".to_string())],
        )))],
        &[],
    );
    let stmts = top_level_stmts(&hir);
    assert!(
        matches!(
            stmts.as_slice(),
            [MirStmt::Raise {
                exception: MirExceptionValue::Constructed { .. },
                ..
            }]
        ),
        "{stmts:?}"
    );
}

/// A native exception reached by name, not by a call, keeps the existing
/// `Raise` of an `Existing` value; only an `object`-typed name becomes an
/// `ObjRaise`.
#[test]
fn a_native_exception_name_still_lowers_to_raise_of_the_existing_value() {
    let hir = module(
        vec![HirItem::Function {
            name: "f".to_string(),
            params: vec![(
                "e".to_string(),
                Ty::Instance(Box::new("ValueError".to_string())),
            )],
            return_ty: Ty::None,
            body: vec![raise(HirExpr::Name("e".to_string()))],
        }],
        &[],
    );
    let mir = build(&hir);
    let body = mir
        .items
        .iter()
        .find_map(|item| match item {
            MirItem::Function { name, body, .. } if name == "f" => Some(body),
            _ => None,
        })
        .expect("`f` must lower");
    assert!(
        body.iter().any(|stmt| matches!(
            stmt,
            MirStmt::Raise {
                exception: MirExceptionValue::Existing(MirExpr::Name { name, .. }),
                ..
            } if name == "e"
        )),
        "{body:?}"
    );
}

/// In a function body the object raise carries no frame name (CPython
/// owns the traceback), and the frame-naming pass leaves it alone.
#[test]
fn an_obj_raise_in_a_function_body_is_untouched_by_frame_naming() {
    let hir = module(
        vec![HirItem::Function {
            name: "f".to_string(),
            params: Vec::new(),
            return_ty: Ty::None,
            body: vec![raise(call("Boom", Vec::new()))],
        }],
        &["Boom"],
    );
    let mir = build(&hir);
    let body = mir
        .items
        .iter()
        .find_map(|item| match item {
            MirItem::Function { name, body, .. } if name == "f" => Some(body),
            _ => None,
        })
        .expect("`f` must lower");
    assert!(
        body.iter()
            .any(|stmt| matches!(stmt, MirStmt::ObjRaise { .. })),
        "{body:?}"
    );
}
