use super::*;
use crate::{ClassAttrValue, PropertyDef, Ty};
use std::collections::HashMap;

fn class(name: &str, mro: &[&str]) -> HirClassDef {
    HirClassDef {
        class_attrs: Vec::new(),
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

fn method(def: &mut HirClassDef, m: &str) {
    def.methods
        .push((m.to_string(), format!("{}.{m}", def.name)));
}

/// `A` defines `g`, `p` (getter and setter), `k` (classmethod), `s`
/// (static) and class attribute `X`; `B(A)` defines nothing; `C(B)`
/// overrides `g`.
fn tables() -> HashMap<String, HirClassDef> {
    let mut a = class("A", &["A"]);
    method(&mut a, "g");
    a.properties.push(PropertyDef {
        name: "p".to_string(),
        getter: "A.p".to_string(),
        setter: Some("A.p.setter".to_string()),
    });
    a.class_methods
        .push(("k".to_string(), "A.k.classmethod".to_string()));
    a.static_methods
        .push(("s".to_string(), "A.s.static".to_string()));
    a.class_attrs
        .push(("X".to_string(), Ty::Int, ClassAttrValue::Int(1)));
    let b = class("B", &["B", "A"]);
    let mut c = class("C", &["C", "B", "A"]);
    method(&mut c, "g");
    [a, b, c].into_iter().map(|d| (d.name.clone(), d)).collect()
}

#[test]
fn primary_copies_are_spelled_receiver_style_and_round_trip() {
    let t = tables();
    let of = |n: &str| t.get(n);
    let b = &t["B"];
    for (origin, copy, kind, member) in [
        ("A.g", "B.g", CopiedMemberKind::Method, "g"),
        ("A.p", "B.p", CopiedMemberKind::Method, "p"),
        ("A.p.setter", "B.p.setter", CopiedMemberKind::Setter, "p"),
        (
            "A.k.classmethod",
            "B.k.classmethod",
            CopiedMemberKind::ClassMethod,
            "k",
        ),
    ] {
        assert_eq!(
            inherited_copy_name(b, "A", origin, &of).as_deref(),
            Some(copy)
        );
        assert_eq!(
            inherited_copy_origin(copy, &of),
            Some(InheritedCopy {
                receiver: "B".to_string(),
                origin_class: "A".to_string(),
                origin_name: origin.to_string(),
                member: member.to_string(),
                kind,
            })
        );
    }
}

#[test]
fn a_static_method_and_a_foreign_name_have_no_copy_name() {
    let t = tables();
    let of = |n: &str| t.get(n);
    assert_eq!(inherited_copy_name(&t["B"], "A", "A.s.static", &of), None);
    assert_eq!(inherited_copy_name(&t["B"], "A", "Z.g", &of), None);
    assert_eq!(inherited_copy_name(&t["B"], "A", "A.g.x.y", &of), None);
    assert_eq!(inherited_copy_origin("B.s.static", &of), None);
}

#[test]
fn a_super_target_copy_uses_the_marker_spelling() {
    let t = tables();
    let of = |n: &str| t.get(n);
    // `C` defines `g` itself, so `A.g` compiled for `C` (reached through
    // `super()`) cannot be named `C.g`.
    let name = inherited_copy_name(&t["C"], "A", "A.g", &of).unwrap();
    assert_eq!(name, "C.g.0super_A");
    let copy = inherited_copy_origin(&name, &of).unwrap();
    assert_eq!(copy.receiver, "C");
    assert_eq!(copy.origin_class, "A");
    assert_eq!(copy.origin_name, "A.g");
    assert_eq!(copy.kind, CopiedMemberKind::Method);
}

#[test]
fn a_super_target_copy_of_a_setter_or_classmethod_keeps_its_kind() {
    let mut t = tables();
    // `C` overrides the property `p` and the classmethod `k`, so `A`'s
    // bodies compiled for `C` take the marker spelling plus the kind suffix.
    let c = t.get_mut("C").unwrap();
    c.properties.push(PropertyDef {
        name: "p".to_string(),
        getter: "C.p".to_string(),
        setter: Some("C.p.setter".to_string()),
    });
    c.class_methods
        .push(("k".to_string(), "C.k.classmethod".to_string()));
    let of = |n: &str| t.get(n);
    for (origin, expected, kind) in [
        (
            "A.p.setter",
            "C.p.0super_A.setter",
            CopiedMemberKind::Setter,
        ),
        (
            "A.k.classmethod",
            "C.k.0super_A.classmethod",
            CopiedMemberKind::ClassMethod,
        ),
        ("A.p", "C.p.0super_A", CopiedMemberKind::Method),
    ] {
        let name = inherited_copy_name(&t["C"], "A", origin, &of).unwrap();
        assert_eq!(name, expected);
        let copy = inherited_copy_origin(&name, &of).unwrap();
        assert_eq!(copy.origin_class, "A");
        assert_eq!(copy.origin_name, origin);
        assert_eq!(copy.kind, kind);
    }
    assert_eq!(inherited_copy_origin("C.p.0super_A.static", &of), None);
    assert_eq!(inherited_copy_origin("C.p.0super_A.setter.x", &of), None);
}

#[test]
fn own_items_and_malformed_names_are_not_copies() {
    let t = tables();
    let of = |n: &str| t.get(n);
    for name in [
        "A.g",            // own item of the root
        "C.g",            // an override
        "f",              // a module-level function
        "Z.g",            // unknown receiver
        "B.X",            // a class attribute, not a method item
        "B.nope",         // no definer
        "C.g.0super_Z",   // unknown origin
        "C.g.0super_C",   // origin is the receiver itself
        "A.g.0super_C",   // origin not in the receiver's MRO
        "C.X.0super_A",   // origin does not own the item
        "C.g.0super_A.x", // trailing segment
        "B.g.extra",      // not a known suffix
    ] {
        assert_eq!(inherited_copy_origin(name, &of), None, "{name}");
    }
}

#[test]
fn first_definer_and_binds_member_cover_every_member_kind() {
    let t = tables();
    let of = |n: &str| t.get(n);
    for member in ["g", "p", "k", "s", "X"] {
        assert!(binds_member(&t["A"], member), "{member}");
        assert_eq!(first_definer(&t["B"], member, &of).unwrap().name, "A");
    }
    assert_eq!(first_definer(&t["C"], "g", &of).unwrap().name, "C");
    assert!(first_definer(&t["C"], "nope", &of).is_none());
}
