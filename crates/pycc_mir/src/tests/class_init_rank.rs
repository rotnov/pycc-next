//! #966: constructor resolution ranks a D-225 implicit `__init__` last.
//!
//! `pycc_hir` flags a class whose `__init__` it synthesized only because
//! nothing in that class's MRO declared one
//! ([`HirClassDef::implicit_object_init`]). Both constructor seams in this
//! crate -- `Instantiate` lowering and the `super()` arm of `MethodCall` --
//! skip a flagged class on a first pass and fall back to one only when the
//! whole MRO is implicit.
//!
//! These are hand-built HIR modules rather than compiled programs because
//! integration tests under `tests/` do not score this crate's own regions
//! (D-014, `docs/TESTING.md`). The end-to-end behaviour they stand for is
//! asserted in `tests/issue_966_inherited_init_rank.rs`.

use crate::*;
use pycc_hir::{HirClassDef, HirExpr, HirItem, HirModule, HirStmt, Ty};

/// A minimal class definition: everything not named here is empty/false.
fn cdef(
    name: &str,
    bases: &[&str],
    mro: &[&str],
    methods: &[&str],
    attrs: &[(&str, Ty)],
    implicit_object_init: bool,
) -> (String, HirClassDef) {
    (
        name.to_string(),
        HirClassDef {
            name: name.to_string(),
            bases: bases.iter().map(|b| b.to_string()).collect(),
            mro: mro.iter().map(|m| m.to_string()).collect(),
            attrs: attrs
                .iter()
                .map(|(a, ty)| (a.to_string(), ty.clone()))
                .collect(),
            methods: methods
                .iter()
                .map(|m| (m.to_string(), format!("{name}.{m}")))
                .collect(),
            properties: Vec::new(),
            static_methods: Vec::new(),
            class_methods: Vec::new(),
            type_param: None,
            is_enum: false,
            implicit_object_init,
            method_defaults: Vec::new(),
            enum_members: Vec::new(),
            class_attrs: Vec::new(),
            is_dataclass: false,
            dataclass_fields: Vec::new(),
            is_protocol: false,
            runtime_checkable: false,
            protocol_members: Vec::new(),
            abstract_methods: Vec::new(),
            is_abstract: false,
            exception_type_tag: None,
        },
    )
}

/// A trivial `HirItem::Function` for `<class>.<method>`. The `super()` arm
/// reads the resolved callee's recorded `$fn:` type, so every candidate the
/// walk could land on must exist as a real item.
fn fn_item(class: &str, method: &str) -> HirItem {
    HirItem::Function {
        name: format!("{class}.{method}"),
        params: vec![(
            "self".to_string(),
            Ty::Instance(Box::new(class.to_string())),
        )],
        return_ty: Ty::None,
        body: vec![HirStmt::Return(None)],
    }
}

/// The mangled constructor name the lowered `c = C()` resolved to.
fn lowered_ctor(class_defs: Vec<(String, HirClassDef)>) -> String {
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![HirItem::TopLevelStmt(HirStmt::Assign {
            target: "c".to_string(),
            value: HirExpr::Call {
                callee: "C".to_string(),
                args: vec![],
            },
        })],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs,
    };
    let mir = build(&hir);
    for item in &mir.items {
        if let MirItem::TopLevelStmt(MirStmt::Assign { value, .. }) = item
            && let MirExpr::Instantiate(inst) = value
        {
            return inst.ctor.clone();
        }
    }
    panic!("the lowered module should contain an `Instantiate`");
}

/// `class A: pass` / `class B: __init__` / `class C(A, B)`: `A`'s implicit
/// constructor must not out-rank `B`'s real one. This is #966 itself.
#[test]
fn instantiate_skips_an_implicit_constructor_for_a_later_real_one() {
    let ctor = lowered_ctor(vec![
        cdef("C", &["A", "B"], &["C", "A", "B"], &[], &[], false),
        cdef("A", &[], &["A"], &["__init__"], &[], true),
        cdef("B", &[], &["B"], &["__init__"], &[("z", Ty::Int)], false),
    ]);
    assert_eq!(
        ctor, "B.__init__",
        "the implicit constructor on `A` must rank last, so `B`'s real one wins"
    );
}

/// The fallback pass: when *every* candidate is implicit there is nothing
/// else to call, and the implicit constructor is the correct answer rather
/// than an internal error.
#[test]
fn instantiate_falls_back_to_an_implicit_constructor_when_all_are_implicit() {
    let ctor = lowered_ctor(vec![
        cdef("C", &["A", "B"], &["C", "A", "B"], &[], &[], false),
        cdef("A", &[], &["A"], &["__init__"], &[], true),
        cdef("B", &[], &["B"], &["__init__"], &[], true),
    ]);
    assert_eq!(
        ctor, "A.__init__",
        "with no real constructor anywhere in the MRO, the first implicit \
         one is the one to call"
    );
}

/// The same ranking on the `super()` seam. `C` declares its own `__init__`
/// whose body is `super().__init__()`; the call must reach `B`, not `A`.
#[test]
fn super_init_skips_an_implicit_constructor_for_a_later_real_one() {
    let mangled = lowered_super_init_callee(false);
    assert_eq!(
        mangled, "B.__init__",
        "`super().__init__()` ranks constructors exactly as instantiation does"
    );
}

/// The mandatory fallback pass on the `super()` seam: `class A: pass` /
/// `class C(A)` calling `super().__init__()` has only the implicit
/// constructor above it, and must resolve to it rather than panicking.
#[test]
fn super_init_falls_back_to_an_implicit_constructor_when_it_is_the_only_one() {
    let mangled = lowered_super_init_callee(true);
    assert_eq!(
        mangled, "A.__init__",
        "the implicit constructor is the only candidate and must be reached"
    );
}

/// Lowers `class C(...): def __init__(self): super().__init__()` and returns
/// the mangled callee the `super()` arm resolved to. With `only_implicit`,
/// `C`'s sole base is the implicit-constructor class `A`; otherwise a real
/// `B` follows `A` in the MRO.
fn lowered_super_init_callee(only_implicit: bool) -> String {
    let self_ty = Ty::Instance(Box::new("C".to_string()));
    let mut class_defs = vec![
        cdef("A", &[], &["A"], &["__init__"], &[], true),
        if only_implicit {
            cdef("C", &["A"], &["C", "A"], &["__init__"], &[], false)
        } else {
            cdef(
                "C",
                &["A", "B"],
                &["C", "A", "B"],
                &["__init__"],
                &[],
                false,
            )
        },
    ];
    if !only_implicit {
        class_defs.push(cdef(
            "B",
            &[],
            &["B"],
            &["__init__"],
            &[("z", Ty::Int)],
            false,
        ));
    }
    let mut items = vec![
        HirItem::Function {
            name: "C.__init__".to_string(),
            params: vec![("self".to_string(), self_ty)],
            return_ty: Ty::None,
            body: vec![HirStmt::ExprStmt(HirExpr::MethodCall {
                base: Box::new(HirExpr::Super),
                method: "__init__".to_string(),
                args: vec![],
            })],
        },
        fn_item("A", "__init__"),
    ];
    if !only_implicit {
        items.push(fn_item("B", "__init__"));
    }
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items,
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs,
    };
    let mir = build(&hir);
    for item in &mir.items {
        if let MirItem::Function { body, .. } = item {
            for stmt in body {
                if let MirStmt::ExprStmt(MirExpr::Call { callee, .. }) = stmt {
                    return callee.clone();
                }
            }
        }
    }
    panic!("the lowered `C.__init__` should contain a `super()` call");
}

/// The `__init__` skip is gated on the method name: every other
/// `super().m()` resolution stays name-agnostic, so a flagged class is
/// still a perfectly good owner of an ordinary method.
#[test]
fn super_leaves_a_non_constructor_method_on_a_flagged_class_alone() {
    let self_ty = Ty::Instance(Box::new("C".to_string()));
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            HirItem::Function {
                name: "C.ping".to_string(),
                params: vec![("self".to_string(), self_ty)],
                return_ty: Ty::None,
                body: vec![HirStmt::ExprStmt(HirExpr::MethodCall {
                    base: Box::new(HirExpr::Super),
                    method: "ping".to_string(),
                    args: vec![],
                })],
            },
            fn_item("A", "ping"),
        ],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: vec![
            cdef("C", &["A"], &["C", "A"], &["ping"], &[], false),
            // `A` carries the implicit constructor *and* a real `ping`.
            cdef("A", &[], &["A"], &["__init__", "ping"], &[], true),
        ],
    };
    let mir = build(&hir);
    let mut callee = None;
    for item in &mir.items {
        if let MirItem::Function { body, .. } = item {
            for stmt in body {
                if let MirStmt::ExprStmt(MirExpr::Call { callee: c, .. }) = stmt {
                    callee = Some(c.clone());
                }
            }
        }
    }
    assert_eq!(
        callee.as_deref(),
        Some("A.ping"),
        "the flagged class still owns `ping` -- only `__init__` is re-ranked"
    );
}
