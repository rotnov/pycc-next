use super::*;

fn class(name: &str, mro: &[&str], methods: &[&str]) -> HirClassDef {
    HirClassDef {
        class_attrs: Vec::new(),
        exception_type_tag: None,
        name: name.to_string(),
        bases: Vec::new(),
        mro: mro.iter().map(|m| (*m).to_string()).collect(),
        attrs: Vec::new(),
        methods: methods
            .iter()
            .map(|m| ((*m).to_string(), format!("{name}.{m}")))
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
        class("A", &["A"], &["m", "g"]),
        class("B", &["B", "A"], &["m"]),
    ]
    .into_iter()
    .map(|d| (d.name.clone(), d))
    .collect()
}

fn scope_with(names: &[&str]) -> Vec<HashMap<String, Ty>> {
    vec![
        names
            .iter()
            .map(|n| (format!("$fn:{n}"), Ty::Int))
            .collect(),
    ]
}

#[test]
fn an_existing_copy_for_the_receiver_is_called() {
    let callee = exact_callee(
        "B",
        "A",
        "A.g".to_string(),
        &scope_with(&["B.g"]),
        &classes(),
    );
    assert_eq!(callee, "B.g");
}

#[test]
fn the_found_item_is_called_without_a_copy() {
    let classes = classes();
    // No copy was materialized for `B`.
    assert_eq!(
        exact_callee("B", "A", "A.g".to_string(), &scope_with(&[]), &classes),
        "A.g"
    );
    // The receiver defines the member itself.
    assert_eq!(
        exact_callee("A", "A", "A.g".to_string(), &scope_with(&["B.g"]), &classes),
        "A.g"
    );
    // An unknown receiver class keeps the found item.
    assert_eq!(
        exact_callee("Z", "A", "A.g".to_string(), &scope_with(&["Z.g"]), &classes),
        "A.g"
    );
}

#[test]
fn only_an_instance_type_names_a_receiver_class() {
    assert_eq!(
        receiver_class(&Ty::Instance(Box::new("B".to_string()))),
        Some("B")
    );
    assert_eq!(receiver_class(&Ty::Int), None);
}
