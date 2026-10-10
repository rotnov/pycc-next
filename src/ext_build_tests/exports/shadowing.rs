//! Member shadowing across the MRO at publication time.

use super::*;

// --- #1146: the walk resolves the namespace, not the export set ----------

/// [`inheriting_module`] with `Derived` shadowing `Base.value` with a
/// `@property` of the same name: the getter is an ordinary compiled
/// `Derived.value` item plus the `properties` entry that tells it apart.
fn property_shadowing_module() -> HirModule {
    let mut hir = inheriting_module();
    hir.items
        .push(func("Derived.value", &[("self", inst("Derived"))], Ty::Int));
    declare_member(&mut hir, "Derived", Member::Property("value"));
    hir
}

#[test]
fn a_derived_property_getter_shadows_its_base_method_and_publishes_nothing() {
    // The #1146 defect, at the unit seam. `collect_exports` has already
    // removed the getter from the export set, so a walk that stops at the
    // first *exportable* hit fell through to `Base.value` and published a
    // `method_descriptor` returning Base's answer where Python's own
    // attribute lookup gives the derived property. Resolving the namespace
    // first stops at `Derived`, which owns the name.
    assert_eq!(
        publication_rows(&publications_of(&property_shadowing_module())),
        vec![
            ("Base", vec!["Base.value"]),
            ("Derived", vec!["Derived.twice"]),
        ]
    );
}

#[test]
fn a_derived_instance_attribute_shadows_its_base_method_and_publishes_nothing() {
    // `__init__`'s `self.value = ...` binds `value` on the instance, and
    // Python resolves `obj.value` against the instance before the type, so
    // the derived slot wins over `Base.value` exactly as an override would.
    // `pycc_hir` accepts the collision -- verified end to end: the same
    // source built with `--ext` published a callable `Derived.value`
    // answering Base's 21 where CPython reads the slot's own value -- so
    // this walk is the only place it can be seen.
    let mut hir = inheriting_module();
    hir.class_defs[1]
        .1
        .attrs
        .push(("value".to_string(), Ty::Int));
    assert_eq!(
        publication_rows(&publications_of(&hir)),
        vec![
            ("Base", vec!["Base.value"]),
            ("Derived", vec!["Derived.twice"]),
        ]
    );
}

#[test]
fn a_sibling_base_s_class_attribute_shadows_a_further_base_s_method() {
    // A class attribute is an ordinary entry in the class object's
    // namespace, so an MRO entry binding `value` as a `ClassVar` answers
    // the name and the walk must stop there rather than reach `Base`'s
    // method behind it. `pycc_hir` accepts this shape: its class-attribute
    // collision check runs over a class's *own* newly declared attributes,
    // so `Derived`, which declares none, combines two independent bases
    // without complaint. Verified end to end: before the fix, the same
    // source built with `--ext` published a callable `mod.Derived(3).f()`
    // answering `3` where CPython raises `TypeError: 'int' object is not
    // callable`, because `d.f` is the sibling base's `2`.
    let mut hir = inheriting_module();
    let mut shadowing = class_def("Shadowing", None);
    shadowing.class_attrs = vec![(
        "value".to_string(),
        Ty::Int,
        pycc_hir::ClassAttrValue::Int(2),
    )];
    hir.class_defs.push(("Shadowing".to_string(), shadowing));
    hir.class_defs[1].1.mro = vec![
        "Derived".to_string(),
        "Shadowing".to_string(),
        "Base".to_string(),
    ];
    assert_eq!(
        publication_rows(&publications_of(&hir)),
        vec![
            ("Base", vec!["Base.value"]),
            ("Derived", vec!["Derived.twice"]),
        ]
    );
}

#[test]
fn a_class_binding_one_name_both_ways_publishes_the_slot_s_neighbours_only() {
    // The same class assigns `self.value` in `__init__` *and* declares
    // `def value`. `pycc_hir` retains both bindings, and a walk that
    // modelled the slot as one more namespace kind would find `Base` owning
    // the name either way and publish the method. Python does not: the
    // instance `__dict__` answers `obj.value` ahead of a non-data
    // descriptor, so the compiled method is unreachable. Verified end to
    // end: before the fix, that source built with `--ext` answered
    // `mod.C(1).value()` with `5` where CPython raises `TypeError: 'int'
    // object is not callable`. `Derived.twice` is the positive direction --
    // an unrelated name on a class whose MRO carries the slot still
    // publishes. `Base` keeps a type object with no method row: it is
    // constructible, and #1450 publishes a constructible class whatever its
    // resolved set holds.
    let mut hir = inheriting_module();
    hir.class_defs[0]
        .1
        .attrs
        .push(("value".to_string(), Ty::Int));
    assert_eq!(
        publication_rows(&publications_of(&hir)),
        vec![("Derived", vec!["Derived.twice"]), ("Base", vec![])]
    );
}

#[test]
fn a_base_s_instance_slot_shadows_a_derived_class_s_method() {
    // The shape a position-sensitive walk cannot answer: the slot is
    // declared by the *least* derived class and the method by the most
    // derived one, so the first MRO entry binding `twice` is `Derived`
    // itself. CPython still reads the instance slot, because its precedence
    // over a non-data descriptor does not depend on where in the MRO the
    // slot was assigned. Verified end to end: before the fix, a `Base`
    // assigning `self.value` with a `Derived(Base)` declaring `def value`
    // built with `--ext` and answered `mod.Derived(7).value()` with `5`
    // where CPython raises `TypeError`. `Base.value` is the positive
    // direction: unshadowed, it is still published on both classes.
    let mut hir = inheriting_module();
    hir.class_defs[0]
        .1
        .attrs
        .push(("twice".to_string(), Ty::Int));
    assert_eq!(
        publication_rows(&publications_of(&hir)),
        vec![
            ("Base", vec!["Base.value"]),
            ("Derived", vec!["Base.value"])
        ]
    );
}

#[test]
fn a_protocol_base_s_declared_method_shadows_a_further_base_s_export() {
    // A `Protocol`'s declaration-style `def f(self) -> int: ...` is a real
    // function object in the protocol's namespace, so CPython answers it and
    // not a later base's binding of the same name. Verified end to end:
    // `class Q(P, A)` with `P` declaring `f` and `A` exporting a
    // `@staticmethod f` built with `--ext` and answered `mod.Q.f()` with `7`,
    // where CPython raises `TypeError: P.f() missing 1 required positional
    // argument: 'self'`. The protocol binding is not itself exportable, so
    // the name is now published by no one, while `Base.value` -- the name no
    // protocol member binds -- is inherited unchanged.
    let mut hir = inheriting_module();
    hir.class_defs[1]
        .1
        .protocol_members
        .push(pycc_hir::ProtocolMember::Method {
            name: "value".to_string(),
            param_tys: Vec::new(),
            return_ty: Ty::Int,
        });
    assert_eq!(
        publication_rows(&publications_of(&hir)),
        vec![
            ("Base", vec!["Base.value"]),
            ("Derived", vec!["Derived.twice"])
        ]
    );
}

#[test]
fn a_protocol_base_s_annotation_only_attribute_shadows_nothing() {
    // The other half of `protocol_members`: `x: int` in a class body
    // declares a type and binds no namespace entry, in a `Protocol` exactly
    // as anywhere else, so it must not suppress a name a real base exports.
    let mut hir = inheriting_module();
    hir.class_defs[1]
        .1
        .protocol_members
        .push(pycc_hir::ProtocolMember::Attribute {
            name: "value".to_string(),
            ty: Ty::Int,
        });
    assert_eq!(
        publication_rows(&publications_of(&hir)),
        vec![
            ("Base", vec!["Base.value"]),
            ("Derived", vec!["Derived.twice", "Base.value"])
        ]
    );
}

#[test]
fn a_constructible_class_whose_whole_resolved_set_is_shadowed_away_publishes_no_method() {
    // The collapse case the per-name matrix above never reaches: every one
    // of `Derived`'s resolved names is shadowed by a non-exporting binding,
    // so its row carries no method at all. Before #1450 such a class got no
    // type object; it is constructible, so it is now published with no
    // exported method, as CPython's own module still answers
    // `mod.Derived(...)` -- with `value` read from the instance, not called.
    let mut hir = inheriting_module();
    hir.class_defs[1]
        .1
        .methods
        .retain(|(name, _)| name != "twice");
    hir.items
        .retain(|item| !matches!(item, HirItem::Function { name, .. } if name == "Derived.twice"));
    hir.class_defs[1]
        .1
        .attrs
        .push(("value".to_string(), Ty::Int));
    assert_eq!(
        publication_rows(&publications_of(&hir)),
        vec![("Base", vec!["Base.value"]), ("Derived", vec![])]
    );
}

#[test]
fn a_derived_method_shadowing_a_base_property_is_published_unchanged() {
    // The mirror direction, which must not regress: `Base` binds `value` as
    // a `@property` and publishes nothing for it, while `Derived`'s ordinary
    // method owns the name on `Derived` and is published there. A
    // non-regression guard rather than a discriminator: the base contributes
    // no export under the name, so nothing exists for a first-exportable-hit
    // walk to fall through to and this shape answered the same before #1146.
    // `Base` exports nothing but is constructible, so since #1450 it is
    // published with no exported method.
    let mut hir = inheriting_module();
    hir.class_defs[0]
        .1
        .methods
        .retain(|(name, _)| name != "value");
    declare_member(&mut hir, "Base", Member::Property("value"));
    hir.items
        .push(func("Derived.value", &[("self", inst("Derived"))], Ty::Int));
    declare_member(&mut hir, "Derived", Member::Method("value"));
    assert_eq!(
        publication_rows(&publications_of(&hir)),
        vec![
            ("Derived", vec!["Derived.twice", "Derived.value"]),
            ("Base", vec![])
        ]
    );
}

#[test]
fn a_derived_abstract_method_shadows_its_base_s_concrete_one() {
    // An `@abstractmethod` binds its name in `methods` exactly as an
    // ordinary method does, and exports nothing of its own
    // (`instance_shape_admissible` refuses an abstract class). `Derived`
    // therefore publishes no `value` at all -- publishing `Base.value` there
    // would answer a name whose derived binding is a stub.
    let mut hir = inheriting_module();
    hir.class_defs[1].1.is_abstract = true;
    hir.class_defs[1]
        .1
        .abstract_methods
        .push("value".to_string());
    declare_member(&mut hir, "Derived", Member::Method("value"));
    assert_eq!(
        publication_rows(&publications_of(&hir)),
        vec![("Base", vec!["Base.value"])]
    );
}

#[test]
fn a_derived_static_or_class_method_shadows_its_base_instance_method() {
    // A shadowing binding of a *different* receiver kind is still the
    // binding Python resolves, and it is published under its own kind: the
    // derived `@staticmethod` wins on `Derived` while `Base` keeps its
    // instance method. The first two arms are non-regression guards -- both
    // bindings are exports, so a first-exportable-hit walk reached the same
    // rows -- and the third discriminates: the shadowing binding of a
    // different kind is not itself an export.
    let mut statics = inheriting_module();
    statics
        .items
        .push(func("Derived.value.static", &[], Ty::Int));
    declare_member(&mut statics, "Derived", Member::Static("value"));
    assert_eq!(
        publication_rows(&publications_of(&statics)),
        vec![
            ("Base", vec!["Base.value"]),
            ("Derived", vec!["Derived.twice", "Derived.value.static"]),
        ]
    );

    // The same for a `@classmethod`, whose table is its own as well.
    let mut classmethods = inheriting_module();
    classmethods.items.push(func(
        "Derived.value.classmethod",
        &[("cls", inst("Derived"))],
        Ty::Int,
    ));
    classmethods.class_defs[1]
        .1
        .class_methods
        .push(("value".to_string(), "Derived.value.classmethod".to_string()));
    assert_eq!(
        publication_rows(&publications_of(&classmethods)),
        vec![
            ("Base", vec!["Base.value"]),
            (
                "Derived",
                vec!["Derived.twice", "Derived.value.classmethod"]
            ),
        ]
    );

    // The kind pair the export set alone cannot answer: `Base` exports `tag`
    // as a `@staticmethod` and `Derived` rebinds the name as a `@property`,
    // which exports nothing. Stopping at the first exportable hit publishes
    // `mod.Derived.tag()` as a callable the Python program does not have.
    let mut over_static = inheriting_module();
    over_static
        .items
        .push(func("Base.tag.static", &[], Ty::Int));
    declare_member(&mut over_static, "Base", Member::Static("tag"));
    over_static
        .items
        .push(func("Derived.tag", &[("self", inst("Derived"))], Ty::Int));
    declare_member(&mut over_static, "Derived", Member::Property("tag"));
    assert_eq!(
        publication_rows(&publications_of(&over_static)),
        vec![
            ("Base", vec!["Base.value", "Base.tag.static"]),
            ("Derived", vec!["Derived.twice", "Base.value"]),
        ]
    );
}

#[test]
fn a_derived_instance_method_shadows_its_base_static_method() {
    // And the reverse direction: an ordinary method shadowing an inherited
    // `@staticmethod`. `Base.tag.static` stays published on `Base` itself.
    // Another non-regression guard: both bindings are exports, so this shape
    // answered the same under the first-exportable-hit walk.
    let mut hir = inheriting_module();
    hir.items.push(func("Base.tag.static", &[], Ty::Int));
    declare_member(&mut hir, "Base", Member::Static("tag"));
    hir.items
        .push(func("Derived.tag", &[("self", inst("Derived"))], Ty::Int));
    declare_member(&mut hir, "Derived", Member::Method("tag"));
    assert_eq!(
        publication_rows(&publications_of(&hir)),
        vec![
            ("Base", vec!["Base.value", "Base.tag.static"]),
            // `value` is not shadowed, so `Derived` still inherits it.
            (
                "Derived",
                vec!["Derived.twice", "Derived.tag", "Base.value"]
            ),
        ]
    );
}

#[test]
fn a_base_method_every_witness_shadows_is_not_exported_and_never_a_c0003() {
    // The export side of the same rule. `Base` is unconstructible (a `tuple`
    // `__init__`), so its only witness is `Derived` -- which shadows `value`
    // with a `@property`. No host instance can ever reach `Base.value`'s
    // compiled body, so it is excluded as representation: a dead
    // `PyMethodDef` row on `Base`'s own type object with the carriable
    // signature, and a whole failed `--ext` build with an uncarriable one.
    let mut hir = property_shadowing_module();
    hir.items[0] = init_func(
        "Base",
        &[("p", Ty::Tuple(Box::new(vec![Ty::Int, Ty::Int])))],
        Ty::None,
    );
    let exports = collect_exports(&hir).expect("a carriable program");
    assert_eq!(
        exports.iter().map(|e| e.name.as_str()).collect::<Vec<_>>(),
        vec!["Derived.twice"]
    );

    hir.items[1] = func(
        "Base.value",
        &[("self", inst("Base")), ("xs", Ty::List(Box::new(Ty::Int)))],
        Ty::Int,
    );
    assert_eq!(
        collect_exports(&hir)
            .expect("a shadowed method is excluded as representation, never a C0003")
            .iter()
            .map(|e| e.name.as_str())
            .collect::<Vec<_>>(),
        vec!["Derived.twice"]
    );
}
