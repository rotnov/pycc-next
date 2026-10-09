//! Foreign-import lowering (`MirItem::ForeignImport`, Part 1 of #1026).
//!
//! PR 1b carried the same information as a `MirModule::foreign_imports`
//! side table; #1080's own ordering requirement replaced it with an item
//! spliced into `MirModule::items` at the position the `import` statement
//! occupied, so codegen emits the import call in source order rather than
//! hoisted. These tests pin both halves: a compile-time-only binding
//! contributes no item at all, and a foreign binding contributes exactly
//! one, at its recorded index.

use crate::*;
use pycc_diag::Span;
use pycc_hir::{HirModule, ImportBinding, ProjectBindingKind};

fn module_with_imports(imports: Vec<ImportBinding>) -> HirModule {
    HirModule {
        seeded_builtin_exception_classes: false,
        items: Vec::new(),
        type_aliases: Vec::new(),
        imports,
        class_defs: Vec::new(),
    }
}

/// A module whose body is `n` `print`-free top-level statements, each a
/// distinct assignment, so an item's index is readable off its target.
fn module_with_stmts(n: usize, imports: Vec<ImportBinding>) -> HirModule {
    let items = (0..n)
        .map(|index| {
            HirItem::TopLevelStmt(pycc_hir::HirStmt::Assign {
                target: format!("v{index}"),
                value: pycc_hir::HirExpr::IntLiteral(index as i64),
            })
        })
        .collect();
    HirModule {
        items,
        ..module_with_imports(imports)
    }
}

fn foreign(local_name: &str, item_index: usize) -> ImportBinding {
    ImportBinding::Foreign {
        local_name: local_name.to_string(),
        module_path: local_name.to_string(),
        from: None,
        site: pycc_hir::ForeignImportSite::Item(item_index),
        span: Span::new(0, 0),
    }
}

/// The `local_name` of every `ForeignImport` in `items`, paired with its
/// index, so a test can state ordering rather than merely membership.
fn foreign_items(module: &MirModule) -> Vec<(usize, String)> {
    module
        .items
        .iter()
        .enumerate()
        .filter_map(|(index, item)| match item {
            MirItem::ForeignImport { local_name, .. } => Some((index, local_name.clone())),
            _ => None,
        })
        .collect()
}

#[test]
fn a_module_without_imports_lowers_no_foreign_import_item() {
    let mir = build(&module_with_imports(Vec::new()));
    assert!(foreign_items(&mir).is_empty());
}

#[test]
fn drops_the_stdlib_module_and_symbol_bindings() {
    let hir = module_with_imports(vec![
        ImportBinding::Module {
            local_name: "math".to_string(),
            module: pycc_std::StdModule::Math,
        },
        ImportBinding::Symbol {
            local_name: "sqrt".to_string(),
            module: pycc_std::StdModule::Math,
            symbol: pycc_std::resolve_symbol(pycc_std::StdModule::Math, "sqrt")
                .expect("`math.sqrt` is a registered stdlib symbol"),
        },
    ]);
    assert!(foreign_items(&build(&hir)).is_empty());
}

#[test]
fn drops_a_project_binding() {
    let hir = module_with_imports(vec![ImportBinding::Project {
        local_name: "helper".to_string(),
        module_path: "pkg.helper".to_string(),
        kind: ProjectBindingKind::Function,
    }]);
    assert!(foreign_items(&build(&hir)).is_empty());
}

#[test]
fn a_foreign_import_lands_at_its_recorded_position() {
    // `v0 = 0` / `import numpy` / `v1 = 1`: the import is recorded at
    // index 1 and must land between the two statements, never hoisted.
    let hir = module_with_stmts(2, vec![foreign("numpy", 1)]);
    let mir = build(&hir);
    assert_eq!(foreign_items(&mir), vec![(1, "numpy".to_string())]);
    assert_eq!(mir.items.len(), 3);
}

#[test]
fn a_leading_foreign_import_lands_first() {
    let hir = module_with_stmts(2, vec![foreign("numpy", 0)]);
    assert_eq!(foreign_items(&build(&hir)), vec![(0, "numpy".to_string())]);
}

#[test]
fn a_trailing_foreign_import_lands_last() {
    let hir = module_with_stmts(2, vec![foreign("numpy", 2)]);
    assert_eq!(foreign_items(&build(&hir)), vec![(2, "numpy".to_string())]);
}

#[test]
fn two_foreign_imports_straddling_a_statement_keep_their_order() {
    // `import a` / `v0 = 0` / `import b`: recorded at 0 and 1, and the
    // running insertion offset is what puts `b` after the statement
    // rather than immediately after `a`.
    let hir = module_with_stmts(1, vec![foreign("a", 0), foreign("b", 1)]);
    let mir = build(&hir);
    assert_eq!(
        foreign_items(&mir),
        vec![(0, "a".to_string()), (2, "b".to_string())]
    );
    assert!(matches!(mir.items[1], MirItem::TopLevelStmt(_)));
}

/// #1278: a from-import's binding is spliced with its `from` intact, so
/// codegen receives the attribute name, the whole fromlist and the index.
#[test]
fn a_foreign_from_import_carries_its_from_import_into_the_item() {
    let from = pycc_hir::FromImport {
        name: "chain".to_string(),
        fromlist: vec!["product".to_string(), "chain".to_string()],
        index: 1,
        level: 0,
    };
    let hir = module_with_stmts(
        1,
        vec![ImportBinding::Foreign {
            local_name: "chain".to_string(),
            module_path: "itertools".to_string(),
            from: Some(from.clone()),
            site: pycc_hir::ForeignImportSite::Item(1),
            span: Span::new(0, 0),
        }],
    );
    let mir = build(&hir);
    assert_eq!(
        mir.items[1],
        MirItem::ForeignImport {
            local_name: "chain".to_string(),
            module_path: "itertools".to_string(),
            from: Some(from),
        }
    );
}

/// #1291: a foreign import nested in a module-level `if` is not spliced as
/// an item; it lowers to a `MirStmt::ForeignImport` inside the `if` body,
/// where it runs only when the branch does. A nested from-import (#1383)
/// carries its `FromImport` through unchanged.
#[test]
fn a_block_foreign_import_lowers_in_place_and_is_not_spliced() {
    let from = pycc_hir::FromImport {
        name: "hls_to_rgb".to_string(),
        fromlist: vec!["hls_to_rgb".to_string()],
        index: 0,
        level: 0,
    };
    for (local_name, from) in [("colorsys", None), ("hls_to_rgb", Some(from))] {
        let bindings = vec![(local_name.to_string(), "colorsys".to_string(), from.clone())];
        let hir = HirModule {
            items: vec![HirItem::TopLevelStmt(pycc_hir::HirStmt::If {
                test: pycc_hir::HirExpr::BoolLiteral(true),
                body: vec![pycc_hir::HirStmt::ForeignImport {
                    bindings: bindings.clone(),
                    span: Span::new(0, 0),
                }],
                orelse: vec![],
            })],
            ..module_with_imports(vec![ImportBinding::Foreign {
                local_name: local_name.to_string(),
                module_path: "colorsys".to_string(),
                from,
                site: pycc_hir::ForeignImportSite::Block { optional: false },
                span: Span::new(0, 0),
            }])
        };
        let mir = build(&hir);
        assert!(foreign_items(&mir).is_empty());
        assert_eq!(mir.items.len(), 1);
        let MirItem::TopLevelStmt(MirStmt::If { body, .. }) = &mir.items[0] else {
            panic!("the `if` stays the only item: {:?}", mir.items);
        };
        assert!(
            matches!(body.as_slice(), [MirStmt::ForeignImport { bindings: lowered }] if *lowered == bindings),
            "{body:?}"
        );
    }
}

// ---------------------------------------------------------------------
// Part 2 of #1026 (#1081): attribute loads on the bound object.
//
// The two tests below are the only place the `HirExpr::AttrGet` ->
// `MirExpr::ObjAttrGet` lowering is exercised from real HIR. The codegen
// crate's own tests hand-build `MirExpr::ObjAttrGet` (they are about what
// LLVM receives, not about how the node is produced), and the integration
// tests stop at `pycc check`, which never lowers. Lowering is also where
// the module-scope `Ty::Object` bind in `build` is proved: without it,
// `lookup` panics on the `numpy` read rather than producing a base.
// ---------------------------------------------------------------------

/// A module that imports `numpy` at index 0 and then evaluates and
/// discards `base_expr` as its single statement.
fn module_with_discarded(base_expr: pycc_hir::HirExpr) -> HirModule {
    HirModule {
        items: vec![HirItem::TopLevelStmt(pycc_hir::HirStmt::ExprStmt(
            base_expr,
        ))],
        ..module_with_imports(vec![foreign("numpy", 0)])
    }
}

/// The lowered form of the single top-level statement in `hir`.
fn only_discarded_expr(hir: &HirModule) -> MirExpr {
    let mir = build(hir);
    mir.items
        .iter()
        .find_map(|item| match item {
            MirItem::TopLevelStmt(MirStmt::ExprStmt(expr)) => Some(expr.clone()),
            _ => None,
        })
        .expect("the module has exactly one top-level statement")
}

fn attr_get(base: pycc_hir::HirExpr, attr: &str) -> pycc_hir::HirExpr {
    pycc_hir::HirExpr::AttrGet {
        base: Box::new(base),
        attr: attr.to_string(),
    }
}

#[test]
fn an_attribute_load_on_a_foreign_object_lowers_to_obj_attr_get() {
    // `import numpy` / `numpy.pi`. The base resolves to `Ty::Object`
    // through the module-scope bind, so the load takes the string-keyed
    // path instead of `class_def_of`, which has no `HirClassDef` to find.
    let hir = module_with_discarded(attr_get(pycc_hir::HirExpr::Name("numpy".to_string()), "pi"));
    let MirExpr::ObjAttrGet { base, attr, ty } = only_discarded_expr(&hir) else {
        panic!("expected an `ObjAttrGet`");
    };
    assert_eq!(attr, "pi");
    assert_eq!(ty, Ty::Object);
    assert!(matches!(*base, MirExpr::Name { ref name, ty: Ty::Object } if name == "numpy"));
}

#[test]
fn a_chained_attribute_load_stays_opaque_at_every_level() {
    // `numpy.linalg.norm`: the inner load's own result is `Ty::Object`, so
    // the outer load re-enters the same arm rather than falling through to
    // the slot-resolving path. This is what makes the node's `ty` field
    // load-bearing rather than decorative.
    let hir = module_with_discarded(attr_get(
        attr_get(pycc_hir::HirExpr::Name("numpy".to_string()), "linalg"),
        "norm",
    ));
    let MirExpr::ObjAttrGet { base, attr, .. } = only_discarded_expr(&hir) else {
        panic!("expected an outer `ObjAttrGet`");
    };
    assert_eq!(attr, "norm");
    let MirExpr::ObjAttrGet { attr: inner, .. } = *base else {
        panic!("expected an inner `ObjAttrGet`");
    };
    assert_eq!(inner, "linalg");
}

// ---------------------------------------------------------------------
// PR 2b of #1081: method calls on the bound object.
//
// Same rationale as the two attribute-load tests above -- this is the
// only place the `HirExpr::MethodCall` -> `MirExpr::ObjMethodCall`
// lowering runs on real HIR. `pycc_codegen`'s `foreign_call.rs` tests
// hand-build the node, and the integration tests either stop at
// `pycc check` (which never lowers) or need a hosting interpreter.
// ---------------------------------------------------------------------

fn method_call(
    base: pycc_hir::HirExpr,
    method: &str,
    args: Vec<pycc_hir::HirExpr>,
) -> pycc_hir::HirExpr {
    pycc_hir::HirExpr::MethodCall {
        base: Box::new(base),
        method: method.to_string(),
        args,
    }
}

#[test]
fn a_method_call_on_a_foreign_object_lowers_to_obj_method_call() {
    // `import numpy` / `numpy.sqrt(2.0)`. The `Ty::Object` base makes the
    // arm above `class_def_of` fire: a foreign object has no
    // `HirClassDef`, so every later branch in that arm would fail to
    // resolve a mangled symbol.
    let hir = module_with_discarded(method_call(
        pycc_hir::HirExpr::Name("numpy".to_string()),
        "sqrt",
        vec![pycc_hir::HirExpr::FloatLiteral(2.0)],
    ));
    let expr = only_discarded_expr(&hir);
    // The node carries no `ty` field, so `ty()` is the only way to ask --
    // and answering `Ty::Object` unconditionally is the contract that
    // replaces the field.
    assert_eq!(expr.ty(), Ty::Object);
    let MirExpr::ObjMethodCall { base, method, args } = expr else {
        panic!("expected an `ObjMethodCall`");
    };
    assert_eq!(method, "sqrt");
    assert_eq!(args.len(), 1);
    assert!(matches!(args[0], MirExpr::FloatLiteral(f) if f == 2.0));
    assert!(matches!(*base, MirExpr::Name { ref name, ty: Ty::Object } if name == "numpy"));
}

#[test]
fn a_method_call_on_a_foreign_attribute_keeps_the_load_as_its_base() {
    // `numpy.linalg.norm(1)`: the base is itself an `ObjAttrGet`, which is
    // what proves the arm keys on the base's *type* rather than on the
    // base being a bare name. Zero-or-more arguments also means the
    // argument vector is lowered element by element rather than reused.
    let hir = module_with_discarded(method_call(
        attr_get(pycc_hir::HirExpr::Name("numpy".to_string()), "linalg"),
        "norm",
        Vec::new(),
    ));
    let MirExpr::ObjMethodCall { base, method, args } = only_discarded_expr(&hir) else {
        panic!("expected an `ObjMethodCall`");
    };
    assert_eq!(method, "norm");
    assert!(args.is_empty());
    let MirExpr::ObjAttrGet { attr, .. } = *base else {
        panic!("expected the base to stay an `ObjAttrGet`");
    };
    assert_eq!(attr, "linalg");
}

// ---------------------------------------------------------------------
// PR 3a of #1082 (Part 3 of #1026): `len` on the bound object.
//
// Same rationale as the four tests above -- this is the only place the
// `HirExpr::Call { callee: "len" }` -> `MirExpr::ObjLen` split runs on
// real HIR. `pycc_codegen`'s `foreign_len.rs` tests hand-build the node
// because they are about what LLVM receives, and the integration tests
// either stop at `pycc check` (which never lowers) or need a hosting
// interpreter. Without a test here the split itself is unexercised.
// ---------------------------------------------------------------------

fn call(callee: &str, args: Vec<pycc_hir::HirExpr>) -> pycc_hir::HirExpr {
    pycc_hir::HirExpr::Call {
        callee: callee.to_string(),
        args,
    }
}

#[test]
fn len_of_a_foreign_object_lowers_to_obj_len() {
    // `import numpy` / `len(numpy)`. The argument's `Ty::Object` is what
    // diverts this away from the ordinary scalar `len` lowering below it,
    // so the operand type -- not the callee name alone -- is the test's
    // subject.
    let hir = module_with_discarded(call(
        "len",
        vec![pycc_hir::HirExpr::Name("numpy".to_string())],
    ));
    let expr = only_discarded_expr(&hir);
    // The node carries no `ty` field for the same reason `ObjMethodCall`
    // does not: `ty()` answering `Ty::Int` unconditionally is the contract.
    assert_eq!(expr.ty(), Ty::Int);
    let MirExpr::ObjLen { base } = expr else {
        panic!("expected an `ObjLen`");
    };
    assert!(matches!(*base, MirExpr::Name { ref name, ty: Ty::Object } if name == "numpy"));
}

#[test]
fn len_of_a_foreign_attribute_keeps_the_load_as_its_base() {
    // `len(numpy.linalg)`: the operand is an `ObjAttrGet` rather than a
    // bare name, which proves the split keys on the lowered argument's
    // type and not on the argument being a module binding.
    let hir = module_with_discarded(call(
        "len",
        vec![attr_get(
            pycc_hir::HirExpr::Name("numpy".to_string()),
            "linalg",
        )],
    ));
    let MirExpr::ObjLen { base } = only_discarded_expr(&hir) else {
        panic!("expected an `ObjLen`");
    };
    let MirExpr::ObjAttrGet { attr, .. } = *base else {
        panic!("expected the base to stay an `ObjAttrGet`");
    };
    assert_eq!(attr, "linalg");
}

#[test]
fn len_of_a_foreign_object_ignores_a_module_level_len_definition() {
    // The regression PR 3a's original guard caused: a module-level
    // `def len(...)` put `$fn:len` in scope, the guard declined the
    // `ObjLen` split, and `len(<object>)` reached codegen as an ordinary
    // `Call` whose `Scalar::Object` argument tripped `expect_list_pointer`.
    // `pycc_types::check` resolves `len` as the reserved builtin either
    // way, so the lowering must not second-guess it. The divergence from
    // CPython -- that the shadow is ignored rather than refused -- is
    // #1098, not this node's concern.
    let hir = HirModule {
        items: vec![
            HirItem::Function {
                name: "len".to_string(),
                params: vec![("x".to_string(), Ty::Int)],
                return_ty: Ty::Int,
                body: vec![pycc_hir::HirStmt::Return(Some(pycc_hir::HirExpr::Name(
                    "x".to_string(),
                )))],
            },
            HirItem::TopLevelStmt(pycc_hir::HirStmt::ExprStmt(call(
                "len",
                vec![pycc_hir::HirExpr::Name("numpy".to_string())],
            ))),
        ],
        ..module_with_imports(vec![foreign("numpy", 0)])
    };
    let MirExpr::ObjLen { base } = only_discarded_expr(&hir) else {
        panic!("expected an `ObjLen` despite the shadowing `def len`");
    };
    assert!(matches!(*base, MirExpr::Name { ref name, ty: Ty::Object } if name == "numpy"));
}

// ---------------------------------------------------------------------
// PR 3b of #1082 (Part 3 of #1026): a subscript load on the bound object.
//
// Same rationale as the tests above: `expr.rs`'s `HirExpr::Subscript`
// arm dispatches on the lowered base's `ty()`, and this is the only
// place that dispatch runs on real HIR. `pycc_codegen`'s
// `foreign_call.rs` tests hand-build the node because they are about
// what LLVM receives.
// ---------------------------------------------------------------------

fn subscript(base: pycc_hir::HirExpr, index: pycc_hir::HirExpr) -> pycc_hir::HirExpr {
    pycc_hir::HirExpr::Subscript {
        base: Box::new(base),
        index: Box::new(index),
    }
}

#[test]
fn a_subscript_of_a_foreign_object_lowers_to_obj_subscript() {
    // `import numpy` / `numpy[0]`. The base's `Ty::Object` is what
    // diverts this away from the `Ty::Dict`/`Ty::List`/`Ty::Str` arms
    // below it, so the base type -- not the index -- is the subject.
    let hir = module_with_discarded(subscript(
        pycc_hir::HirExpr::Name("numpy".to_string()),
        pycc_hir::HirExpr::IntLiteral(0),
    ));
    let expr = only_discarded_expr(&hir);
    // The node carries no `ty` field for the same reason `ObjMethodCall`
    // does not: `ty()` answering `Ty::Object` unconditionally is the
    // contract, because `PyObject_GetItem` tells us nothing more.
    assert_eq!(expr.ty(), Ty::Object);
    let MirExpr::ObjSubscript { base, index } = expr else {
        panic!("expected an `ObjSubscript`");
    };
    assert!(matches!(*base, MirExpr::Name { ref name, ty: Ty::Object } if name == "numpy"));
    assert!(matches!(*index, MirExpr::IntLiteral(0)));
}

#[test]
fn a_subscript_of_a_foreign_attribute_keeps_the_load_as_its_base() {
    // `numpy.garbage["k"]`: the base is itself an `ObjAttrGet` and the
    // key is a `str` rather than an `int`, which together prove the arm
    // keys on the lowered base's type and admits every packable scalar.
    let hir = module_with_discarded(subscript(
        attr_get(pycc_hir::HirExpr::Name("numpy".to_string()), "garbage"),
        pycc_hir::HirExpr::StringLiteral("k".to_string()),
    ));
    let MirExpr::ObjSubscript { base, index } = only_discarded_expr(&hir) else {
        panic!("expected an `ObjSubscript`");
    };
    let MirExpr::ObjAttrGet { attr, .. } = *base else {
        panic!("expected the base to stay an `ObjAttrGet`");
    };
    assert_eq!(attr, "garbage");
    assert!(matches!(*index, MirExpr::StringLiteral(ref s) if s == "k"));
}

#[test]
fn a_walrus_in_a_foreign_subscript_key_binds_for_the_next_statement() {
    // PEP 572 (#774): `MirExpr::collect_named_expr_bindings` has to
    // recurse into *both* sides of the new node. A walrus can hide in
    // the key as easily as in the base, and `stmt.rs`'s `ExprStmt` arm
    // binds whatever that walk finds -- so without the `index` recursion
    // the following `print(n)` would lower against an unbound `n`.
    let hir = HirModule {
        items: vec![
            HirItem::TopLevelStmt(pycc_hir::HirStmt::ExprStmt(subscript(
                pycc_hir::HirExpr::Name("numpy".to_string()),
                pycc_hir::HirExpr::NamedExpr {
                    name: "n".to_string(),
                    value: Box::new(pycc_hir::HirExpr::IntLiteral(1)),
                },
            ))),
            HirItem::TopLevelStmt(pycc_hir::HirStmt::ExprStmt(pycc_hir::HirExpr::Name(
                "n".to_string(),
            ))),
        ],
        ..module_with_imports(vec![foreign("numpy", 0)])
    };
    let mir = build(&hir);
    let lowered: Vec<&MirExpr> = mir
        .items
        .iter()
        .filter_map(|item| match item {
            MirItem::TopLevelStmt(MirStmt::ExprStmt(expr)) => Some(expr),
            _ => None,
        })
        .collect();
    assert_eq!(lowered.len(), 2, "{lowered:?}");
    assert!(matches!(lowered[0], MirExpr::ObjSubscript { .. }));
    assert!(
        matches!(lowered[1], MirExpr::Name { name, ty: Ty::Int } if name == "n"),
        "{:?}",
        lowered[1]
    );
}

// -- PR 3c of #1082: `for` over an `object` value --------------------------
//
// The statement only ever reaches lowering with a `Ty::Object` iterable --
// `pycc_types` refuses every other one with `I0404` -- so these pin the
// three things the arm itself owns: the iterable is lowered as an ordinary
// expression, the loop variable is bound as `Ty::Object` for the body, and
// the body is lowered as a loop body.

/// A module importing `numpy` at index 0 whose single top-level statement
/// is `for x in numpy.pi:` with `body`.
fn for_object_module(body: Vec<pycc_hir::HirStmt>) -> HirModule {
    HirModule {
        items: vec![HirItem::TopLevelStmt(pycc_hir::HirStmt::ForObject {
            var: "x".to_string(),
            iter: Box::new(attr_get(pycc_hir::HirExpr::Name("numpy".to_string()), "pi")),
            body,
        })],
        ..module_with_imports(vec![foreign("numpy", 0)])
    }
}

/// The lowered form of `hir`'s single top-level `ForObject`.
fn only_for_object(hir: &HirModule) -> (MirExpr, Vec<MirStmt>) {
    let mir = build(hir);
    mir.items
        .iter()
        .find_map(|item| match item {
            MirItem::TopLevelStmt(MirStmt::ForObject { var, iter, body }) => {
                assert_eq!(var, "x");
                Some((iter.clone(), body.clone()))
            }
            _ => None,
        })
        .expect("the module has exactly one `for` loop")
}

#[test]
fn a_for_loop_over_a_foreign_object_lowers_its_iterable_as_an_expression() {
    // Unlike `ForList`'s bare list name, the iterable is a real
    // expression and goes through `lower_expr` -- here reaching the same
    // `ObjAttrGet` arm an `ExprStmt` would.
    let (iter, body) = only_for_object(&for_object_module(Vec::new()));
    let MirExpr::ObjAttrGet { base, attr, ty } = iter else {
        panic!("expected the iterable to lower to an `ObjAttrGet`");
    };
    assert_eq!(attr, "pi");
    assert_eq!(ty, Ty::Object);
    assert!(matches!(*base, MirExpr::Name { ref name, ty: Ty::Object } if name == "numpy"));
    assert!(body.is_empty(), "{body:?}");
}

#[test]
fn a_for_loop_over_a_foreign_object_binds_its_variable_as_an_object() {
    // The bind is what the body is lowered against: without it, `lookup`
    // panics on the `x` read rather than resolving it, and the item
    // codegen stores into the loop slot would have no type.
    let (_, body) = only_for_object(&for_object_module(vec![pycc_hir::HirStmt::ExprStmt(
        pycc_hir::HirExpr::Name("x".to_string()),
    )]));
    let [MirStmt::ExprStmt(MirExpr::Name { name, ty })] = body.as_slice() else {
        panic!("expected the body to lower to a single name read: {body:?}");
    };
    assert_eq!(name, "x");
    assert_eq!(*ty, Ty::Object);
}

#[test]
fn a_for_loop_over_a_foreign_object_overwrites_an_earlier_binding_of_its_variable() {
    // The arm calls `bind`, not `bind_variable`: `ForObject` is the first
    // loop construct whose target type can differ from a name's earlier
    // binding (`ForList`/`ForRange` always bind `Ty::Int`), so
    // `bind_variable`'s `or_insert` would keep the stale `Ty::Int` here and
    // lower the body against the wrong type. `pycc_types` rejects this
    // source shape outright (`T0023`), so this is defence in depth against a
    // future caller that hands MIR the shape directly -- and it is the only
    // discriminating test for the choice, since the checker never lets the
    // CLI reach it.
    let mut hir = for_object_module(vec![pycc_hir::HirStmt::ExprStmt(pycc_hir::HirExpr::Name(
        "x".to_string(),
    ))]);
    hir.items.insert(
        0,
        HirItem::TopLevelStmt(pycc_hir::HirStmt::Assign {
            target: "x".to_string(),
            value: pycc_hir::HirExpr::IntLiteral(5),
        }),
    );
    let (_, body) = only_for_object(&hir);
    let [MirStmt::ExprStmt(MirExpr::Name { name, ty })] = body.as_slice() else {
        panic!("expected the body to lower to a single name read: {body:?}");
    };
    assert_eq!(name, "x");
    assert_eq!(*ty, Ty::Object, "the loop must overwrite the earlier `int`");
}

// ---------------------------------------------------------------------
// PR 4a of #1083 (Part 4 of #1026): `float(o)` and `bool(o)`.
//
// Neither gets a `MirExpr` variant -- both stay ordinary `Call` nodes and
// only the *type* the lowering assigns them is at stake. Without a `bool`
// branch, `bool(o)` falls to `lookup`'s `$fn:bool` miss and panics: a
// `pycc check` that exits 0 followed by a `pycc build --ext` that aborts.
// ---------------------------------------------------------------------

#[test]
fn float_of_a_foreign_object_stays_a_call_typed_float() {
    let hir = module_with_discarded(call(
        "float",
        vec![pycc_hir::HirExpr::Name("numpy".to_string())],
    ));
    let MirExpr::Call { callee, args, ty } = only_discarded_expr(&hir) else {
        panic!("expected a plain `Call`");
    };
    assert_eq!(callee, "float");
    assert_eq!(ty, Ty::Float);
    assert!(matches!(
        args.as_slice(),
        [MirExpr::Name { ty: Ty::Object, .. }]
    ));
}

#[test]
fn bool_of_a_foreign_object_stays_a_call_typed_bool() {
    let hir = module_with_discarded(call(
        "bool",
        vec![pycc_hir::HirExpr::Name("numpy".to_string())],
    ));
    let MirExpr::Call { callee, args, ty } = only_discarded_expr(&hir) else {
        panic!("expected a plain `Call`");
    };
    assert_eq!(callee, "bool");
    assert_eq!(ty, Ty::Bool);
    assert!(matches!(
        args.as_slice(),
        [MirExpr::Name { ty: Ty::Object, .. }]
    ));
}

#[test]
fn a_user_defined_bool_function_is_lowered_as_a_real_call_not_the_builtin() {
    // The C4 guard, mirrored from `float`'s: `pycc_types` honours a
    // `def bool(...)` and refuses the builtin arm outright, so a lowering
    // that ignored the shadow would type the call `Ty::Bool` against the
    // user function's own registered `Ty::Int` return. This is the opposite
    // of `len`'s case (`len_of_a_foreign_object_ignores_a_module_level_len_
    // definition` above), where the checker resolves the builtin regardless
    // and the lowering must follow it.
    let hir = HirModule {
        items: vec![
            HirItem::Function {
                name: "bool".to_string(),
                params: vec![("x".to_string(), Ty::Int)],
                return_ty: Ty::Int,
                body: vec![pycc_hir::HirStmt::Return(Some(pycc_hir::HirExpr::Name(
                    "x".to_string(),
                )))],
            },
            HirItem::TopLevelStmt(pycc_hir::HirStmt::ExprStmt(call(
                "bool",
                vec![pycc_hir::HirExpr::IntLiteral(1)],
            ))),
        ],
        ..module_with_imports(vec![foreign("numpy", 0)])
    };
    let mir = build(&hir);
    let MirItem::TopLevelStmt(MirStmt::ExprStmt(MirExpr::Call { ty, .. })) = &mir.items[2] else {
        panic!(
            "expected the shadowing call to stay a `Call`: {:?}",
            mir.items
        );
    };
    assert_eq!(*ty, Ty::Int, "the user function's return type must win");
}

#[test]
fn int_and_str_of_a_foreign_object_stay_calls_typed_int_and_str() {
    // Part 4 of #1026 (PR 4b of #1083), `bool_of_a_foreign_object_stays_a_
    // call_typed_bool`'s claim for the other two conversions: without the
    // lowering arm the call falls to `lookup`, finds no `$fn:int`, and
    // panics on a program `pycc check` has already accepted.
    for (callee, expected) in [("int", Ty::Int), ("str", Ty::Str)] {
        let hir = module_with_discarded(call(
            callee,
            vec![pycc_hir::HirExpr::Name("numpy".to_string())],
        ));
        let MirExpr::Call {
            callee: c,
            args,
            ty,
        } = only_discarded_expr(&hir)
        else {
            panic!("{callee}: expected a plain `Call`");
        };
        assert_eq!(c, callee);
        assert_eq!(ty, expected);
        assert!(matches!(
            args.as_slice(),
            [MirExpr::Name { ty: Ty::Object, .. }]
        ));
    }
}

#[test]
fn a_user_defined_int_or_str_function_is_lowered_as_a_real_call_not_the_builtin() {
    // `a_user_defined_bool_function_is_lowered_as_a_real_call_not_the_
    // builtin`'s claim for PR 4b's two names: `pycc_types` honours a
    // `def int(...)`/`def str(...)` and refuses the builtin arm outright, so
    // a lowering that ignored the shadow would type the call `Ty::Int`/
    // `Ty::Str` against the user function's own registered `Ty::Float`
    // return.
    for callee in ["int", "str"] {
        let hir = HirModule {
            items: vec![
                HirItem::Function {
                    name: callee.to_string(),
                    params: vec![("x".to_string(), Ty::Float)],
                    return_ty: Ty::Float,
                    body: vec![pycc_hir::HirStmt::Return(Some(pycc_hir::HirExpr::Name(
                        "x".to_string(),
                    )))],
                },
                HirItem::TopLevelStmt(pycc_hir::HirStmt::ExprStmt(call(
                    callee,
                    vec![pycc_hir::HirExpr::FloatLiteral(1.0)],
                ))),
            ],
            ..module_with_imports(vec![foreign("numpy", 0)])
        };
        let mir = build(&hir);
        let MirItem::TopLevelStmt(MirStmt::ExprStmt(MirExpr::Call { ty, .. })) = &mir.items[2]
        else {
            panic!(
                "{callee}: expected the shadowing call to stay a `Call`: {:?}",
                mir.items
            );
        };
        assert_eq!(
            *ty,
            Ty::Float,
            "{callee}: the user function's return type must win"
        );
    }
}

// ---------------------------------------------------------------------
// PR 4c of #1083 (Part 4 of #1026): a module-level annotated assignment
// of the bound object to a fixed-arity all-`float` tuple.
//
// Same rationale as the sections above -- this is the only place the
// `HirStmt::AnnAssign` -> `MirExpr::ObjUnpackFloatTuple` split runs on
// real HIR. `pycc_codegen`'s `foreign_len.rs` tests hand-build the node
// because they are about what LLVM receives, and the integration tests
// either stop at `pycc check` (which never lowers) or need a hosting
// interpreter.
//
// 4c adds **no HIR variant**: the HIR is an ordinary `AnnAssign` over an
// ordinary value expression. That is why `monomorphize.rs`'s
// `rewrite_protocol_calls_in_stmt`, `pycc_types`' `bind_local_types_in_stmt`
// and the empty-container pre-pass need no new arm -- each already has an
// `AnnAssign` arm, and the #1104/#1105 missing-arm defect class reproduces
// only for a *new* statement or expression shape.
// ---------------------------------------------------------------------

/// A module importing `numpy` at index 0 whose single statement is
/// `<target>: <annotation> = <value>`.
fn module_with_ann_assign(target: &str, annotation: Ty, value: pycc_hir::HirExpr) -> HirModule {
    HirModule {
        items: vec![HirItem::TopLevelStmt(pycc_hir::HirStmt::AnnAssign {
            target: target.to_string(),
            annotation,
            value: Some(value),
            is_final: false,
        })],
        ..module_with_imports(vec![foreign("numpy", 0)])
    }
}

/// The value of the single lowered top-level `Assign` in `hir`.
fn only_assigned_value(hir: &HirModule) -> MirExpr {
    let mir = build(hir);
    mir.items
        .iter()
        .find_map(|item| match item {
            MirItem::TopLevelStmt(MirStmt::Assign { value, .. }) => Some(value.clone()),
            _ => None,
        })
        .expect("the module has exactly one top-level assignment")
}

fn float_tuple(arity: usize) -> Ty {
    Ty::Tuple(Box::new(vec![Ty::Float; arity]))
}

#[test]
fn an_annotated_assignment_of_a_foreign_object_lowers_to_an_unpack() {
    // `import numpy` / `x: tuple[float, float, float] = numpy`. Two
    // arities, because the node carries the arity rather than rediscovering
    // it and nothing may hard-code the three.
    for arity in [1usize, 3] {
        let hir = module_with_ann_assign(
            "x",
            float_tuple(arity),
            pycc_hir::HirExpr::Name("numpy".to_string()),
        );
        let value = only_assigned_value(&hir);
        // The node carries no `ty` field: `ty()` rebuilds the annotation
        // from the arity, which is the contract the variant documents.
        assert_eq!(value.ty(), float_tuple(arity), "{arity}");
        let MirExpr::ObjUnpackFloatTuple { base, arity: got } = value else {
            panic!("{arity}: expected an `ObjUnpackFloatTuple`");
        };
        assert_eq!(got, arity);
        assert!(matches!(*base, MirExpr::Name { ref name, ty: Ty::Object } if name == "numpy"));
    }
}

#[test]
fn an_annotated_assignment_of_a_foreign_attribute_keeps_the_load_as_its_base() {
    // `x: tuple[float, float] = numpy.shape`: the initializer is an
    // `ObjAttrGet` rather than a bare name, which proves the split keys on
    // the lowered initializer's *type* and not on it being a module
    // binding -- `foreign.rs`'s "every refusal must key on the type, never
    // on the producing expression shape" rule, in its admitting direction.
    let hir = module_with_ann_assign(
        "x",
        float_tuple(2),
        attr_get(pycc_hir::HirExpr::Name("numpy".to_string()), "shape"),
    );
    let MirExpr::ObjUnpackFloatTuple { base, .. } = only_assigned_value(&hir) else {
        panic!("expected an `ObjUnpackFloatTuple`");
    };
    let MirExpr::ObjAttrGet { attr, .. } = *base else {
        panic!("expected the base to stay an `ObjAttrGet`");
    };
    assert_eq!(attr, "shape");
}

#[test]
fn an_annotated_assignment_that_is_not_the_admitted_shape_is_left_alone() {
    // The negative half of `float_tuple_annotation_arity`, driven through the
    // lowering rather than through the predicate directly so the guard's
    // placement in the widening chain is what is under test.
    //
    // Three shapes, one per clause. A non-object initializer under a float
    // tuple annotation (the `matches!(value.ty(), Ty::Object)` half), an
    // object under a *mixed* tuple (`elems.iter().all(...)`, which
    // `pycc_types` refuses with `T0025` before lowering ever runs), and an
    // object under a non-tuple annotation. None may become an unpack; the
    // last is not even reachable from a real program, since `pycc_types`
    // refuses it too -- lowering must simply not invent a node for it.
    let numpy = || pycc_hir::HirExpr::Name("numpy".to_string());
    for (label, annotation, value) in [
        (
            "a native tuple initializer",
            float_tuple(2),
            pycc_hir::HirExpr::TupleLiteral(vec![
                pycc_hir::HirExpr::FloatLiteral(1.0),
                pycc_hir::HirExpr::FloatLiteral(2.0),
            ]),
        ),
        (
            "a mixed tuple annotation",
            Ty::Tuple(Box::new(vec![Ty::Float, Ty::Int])),
            numpy(),
        ),
        ("a non-tuple annotation", Ty::Float, numpy()),
    ] {
        let hir = module_with_ann_assign("x", annotation, value);
        assert!(
            !matches!(
                only_assigned_value(&hir),
                MirExpr::ObjUnpackFloatTuple { .. }
            ),
            "{label}: must not lower to an unpack"
        );
    }
}

#[test]
fn an_unpacks_walrus_binding_is_collected_from_its_base() {
    // `MirExpr::collect_named_expr_bindings` is where the #1104/#1105
    // defect class actually lands for a new expression node: a walrus the
    // walk misses is a name codegen never allocates storage for. The node
    // has exactly one child, so the base is the only place one can hide.
    let mut found = Vec::new();
    MirExpr::ObjUnpackFloatTuple {
        base: Box::new(MirExpr::NamedExpr {
            name: "o".to_string(),
            value: Box::new(MirExpr::Name {
                name: "numpy".to_string(),
                ty: Ty::Object,
            }),
            ty: Ty::Object,
        }),
        arity: 3,
    }
    .collect_named_expr_bindings(&mut found);
    assert_eq!(found, vec![("o".to_string(), Ty::Object)]);
}

// Part 11 of #1371: `type(o)` on the bound object.
mod object_type;
