//! Unit tests for chained-comparison MIR lowering (#1212): the per-link
//! kind, including the same-dataclass `__eq__` lookup's refusals.

use super::*;

fn class(name: &str, is_dataclass: bool) -> HirClassDef {
    HirClassDef {
        class_attrs: Vec::new(),
        exception_type_tag: None,
        name: name.to_string(),
        bases: Vec::new(),
        mro: vec![name.to_string()],
        attrs: Vec::new(),
        methods: vec![("__eq__".to_string(), format!("{name}.__eq__"))],
        type_param: None,
        properties: Vec::new(),
        static_methods: Vec::new(),
        class_methods: Vec::new(),
        is_enum: false,
        implicit_object_init: false,
        method_defaults: Vec::new(),
        enum_members: Vec::new(),
        is_dataclass,
        dataclass_fields: Vec::new(),
        is_protocol: false,
        runtime_checkable: false,
        protocol_members: Vec::new(),
        abstract_methods: Vec::new(),
        is_abstract: false,
    }
}

fn classes() -> HashMap<String, HirClassDef> {
    [("D", true), ("E", true), ("C", false)]
        .into_iter()
        .map(|(name, is_dataclass)| (name.to_string(), class(name, is_dataclass)))
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

#[test]
fn the_eq_callee_is_found_only_for_two_instances_of_one_dataclass() {
    let classes = classes();
    assert_eq!(
        dataclass_eq_callee(&instance("D"), &instance("D"), &classes),
        Some("D.__eq__".to_string())
    );
    // Not two instances.
    assert_eq!(
        dataclass_eq_callee(&Ty::Int, &instance("D"), &classes),
        None
    );
    // Two different classes.
    assert_eq!(
        dataclass_eq_callee(&instance("D"), &instance("E"), &classes),
        None
    );
    // An unregistered class.
    assert_eq!(
        dataclass_eq_callee(&instance("X"), &instance("X"), &classes),
        None
    );
    // A plain class.
    assert_eq!(
        dataclass_eq_callee(&instance("C"), &instance("C"), &classes),
        None
    );
}

#[test]
fn each_link_takes_its_kind_from_its_own_pair() {
    let chain = lower_compare_chain(
        name("p", instance("D")),
        vec![
            (CmpOpKind::Eq, name("q", instance("D"))),
            (CmpOpKind::NotEq, name("r", instance("D"))),
            (CmpOpKind::Lt, name("r", instance("D"))),
            (CmpOpKind::Eq, name("n", Ty::Int)),
        ],
        &classes(),
    );
    let MirExpr::CompareChain { first, links } = chain else {
        panic!("expected a chain, got {chain:?}");
    };
    assert_eq!(*first, name("p", instance("D")));
    let kinds: Vec<MirCompareKind> = links.into_iter().map(|link| link.kind).collect();
    assert_eq!(
        kinds,
        vec![
            MirCompareKind::DataclassEq {
                callee: "D.__eq__".to_string(),
                negate: false,
            },
            MirCompareKind::DataclassEq {
                callee: "D.__eq__".to_string(),
                negate: true,
            },
            MirCompareKind::Plain(CmpOpKind::Lt),
            MirCompareKind::Plain(CmpOpKind::Eq),
        ]
    );
}
