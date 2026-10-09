use super::*;

use pycc_mir::{HirClassDef, Ty};

fn class(name: &str, mro: &[&str], methods: &[&str]) -> (String, HirClassDef) {
    let def = HirClassDef {
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
    };
    (name.to_string(), def)
}

fn function(name: &str) -> MirItem {
    MirItem::Function {
        name: name.to_string(),
        params: Vec::new(),
        return_ty: Ty::Int,
        body: Vec::new(),
    }
}

/// `A.g` is defined twice; each definition has one copy for `B`, appended
/// after every item as the copy pass appends them.
#[test]
fn each_copy_binds_with_the_matching_occurrence_of_its_origin() {
    let mir = MirModule {
        items: vec![
            function("A.m"),
            function("A.g"),
            function("B.m"),
            function("A.g"),
            function("B.g"),
            function("B.g"),
        ],
        class_defs: vec![
            class("A", &["A"], &["m", "g"]),
            class("B", &["B", "A"], &["m"]),
        ],
    };
    let slots = CopySlots::new(&mir);
    assert_eq!(slots.bound_with(1), &[4]);
    assert_eq!(slots.bound_with(3), &[5]);
    assert!(slots.bound_with(0).is_empty());
    assert!(slots.is_copy(4) && slots.is_copy(5));
    assert!(!slots.is_copy(1) && !slots.is_copy(2));
}
