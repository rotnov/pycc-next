//! Unit tests for chained-comparison MIR lowering (#1212): the per-link
//! kind, including the instance comparison dunder lookup's refusals.

use super::*;

fn class(name: &str, methods: &[&str]) -> HirClassDef {
    HirClassDef {
        class_attrs: Vec::new(),
        exception_type_tag: None,
        name: name.to_string(),
        bases: Vec::new(),
        mro: vec![name.to_string()],
        attrs: Vec::new(),
        methods: methods
            .iter()
            .map(|m| (m.to_string(), format!("{name}.{m}")))
            .collect(),
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

fn classes() -> HashMap<String, HirClassDef> {
    [
        ("D", &["__eq__", "__lt__"][..]),
        ("E", &["__eq__"][..]),
        ("C", &[][..]),
        ("N", &["__eq__", "__ne__"][..]),
    ]
    .into_iter()
    .map(|(name, methods)| (name.to_string(), class(name, methods)))
    .collect()
}

fn instance(name: &str) -> Ty {
    Ty::Instance(Box::new(name.to_string()))
}

fn name(n: &str, ty: Ty) -> MirExpr {
    MirExpr::Name {
        name: n.to_string(),
        ty,
    }
}

fn callee(op: CmpOpKind, left: &str, right: &str) -> Option<(String, bool)> {
    instance_compare_callee(op, &instance(left), &instance(right), &classes(), &[])
}

#[test]
fn the_dunder_is_found_only_for_two_instances_of_one_class_defining_it() {
    assert_eq!(
        callee(CmpOpKind::Eq, "D", "D"),
        Some(("D.__eq__".to_string(), false))
    );
    assert_eq!(
        callee(CmpOpKind::NotEq, "D", "D"),
        Some(("D.__eq__".to_string(), true))
    );
    assert_eq!(
        callee(CmpOpKind::NotEq, "N", "N"),
        Some(("N.__ne__".to_string(), false))
    );
    assert_eq!(
        callee(CmpOpKind::Lt, "D", "D"),
        Some(("D.__lt__".to_string(), false))
    );
    // Not two instances.
    assert_eq!(
        instance_compare_callee(CmpOpKind::Eq, &Ty::Int, &instance("D"), &classes(), &[]),
        None
    );
    // Two different classes, an unregistered class, an undefined dunder,
    // an operator with no dunder.
    assert_eq!(callee(CmpOpKind::Eq, "D", "E"), None);
    assert_eq!(callee(CmpOpKind::Eq, "X", "X"), None);
    assert_eq!(callee(CmpOpKind::Eq, "C", "C"), None);
    assert_eq!(callee(CmpOpKind::Gt, "D", "D"), None);
    assert_eq!(callee(CmpOpKind::Is, "D", "D"), None);
}

#[test]
fn instance_identity_compares_pointers() {
    let c = instance("C");
    assert_eq!(instance_identity_op(CmpOpKind::Is, &c, &c), CmpOpKind::Eq);
    assert_eq!(
        instance_identity_op(CmpOpKind::IsNot, &c, &c),
        CmpOpKind::NotEq
    );
    assert_eq!(
        instance_identity_op(CmpOpKind::Is, &Ty::Int, &c),
        CmpOpKind::Is
    );
}

#[test]
fn each_link_takes_its_kind_from_its_own_pair() {
    let chain = lower_compare_chain(
        name("p", instance("D")),
        vec![
            (CmpOpKind::Eq, name("q", instance("D"))),
            (CmpOpKind::NotEq, name("r", instance("D"))),
            (CmpOpKind::Gt, name("r", instance("D"))),
            (CmpOpKind::Eq, name("n", Ty::Int)),
        ],
        &classes(),
        &[],
    );
    let MirExpr::CompareChain { first, links } = chain else {
        panic!("expected a chain, got {chain:?}");
    };
    assert_eq!(*first, name("p", instance("D")));
    let kinds: Vec<MirCompareKind> = links.into_iter().map(|link| link.kind).collect();
    assert_eq!(
        kinds,
        vec![
            MirCompareKind::InstanceDunder {
                callee: "D.__eq__".to_string(),
                negate: false,
            },
            MirCompareKind::InstanceDunder {
                callee: "D.__eq__".to_string(),
                negate: true,
            },
            MirCompareKind::Plain(CmpOpKind::Gt),
            MirCompareKind::Plain(CmpOpKind::Eq),
        ]
    );
}
