//! Part 1 of #891: lowering of a tuple-unpacking assignment's value,
//! `HirExpr::Unpack` -- a CPython object becomes `MirExpr::ObjUnpack`, a
//! native tuple passes through unchanged.

use crate::*;
use pycc_diag::Span;
use pycc_hir::{HirExpr, HirModule, HirStmt, ImportBinding};

/// A module whose body is `stmts`, below one foreign import of `numpy`.
fn module_with_stmts(stmts: Vec<HirStmt>) -> HirModule {
    HirModule {
        seeded_builtin_exception_classes: false,
        items: stmts.into_iter().map(HirItem::TopLevelStmt).collect(),
        type_aliases: Vec::new(),
        imports: vec![ImportBinding::Foreign {
            local_name: "numpy".to_string(),
            module_path: "numpy".to_string(),
            from: None,
            site: pycc_hir::ForeignImportSite::Item(0),
            span: Span::new(0, 0),
        }],
        class_defs: Vec::new(),
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

fn unpack(value: HirExpr, arity: usize) -> HirExpr {
    HirExpr::Unpack {
        value: Box::new(value),
        arity,
    }
}

fn numpy_pi() -> HirExpr {
    HirExpr::AttrGet {
        base: Box::new(HirExpr::Name("numpy".to_string())),
        attr: "pi".to_string(),
    }
}

fn assign(target: &str, value: HirExpr) -> HirStmt {
    HirStmt::Assign {
        target: target.to_string(),
        value,
    }
}

/// `a, b = numpy.pi` binds the temporary to an `ObjUnpack` of arity 2, and
/// each target to an object subscript of that temporary.
#[test]
fn an_object_value_lowers_to_an_obj_unpack() {
    let stmts = top_level_stmts(&module_with_stmts(vec![
        assign("0unpack_0", unpack(numpy_pi(), 2)),
        assign(
            "a",
            HirExpr::Subscript {
                base: Box::new(HirExpr::Name("0unpack_0".to_string())),
                index: Box::new(HirExpr::IntLiteral(0)),
            },
        ),
    ]));
    let [
        MirStmt::Assign { value: temp, .. },
        MirStmt::Assign { value: element, .. },
    ] = stmts.as_slice()
    else {
        panic!("expected two assignments, got {stmts:?}");
    };
    let MirExpr::ObjUnpack { value, arity: 2 } = temp else {
        panic!("expected an arity-2 ObjUnpack, got {temp:?}");
    };
    assert!(matches!(**value, MirExpr::ObjAttrGet { .. }), "{value:?}");
    assert_eq!(temp.ty(), Ty::Object);
    assert!(
        matches!(element, MirExpr::ObjSubscript { .. }),
        "{element:?}"
    );
}

/// A native tuple's arity was checked statically: the value is the tuple
/// itself, with its own tuple type.
#[test]
fn a_native_tuple_value_passes_through() {
    let stmts = top_level_stmts(&module_with_stmts(vec![assign(
        "0unpack_0",
        unpack(
            HirExpr::TupleLiteral(vec![HirExpr::IntLiteral(1), HirExpr::FloatLiteral(2.5)]),
            2,
        ),
    )]));
    let [MirStmt::Assign { value, .. }] = stmts.as_slice() else {
        panic!("expected one assignment, got {stmts:?}");
    };
    assert!(matches!(value, MirExpr::TupleLiteral(_)), "{value:?}");
    assert_eq!(value.ty(), Ty::Tuple(Box::new(vec![Ty::Int, Ty::Float])));
}

/// A walrus can hide only in the unpacked value, the node's one child.
#[test]
fn named_expr_bindings_are_collected_from_the_unpacked_value() {
    let node = MirExpr::ObjUnpack {
        value: Box::new(MirExpr::NamedExpr {
            name: "o".to_string(),
            value: Box::new(MirExpr::IntLiteral(1)),
            ty: Ty::Int,
        }),
        arity: 2,
    };
    let mut out = Vec::new();
    node.collect_named_expr_bindings(&mut out);
    let names: Vec<&str> = out.iter().map(|(name, _)| name.as_str()).collect();
    assert_eq!(names, ["o"]);
}
