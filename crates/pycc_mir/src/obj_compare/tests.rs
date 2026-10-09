//! Lowering of comparisons and `isinstance` with a CPython object operand
//! (Part 1 of #1371), of membership in and slices of one (Part 2b), and of
//! deleting a slice of one (Part 2c), and of storing and deleting one's
//! attribute (#1457), exercised from real HIR with `numpy` bound as a foreign import.

use super::lower_object_isinstance;
use crate::*;
use pycc_diag::Span;
use pycc_hir::{CmpOpKind, HirExpr, HirItem, HirModule, HirStmt, ImportBinding};

fn numpy() -> HirExpr {
    HirExpr::Name("numpy".to_string())
}

fn numpy_attr(attr: &str) -> HirExpr {
    HirExpr::AttrGet {
        base: Box::new(numpy()),
        attr: attr.to_string(),
    }
}

/// The lowered form of `expr` evaluated and discarded as the single
/// statement of a module that imports `numpy` first.
fn lower_discarded(expr: HirExpr) -> MirExpr {
    lower_discarded_with(expr, Vec::new())
}

/// [`lower_discarded`] in a module that also defines `class_defs`.
fn lower_discarded_with(
    expr: HirExpr,
    class_defs: Vec<(String, pycc_hir::HirClassDef)>,
) -> MirExpr {
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
        class_defs,
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

fn compare(op: CmpOpKind, left: HirExpr, right: HirExpr) -> HirExpr {
    HirExpr::Compare {
        op,
        left: Box::new(left),
        right: Box::new(right),
    }
}

fn isinstance(value: HirExpr, class: HirExpr) -> HirExpr {
    HirExpr::Call {
        callee: "isinstance".to_string(),
        args: vec![value, class],
    }
}

#[test]
fn a_rich_comparison_with_an_object_operand_is_an_object_valued_obj_compare() {
    for (left, right) in [
        (numpy_attr("pi"), HirExpr::IntLiteral(1)),
        (HirExpr::FloatLiteral(1.5), numpy_attr("pi")),
    ] {
        let lowered = lower_discarded(compare(CmpOpKind::Lt, left, right));
        assert!(
            matches!(
                lowered,
                MirExpr::ObjCompare {
                    op: CmpOpKind::Lt,
                    ..
                }
            ),
            "{lowered:?}"
        );
        assert_eq!(lowered.ty(), Ty::Object);
    }
}

#[test]
fn an_identity_test_on_an_object_is_a_bool_valued_obj_compare() {
    for op in [CmpOpKind::Is, CmpOpKind::IsNot] {
        let lowered = lower_discarded(compare(op, numpy_attr("pi"), HirExpr::NoneLiteral));
        let MirExpr::ObjCompare { right, .. } = &lowered else {
            panic!("expected an `ObjCompare`: {lowered:?}");
        };
        assert_eq!(**right, MirExpr::NoneLiteral);
        assert_eq!(lowered.ty(), Ty::Bool);
    }
}

#[test]
fn a_native_comparison_keeps_its_native_node() {
    let lowered = lower_discarded(compare(
        CmpOpKind::Eq,
        HirExpr::IntLiteral(1),
        HirExpr::IntLiteral(2),
    ));
    assert!(matches!(lowered, MirExpr::Compare { .. }), "{lowered:?}");
}

#[test]
fn isinstance_on_an_object_is_a_run_time_test_never_a_folded_constant() {
    let lowered = lower_discarded(isinstance(
        numpy_attr("pi"),
        HirExpr::Name("float".to_string()),
    ));
    assert!(
        matches!(
            lowered,
            MirExpr::ObjIsInstance {
                class: ObjIsInstanceClass::Builtin(ObjBuiltinClass::Float),
                ..
            }
        ),
        "{lowered:?}"
    );
    assert_eq!(lowered.ty(), Ty::Bool);

    let lowered = lower_discarded(isinstance(numpy_attr("pi"), numpy_attr("ndarray")));
    let MirExpr::ObjIsInstance {
        class: ObjIsInstanceClass::Object(class),
        ..
    } = &lowered
    else {
        panic!("expected an object-class `ObjIsInstance`: {lowered:?}");
    };
    assert!(matches!(**class, MirExpr::ObjAttrGet { ref attr, .. } if attr == "ndarray"));
}

#[test]
fn every_builtin_class_name_maps_to_its_own_shim_selector() {
    let codes: Vec<u64> = ["int", "str", "float", "bool"]
        .into_iter()
        .map(|name| ObjBuiltinClass::from_name(name).expect(name).shim_code())
        .collect();
    assert_eq!(codes, [0, 1, 2, 3]);
    // Part 7 of #1371: the three container classes take the next selectors.
    let codes: Vec<u64> = ["list", "dict", "tuple"]
        .into_iter()
        .map(|name| ObjBuiltinClass::from_name(name).expect(name).shim_code())
        .collect();
    assert_eq!(codes, [4, 5, 6]);
    assert_eq!(ObjBuiltinClass::from_name("set"), None);
}

/// A plain (non-exception) class `name` with the MRO `mro`, cloned from a
/// seeded definition the way `exception_isinstance_tests` builds one.
fn plain_class(name: &str, mro: &[&str]) -> pycc_hir::HirClassDef {
    let mut def = pycc_hir::builtin_exception_class_defs()
        .into_iter()
        .find(|(class, _)| class == "ValueError")
        .expect("ValueError is seeded")
        .1;
    def.name = name.to_string();
    def.bases = mro
        .get(1)
        .map(|base| base.to_string())
        .into_iter()
        .collect();
    def.mro = mro.iter().map(|class| class.to_string()).collect();
    def.exception_type_tag = None;
    def
}

/// Part 7 of #1371: a class compiled in this module is carried by name,
/// for the generated published-family test; a name that is neither a
/// builtin nor a compiled class stays an evaluated object class.
#[test]
fn isinstance_against_a_compiled_class_carries_the_class_name() {
    let lowered = lower_discarded_with(
        isinstance(numpy_attr("pi"), HirExpr::Name("Base".to_string())),
        vec![
            ("Base".to_string(), plain_class("Base", &["Base", "object"])),
            (
                "Derived".to_string(),
                plain_class("Derived", &["Derived", "Base", "object"]),
            ),
        ],
    );
    assert!(
        matches!(
            &lowered,
            MirExpr::ObjIsInstance {
                class: ObjIsInstanceClass::Compiled(name),
                ..
            } if name == "Base"
        ),
        "{lowered:?}"
    );
    assert_eq!(lowered.ty(), Ty::Bool);
}

/// #1476: a module-level class spelled like a builtin shadows the builtin,
/// as in CPython, so the guard tests the compiled class.
#[test]
fn a_compiled_class_spelled_like_a_builtin_shadows_the_builtin() {
    let lowered = lower_discarded_with(
        isinstance(numpy_attr("pi"), HirExpr::Name("int".to_string())),
        vec![("int".to_string(), plain_class("int", &["int", "object"]))],
    );
    assert!(
        matches!(
            &lowered,
            MirExpr::ObjIsInstance {
                class: ObjIsInstanceClass::Compiled(name),
                ..
            } if name == "int"
        ),
        "{lowered:?}"
    );
}

/// A local spelled like a compiled class shadows it: the class argument is
/// then the evaluated local, not the published family.
#[test]
fn a_local_shadowing_a_compiled_class_is_an_evaluated_object_class() {
    let classes: HashMap<String, pycc_hir::HirClassDef> =
        [("Base".to_string(), plain_class("Base", &["Base", "object"]))]
            .into_iter()
            .collect();
    let scopes = vec![[("Base".to_string(), Ty::Object)].into_iter().collect()];
    let value = lower_discarded(numpy_attr("pi"));
    let lowered = lower_object_isinstance(
        value,
        &HirExpr::Name("Base".to_string()),
        &scopes,
        &classes,
        None,
    );
    assert!(
        matches!(
            &lowered,
            MirExpr::ObjIsInstance {
                class: ObjIsInstanceClass::Object(_),
                ..
            }
        ),
        "{lowered:?}"
    );
}

/// A local spelled like a builtin class shadows it in the same way.
#[test]
fn a_local_shadowing_a_builtin_class_is_an_evaluated_object_class() {
    let scopes = vec![[("list".to_string(), Ty::Object)].into_iter().collect()];
    let lowered = lower_object_isinstance(
        lower_discarded(numpy_attr("pi")),
        &HirExpr::Name("list".to_string()),
        &scopes,
        &HashMap::new(),
        None,
    );
    assert!(
        matches!(
            &lowered,
            MirExpr::ObjIsInstance {
                class: ObjIsInstanceClass::Object(_),
                ..
            }
        ),
        "{lowered:?}"
    );
}

/// A walrus can hide in either operand of a comparison, and in either
/// argument of `isinstance`; a binding missed there is a local codegen
/// never allocates storage for.
#[test]
fn named_expr_bindings_are_collected_from_every_operand() {
    let walrus = |name: &str, value: HirExpr| HirExpr::NamedExpr {
        name: name.to_string(),
        value: Box::new(value),
    };
    let mut out = Vec::new();
    lower_discarded(compare(
        CmpOpKind::Eq,
        walrus("a", numpy_attr("pi")),
        walrus("b", HirExpr::IntLiteral(1)),
    ))
    .collect_named_expr_bindings(&mut out);
    lower_discarded(isinstance(
        walrus("c", numpy_attr("pi")),
        walrus("d", numpy_attr("ndarray")),
    ))
    .collect_named_expr_bindings(&mut out);
    lower_discarded(isinstance(
        walrus("e", numpy_attr("pi")),
        HirExpr::Name("int".to_string()),
    ))
    .collect_named_expr_bindings(&mut out);
    let names: Vec<&str> = out.iter().map(|(name, _)| name.as_str()).collect();
    assert_eq!(names, ["a", "b", "c", "d", "e"]);
}

fn slice(
    base: HirExpr,
    start: Option<HirExpr>,
    stop: Option<HirExpr>,
    step: Option<HirExpr>,
) -> HirExpr {
    HirExpr::Slice {
        base: Box::new(base),
        start: start.map(Box::new),
        stop: stop.map(Box::new),
        step: step.map(Box::new),
    }
}

/// Part 2b of #1371: membership in an object container is its own
/// bool-valued node, item first, with `not in` as `negate`.
#[test]
fn membership_in_an_object_container_is_a_bool_valued_obj_contains() {
    for (op, expected_negate) in [(CmpOpKind::In, false), (CmpOpKind::NotIn, true)] {
        let lowered = lower_discarded(compare(op, HirExpr::IntLiteral(1), numpy_attr("pi")));
        let MirExpr::ObjContains {
            negate,
            item,
            container,
        } = &lowered
        else {
            panic!("expected an `ObjContains`: {lowered:?}");
        };
        assert_eq!(*negate, expected_negate);
        assert_eq!(**item, MirExpr::IntLiteral(1));
        assert!(matches!(**container, MirExpr::ObjAttrGet { .. }));
        assert_eq!(lowered.ty(), Ty::Bool);
    }
}

/// Part 2b of #1371: a slice of an object is its own object-valued node,
/// whatever bounds are present; a native base keeps `MirExpr::Slice`.
#[test]
fn a_slice_of_an_object_is_an_object_valued_obj_slice() {
    let lowered = lower_discarded(slice(
        numpy_attr("pi"),
        Some(HirExpr::IntLiteral(1)),
        None,
        Some(HirExpr::IntLiteral(2)),
    ));
    let MirExpr::ObjSlice {
        start, stop, step, ..
    } = &lowered
    else {
        panic!("expected an `ObjSlice`: {lowered:?}");
    };
    assert_eq!(start.as_deref(), Some(&MirExpr::IntLiteral(1)));
    assert_eq!(stop.as_deref(), None);
    assert_eq!(step.as_deref(), Some(&MirExpr::IntLiteral(2)));
    assert_eq!(lowered.ty(), Ty::Object);

    let native = lower_discarded(slice(
        HirExpr::ListLiteral(vec![HirExpr::IntLiteral(1)]),
        None,
        Some(HirExpr::IntLiteral(1)),
        None,
    ));
    assert!(matches!(native, MirExpr::Slice { .. }), "{native:?}");
}

/// A walrus can hide in either operand of a membership test and in the
/// base or any bound of an object slice.
#[test]
fn named_expr_bindings_are_collected_from_membership_and_slice_operands() {
    let walrus = |name: &str, value: HirExpr| HirExpr::NamedExpr {
        name: name.to_string(),
        value: Box::new(value),
    };
    let mut out = Vec::new();
    lower_discarded(compare(
        CmpOpKind::In,
        walrus("a", HirExpr::IntLiteral(1)),
        walrus("b", numpy_attr("pi")),
    ))
    .collect_named_expr_bindings(&mut out);
    lower_discarded(slice(
        walrus("c", numpy_attr("pi")),
        Some(walrus("d", HirExpr::IntLiteral(1))),
        Some(walrus("e", HirExpr::IntLiteral(2))),
        Some(walrus("f", HirExpr::IntLiteral(1))),
    ))
    .collect_named_expr_bindings(&mut out);
    let names: Vec<&str> = out.iter().map(|(name, _)| name.as_str()).collect();
    assert_eq!(names, ["a", "b", "c", "d", "e", "f"]);
}

/// Part 2d of #1371: a list display the empty-container pre-pass resolved
/// to an object slot lowers to its own object-valued node, elements in
/// source order, and a walrus inside an element still binds its target.
#[test]
fn an_object_list_display_is_an_object_valued_obj_list() {
    let lowered = lower_discarded(HirExpr::ObjectList(vec![
        HirExpr::IntLiteral(1),
        HirExpr::StringLiteral("a".to_string()),
        numpy_attr("pi"),
    ]));
    let MirExpr::ObjList { elements } = &lowered else {
        panic!("expected an `ObjList`: {lowered:?}");
    };
    assert_eq!(elements.len(), 3);
    assert_eq!(elements[0], MirExpr::IntLiteral(1));
    assert!(matches!(elements[2], MirExpr::ObjAttrGet { .. }));
    assert_eq!(lowered.ty(), Ty::Object);
    assert_eq!(
        lower_discarded(HirExpr::ObjectList(Vec::new())).ty(),
        Ty::Object
    );

    let mut out = Vec::new();
    lower_discarded(HirExpr::ObjectList(vec![HirExpr::NamedExpr {
        name: "n".to_string(),
        value: Box::new(HirExpr::IntLiteral(3)),
    }]))
    .collect_named_expr_bindings(&mut out);
    assert_eq!(out, [("n".to_string(), Ty::Int)]);
}

/// Part 2c of #1371: `del o[a:b:c]` lowers to one `ObjDelSlice`, in a
/// module body and in a function body (which `set_frame_function` and the
/// receiver verifier both walk).
#[test]
fn a_slice_delete_of_an_object_is_one_obj_del_slice() {
    let del = |start: Option<HirExpr>| HirStmt::DeleteSlice {
        base: Box::new(numpy_attr("pi")),
        start: start.map(Box::new),
        stop: None,
        step: Some(Box::new(HirExpr::IntLiteral(2))),
        span: Span::new(0, 0),
    };
    let hir = HirModule {
        items: vec![
            HirItem::TopLevelStmt(del(Some(HirExpr::IntLiteral(1)))),
            HirItem::Function {
                name: "f".to_string(),
                params: vec![],
                return_ty: pycc_hir::Ty::None,
                body: vec![del(None), HirStmt::Return(None)],
            },
        ],
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
    let mir = build(&hir);
    let deletes: Vec<&MirStmt> = mir
        .items
        .iter()
        .flat_map(|item| match item {
            MirItem::TopLevelStmt(stmt) => std::slice::from_ref(stmt),
            MirItem::Function { body, .. } => body.as_slice(),
            _ => &[],
        })
        .filter(|stmt| matches!(stmt, MirStmt::ObjDelSlice { .. }))
        .collect();
    let [
        MirStmt::ObjDelSlice {
            base,
            start: Some(MirExpr::IntLiteral(1)),
            stop: None,
            step: Some(MirExpr::IntLiteral(2)),
        },
        MirStmt::ObjDelSlice { start: None, .. },
    ] = deletes.as_slice()
    else {
        panic!("expected two `ObjDelSlice`s: {deletes:?}");
    };
    assert!(matches!(base, MirExpr::ObjAttrGet { .. }), "{base:?}");
}

/// #1457: `o.x = v` on an object lowers to one `ObjAttrSet` and `del o.x`
/// to one `ObjDelAttr`, in a module body and in a function body (which
/// `set_frame_function` and the receiver verifier both walk).
#[test]
fn an_attribute_store_and_delete_on_an_object_lower_to_obj_attr_stmts() {
    let store = || HirStmt::AttrSet {
        base: numpy_attr("pi"),
        attr: "n".to_string(),
        value: HirExpr::IntLiteral(7),
    };
    let del = || HirStmt::DeleteAttr {
        base: Box::new(numpy_attr("pi")),
        attr: "m".to_string(),
        span: Span::new(0, 0),
    };
    let hir = HirModule {
        items: vec![
            HirItem::TopLevelStmt(store()),
            HirItem::TopLevelStmt(del()),
            HirItem::Function {
                name: "f".to_string(),
                params: vec![],
                return_ty: pycc_hir::Ty::None,
                body: vec![store(), del(), HirStmt::Return(None)],
            },
        ],
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
    let mir = build(&hir);
    let stmts: Vec<&MirStmt> = mir
        .items
        .iter()
        .flat_map(|item| match item {
            MirItem::TopLevelStmt(stmt) => std::slice::from_ref(stmt),
            MirItem::Function { body, .. } => body.as_slice(),
            _ => &[],
        })
        .filter(|stmt| {
            matches!(
                stmt,
                MirStmt::ObjAttrSet { .. } | MirStmt::ObjDelAttr { .. }
            )
        })
        .collect();
    assert_eq!(stmts.len(), 4, "{stmts:?}");
    for pair in stmts.chunks(2) {
        let [
            MirStmt::ObjAttrSet {
                base: set_base,
                attr: set_attr,
                value: MirExpr::IntLiteral(7),
            },
            MirStmt::ObjDelAttr {
                base: del_base,
                attr: del_attr,
            },
        ] = pair
        else {
            panic!("expected a store then a delete: {pair:?}");
        };
        assert_eq!((set_attr.as_str(), del_attr.as_str()), ("n", "m"));
        assert!(
            matches!(set_base, MirExpr::ObjAttrGet { .. }),
            "{set_base:?}"
        );
        assert!(
            matches!(del_base, MirExpr::ObjAttrGet { .. }),
            "{del_base:?}"
        );
    }
}
