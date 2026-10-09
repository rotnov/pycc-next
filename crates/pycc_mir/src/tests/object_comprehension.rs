//! Part 1 of #1255: a list or set comprehension whose iterable is a CPython
//! object lowers to [`CompSource::Object`], binds its loop variable as an
//! object, and produces an object.

use crate::*;
use pycc_diag::Span;
use pycc_hir::{CompElt, HirComprehension, HirExpr, HirItem, HirModule, HirStmt, ImportBinding};

fn numpy_pi() -> HirExpr {
    HirExpr::AttrGet {
        base: Box::new(HirExpr::Name("numpy".to_string())),
        attr: "pi".to_string(),
    }
}

fn lower_discarded(expr: HirExpr) -> MirExpr {
    let hir = HirModule {
        items: vec![HirItem::TopLevelStmt(HirStmt::ExprStmt(expr))],
        imports: vec![ImportBinding::Foreign {
            local_name: "numpy".to_string(),
            module_path: "numpy".to_string(),
            from: None,
            site: pycc_hir::ForeignImportSite::Item(0),
            span: Span::new(0, 0),
        }],
        seeded_builtin_exception_classes: false,
        type_aliases: Vec::new(),
        class_defs: Vec::new(),
    };
    build(&hir)
        .items
        .into_iter()
        .find_map(|item| match item {
            MirItem::TopLevelStmt(MirStmt::ExprStmt(expr)) => Some(expr),
            _ => None,
        })
        .expect("the module has exactly one top-level statement")
}

fn mir_numpy_pi() -> MirExpr {
    MirExpr::ObjAttrGet {
        base: Box::new(MirExpr::Name {
            name: "numpy".to_string(),
            ty: Ty::Object,
        }),
        attr: "pi".to_string(),
        ty: Ty::Object,
    }
}

/// `{s for s in numpy.pi if s}`: the iterable is lowered in the enclosing
/// scope, the loop variable and the filter read it as an object, and the
/// comprehension's own type is `object` rather than a native `set`.
#[test]
fn a_comprehension_over_an_object_lowers_to_an_object_source() {
    let lowered = lower_discarded(HirExpr::Comprehension(Box::new(HirComprehension {
        var: "s".to_string(),
        iter: CompIter::Iterable(Box::new(numpy_pi())),
        cond: Some(HirExpr::Name("s".to_string())),
        elt: CompElt::Set(HirExpr::Name("s".to_string())),
    })));
    let var = MirExpr::Name {
        name: "s".to_string(),
        ty: Ty::Object,
    };
    let expected = MirComprehension {
        var: "s".to_string(),
        var_ty: Ty::Object,
        source: CompSource::Object(mir_numpy_pi()),
        cond: Some(var.clone()),
        elt: MirCompElt::Set(var, None),
    };
    assert_eq!(expected.ty(), Ty::Object);
    assert_eq!(lowered, MirExpr::Comprehension(Box::new(expected)));
    assert_eq!(lowered.ty(), Ty::Object);
}

/// A list comprehension over an object is an object too, whatever its
/// element type: `[1 for s in numpy.pi]` is not a `list[int]`.
#[test]
fn a_list_comprehension_over_an_object_is_an_object_whatever_its_element() {
    let lowered = lower_discarded(HirExpr::Comprehension(Box::new(HirComprehension {
        var: "s".to_string(),
        iter: CompIter::Iterable(Box::new(numpy_pi())),
        cond: None,
        elt: CompElt::List(HirExpr::IntLiteral(1)),
    })));
    assert_eq!(lowered.ty(), Ty::Object);
    let MirExpr::Comprehension(comp) = lowered else {
        panic!("expected a comprehension, got {lowered:?}");
    };
    assert_eq!(comp.source, CompSource::Object(mir_numpy_pi()));
    assert_eq!(comp.elt, MirCompElt::List(MirExpr::IntLiteral(1)));
}
