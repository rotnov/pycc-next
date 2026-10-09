//! Part 1 of #1284: a `staticmethod(<foreign callable>)` class attribute
//! lowers, at every read and call through the class name or a plain-name
//! receiver, to exactly the MIR a hand-written use of the recorded foreign
//! reference produces. A non-name receiver (#1346) arrives at `pycc_codegen`
//! as `MirExpr::Sequence`, the receiver first and that same MIR as its
//! value; no other new node reaches codegen.

use crate::*;
use pycc_diag::Span;
use pycc_hir::{
    ClassAttrValue, ForeignCallableRef, HirClassDef, HirExpr, HirItem, HirModule, HirStmt,
    ImportBinding, Ty,
};

fn foreign(local_name: &str, item_index: usize) -> ImportBinding {
    ImportBinding::Foreign {
        local_name: local_name.to_string(),
        module_path: local_name.to_string(),
        from: None,
        site: pycc_hir::ForeignImportSite::Item(item_index),
        span: Span::new(0, 0),
    }
}

fn class(name: &str, mro: &[&str], class_attrs: Vec<(String, Ty, ClassAttrValue)>) -> HirClassDef {
    HirClassDef {
        class_attrs,
        exception_type_tag: None,
        name: name.to_string(),
        bases: Vec::new(),
        mro: mro.iter().map(|m| (*m).to_string()).collect(),
        attrs: Vec::new(),
        methods: Vec::new(),
        type_param: None,
        properties: Vec::new(),
        static_methods: Vec::new(),
        class_methods: Vec::new(),
        is_enum: false,
        implicit_object_init: false,
        method_defaults: Vec::new(),
        enum_members: Vec::new(),
        is_dataclass: false,
        dataclass_fields: Vec::new(),
        is_protocol: false,
        runtime_checkable: false,
        protocol_members: Vec::new(),
        abstract_methods: Vec::new(),
        is_abstract: false,
    }
}

fn foreign_static(attr: &str, root: &str, path: &[&str]) -> (String, Ty, ClassAttrValue) {
    (
        attr.to_string(),
        Ty::Object,
        ClassAttrValue::ForeignStatic(ForeignCallableRef {
            root: root.to_string(),
            path: path.iter().map(|p| (*p).to_string()).collect(),
        }),
    )
}

fn name(n: &str) -> HirExpr {
    HirExpr::Name(n.to_string())
}

fn attr(base: HirExpr, attr: &str) -> HirExpr {
    HirExpr::AttrGet {
        base: Box::new(base),
        attr: attr.to_string(),
    }
}

fn method_call(base: HirExpr, method: &str, args: Vec<HirExpr>) -> HirExpr {
    HirExpr::MethodCall {
        base: Box::new(base),
        method: method.to_string(),
        args,
    }
}

/// `import os` / `from operator import add`, then `class FS` whose
/// `exists = staticmethod(os.path.exists)` and `plus = staticmethod(add)`,
/// then one method `FS.m(self)` whose body is `body`, and finally `top` as
/// module-body expression statements.
fn module(body: Vec<HirExpr>, top: Vec<HirExpr>) -> HirModule {
    let mut items = vec![HirItem::Function {
        name: "FS.m".to_string(),
        params: vec![("self".to_string(), Ty::Instance(Box::new("FS".to_string())))],
        return_ty: Ty::None,
        body: body.into_iter().map(HirStmt::ExprStmt).collect(),
    }];
    items.extend(
        top.into_iter()
            .map(|e| HirItem::TopLevelStmt(HirStmt::ExprStmt(e))),
    );
    HirModule {
        seeded_builtin_exception_classes: false,
        items,
        type_aliases: Vec::new(),
        imports: vec![foreign("os", 0), foreign("add", 0)],
        class_defs: vec![(
            "FS".to_string(),
            class(
                "FS",
                &["FS"],
                vec![
                    foreign_static("exists", "os", &["path", "exists"]),
                    foreign_static("plus", "add", &[]),
                ],
            ),
        )],
    }
}

fn top_exprs(mir: &MirModule) -> Vec<MirExpr> {
    mir.items
        .iter()
        .filter_map(|item| match item {
            MirItem::TopLevelStmt(MirStmt::ExprStmt(expr)) => Some(expr.clone()),
            _ => None,
        })
        .collect()
}

fn method_body(mir: &MirModule) -> Vec<MirStmt> {
    mir.items
        .iter()
        .find_map(|item| match item {
            MirItem::Function { name, body, .. } if name == "FS.m" => Some(body.clone()),
            _ => None,
        })
        .expect("the method is lowered")
}

fn p() -> HirExpr {
    HirExpr::StringLiteral("/".to_string())
}

#[test]
fn a_class_name_call_lowers_like_the_hand_written_foreign_call() {
    let written = method_call(attr(name("os"), "path"), "exists", vec![p()]);
    let via_class = method_call(name("FS"), "exists", vec![p()]);
    let bare_written = HirExpr::Call {
        callee: "add".to_string(),
        args: vec![HirExpr::IntLiteral(2), HirExpr::IntLiteral(3)],
    };
    let bare_via_class = method_call(
        name("FS"),
        "plus",
        vec![HirExpr::IntLiteral(2), HirExpr::IntLiteral(3)],
    );
    let mir = build(&module(
        Vec::new(),
        vec![written, via_class, bare_written, bare_via_class],
    ));
    let exprs = top_exprs(&mir);
    assert_eq!(exprs.len(), 4, "{exprs:?}");
    assert_eq!(exprs[0], exprs[1]);
    assert!(
        matches!(exprs[1], MirExpr::ObjMethodCall { .. }),
        "{exprs:?}"
    );
    assert_eq!(exprs[2], exprs[3]);
    assert!(matches!(exprs[3], MirExpr::ObjCall { .. }), "{exprs:?}");
}

#[test]
fn a_class_name_read_lowers_like_the_hand_written_foreign_read() {
    let mir = build(&module(
        Vec::new(),
        vec![
            attr(attr(name("os"), "path"), "exists"),
            attr(name("FS"), "exists"),
            name("add"),
            attr(name("FS"), "plus"),
        ],
    ));
    let exprs = top_exprs(&mir);
    assert_eq!(exprs[0], exprs[1]);
    assert!(matches!(exprs[1], MirExpr::ObjAttrGet { .. }), "{exprs:?}");
    assert_eq!(exprs[2], exprs[3]);
}

#[test]
fn an_instance_call_and_read_lower_like_the_hand_written_foreign_use() {
    let mir = build(&module(
        vec![
            method_call(attr(name("os"), "path"), "exists", vec![p()]),
            method_call(name("self"), "exists", vec![p()]),
            attr(attr(name("os"), "path"), "exists"),
            attr(name("self"), "exists"),
        ],
        Vec::new(),
    ));
    let body = method_body(&mir);
    let exprs: Vec<&MirExpr> = body
        .iter()
        .filter_map(|stmt| match stmt {
            MirStmt::ExprStmt(expr) => Some(expr),
            _ => None,
        })
        .collect();
    assert_eq!(exprs.len(), 4, "{body:?}");
    assert_eq!(exprs[0], exprs[1]);
    assert_eq!(exprs[2], exprs[3]);
}

/// #1346: through a non-name receiver, the read and the call lower to a
/// `Sequence` whose `discard` is the receiver's own lowering and whose
/// `value` is exactly the hand-written foreign use -- while the plain-name
/// `self` receiver above keeps the Part 1 MIR with no `Sequence`.
#[test]
fn a_non_name_receiver_is_sequenced_before_the_foreign_use() {
    let receiver = || HirExpr::Call {
        callee: "FS".to_string(),
        args: Vec::new(),
    };
    let mut hir = module(
        Vec::new(),
        vec![
            receiver(),
            method_call(receiver(), "exists", vec![p()]),
            method_call(attr(name("os"), "path"), "exists", vec![p()]),
            attr(receiver(), "exists"),
            attr(attr(name("os"), "path"), "exists"),
        ],
    );
    // `FS()` needs an `__init__`.
    hir.class_defs[0].1.methods = vec![("__init__".to_string(), "FS.__init__".to_string())];
    hir.items.insert(
        0,
        HirItem::Function {
            name: "FS.__init__".to_string(),
            params: vec![("self".to_string(), Ty::Instance(Box::new("FS".to_string())))],
            return_ty: Ty::None,
            body: vec![HirStmt::Return(None)],
        },
    );
    let exprs = top_exprs(&build(&hir));
    assert_eq!(exprs.len(), 5, "{exprs:?}");
    let sequenced = |value: &MirExpr| MirExpr::Sequence {
        discard: Box::new(exprs[0].clone()),
        value: Box::new(value.clone()),
    };
    assert_eq!(exprs[1], sequenced(&exprs[2]));
    assert_eq!(exprs[3], sequenced(&exprs[4]));
    assert_eq!(exprs[1].ty(), exprs[2].ty());
}

/// `pycc_types` refuses `super().exists` when the winner is a
/// `staticmethod(...)` class attribute, so `fold_class_attr` returning
/// `None` for it leaves the `super()` walk on its internal-error panic.
#[test]
#[should_panic(expected = "pycc_mir: internal error: `super().exists` is not a property or class")]
fn a_super_read_of_a_foreign_static_attribute_reaches_the_internal_error() {
    let mut hir = module(Vec::new(), Vec::new());
    hir.class_defs
        .push(("D".to_string(), class("D", &["D", "FS"], Vec::new())));
    hir.items.push(HirItem::Function {
        name: "D.m".to_string(),
        params: vec![("self".to_string(), Ty::Instance(Box::new("D".to_string())))],
        return_ty: Ty::None,
        body: vec![HirStmt::ExprStmt(attr(HirExpr::Super, "exists"))],
    });
    let _ = build(&hir);
}
