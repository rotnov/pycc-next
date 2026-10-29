//! Constructibility and constructor descriptors.

use super::*;

#[test]
fn an_instance_method_of_a_constructible_class_is_exported_with_a_real_receiver() {
    let exports = collect_exports(&constructible_module("Grid")).expect("a carriable signature");
    assert_eq!(
        exports
            .iter()
            .map(|e| (
                e.name.as_str(),
                e.class.as_deref(),
                e.method.as_deref(),
                e.receiver,
                e.params.len()
            ))
            .collect::<Vec<_>>(),
        // `__init__` is not public under `is_public_name`, so it is never an
        // export in its own right; only `area` is, and its `self` is split
        // off exactly as a `@classmethod`'s `cls` is.
        vec![(
            "Grid.area",
            Some("Grid"),
            Some("area"),
            ExtReceiver::SelfInstance,
            0
        )]
    );
}

#[test]
fn a_constructible_class_yields_one_constructor_descriptor_with_the_carried_tail_only() {
    let hir = constructible_module("Grid");
    let exports = collect_exports(&hir).expect("a carriable signature");
    assert_eq!(
        collect_constructors(&hir, &collect_class_publications(&hir, &exports)),
        vec![ExtCtor {
            defaults: Vec::new(),
            class: "Grid".to_string(),
            name: "Grid.__init__".to_string(),
            // `self` is gone; the two `int`s remain, and the slot layout is
            // `flat_attr_layout`'s, not the parameter list -- they agree in
            // count here only because the fixture declares two attributes.
            param_names: vec!["w".to_string(), "h".to_string()],
            params: vec![Ty::Int, Ty::Int],
            param_writable: vec![false; 2],
            slot_names: vec!["w".to_string(), "h".to_string()],
            slot_kinds: "ii".to_string(),
            keyword_names: None,
            keyword_receiver: None,
            vararg: None,
        }]
    );
    // #1442: both `int` slots are carriable, so both are descriptors,
    // indexed in slot order (writable since Part 1 of #1443); since #1448
    // the table is keyed by carrier class rather than held by the ctor.
    assert_eq!(grid_getsets(&hir), grid_slot_getsets());
}

/// Each row removes exactly one constructibility condition from
/// [`constructible_module`] and asserts both consequences at once: the class
/// contributes no constructor descriptor, and its instance method is not an
/// export. The second half is the one that matters -- an exclusion that
/// dropped the constructor but still published the method would emit a
/// wrapper that dereferences a receiver no host can ever build.
#[test]
fn every_constructibility_condition_removes_the_class_and_its_instance_methods() {
    let mut rows: Vec<(&str, HirModule)> = Vec::new();

    let mut abstract_class = constructible_module("Grid");
    abstract_class.class_defs[0].1.is_abstract = true;
    rows.push(("is_abstract", abstract_class));

    let mut protocol = constructible_module("Grid");
    protocol.class_defs[0].1.is_protocol = true;
    rows.push(("is_protocol", protocol));

    let mut enum_class = constructible_module("Grid");
    enum_class.class_defs[0].1.is_enum = true;
    rows.push(("is_enum", enum_class));

    let mut tagged = constructible_module("Grid");
    tagged.class_defs[0].1.exception_type_tag = Some(FIRST_USER_EXCEPTION_TYPE_TAG);
    rows.push(("exception_type_tag", tagged));

    // A seeded builtin exception class carries no tag of its own, so this
    // row is the one `exception_type_tag` alone does not cover.
    let mut builtin_exception = constructible_module("ValueError");
    builtin_exception.class_defs[0].1.exception_type_tag = None;
    rows.push(("is_builtin_exception_class", builtin_exception));

    // An `__init__` that does not return `None` is not a constructor pycc
    // can call for its effect; nothing consumes the value at this boundary.
    let mut returning_init = constructible_module("Grid");
    returning_init.items[0] = init_func("Grid", &[("w", Ty::Int), ("h", Ty::Int)], Ty::Int);
    rows.push(("__init__ return type", returning_init));

    // Since Part 1 of #1447 (#1449) a `Ty::Instance` is carried exactly when
    // it names a regular class of this module, so the parameter that keeps a
    // class non-constructible is an instance of a class the module's table
    // does not hold -- the case a predicate on the type alone would miss.
    // The admitted counterpart is
    // `an_init_taking_a_same_module_instance_makes_the_class_constructible`.
    let mut instance_param = constructible_module("Grid");
    instance_param.items[0] = init_func(
        "Grid",
        &[("other", Ty::Instance(Box::new("Elsewhere".to_string())))],
        Ty::None,
    );
    rows.push(("uncarriable __init__ parameter", instance_param));

    // A `tuple` parameter is carriable everywhere else and refused here
    // alone: the constructor is called through its `fnptr_` slot with no
    // thunk, so no flattening exists for an aggregate (C8).
    let mut tuple_param = constructible_module("Grid");
    tuple_param.items[0] = init_func(
        "Grid",
        &[("wh", Ty::Tuple(Box::new(vec![Ty::Int, Ty::Int])))],
        Ty::None,
    );
    rows.push(("tuple __init__ parameter", tuple_param));

    // No `__init__` resolves at all: `methods` is empty, so `resolved_init`
    // finds nothing in the MRO.
    let mut no_init = constructible_module("Grid");
    no_init.class_defs[0].1.methods.clear();
    rows.push(("no resolved __init__", no_init));

    for (label, hir) in rows {
        let exports = collect_exports(&hir).expect("{label}: not a capability gap");
        assert!(
            exports.is_empty(),
            "{label}: an excluded class still exported {exports:?}"
        );
        assert!(
            collect_constructors(&hir, &collect_class_publications(&hir, &exports)).is_empty(),
            "{label}: an excluded class still yielded a constructor"
        );
    }
}

#[test]
fn a_class_whose_class_def_is_missing_entirely_is_not_constructible() {
    // The class table is what every constructibility condition is read
    // from, so a name with no entry has to fail closed rather than fall
    // through to "nothing refused it".
    let hir = module(vec![
        init_func("Grid", &[("w", Ty::Int)], Ty::None),
        func(
            "Grid.area",
            &[("self", Ty::Instance(Box::new("Grid".to_string())))],
            Ty::Int,
        ),
    ]);
    let exports = collect_exports(&hir).expect("not a capability gap");
    assert!(exports.is_empty(), "{exports:?}");
    assert!(collect_constructors(&hir, &collect_class_publications(&hir, &exports)).is_empty());
}

#[test]
fn a_property_getter_is_excluded_by_the_class_table_not_by_its_name() {
    // The getter's mangled name is the bare `<Class>.<method>` spelling, so
    // nothing lexical separates it from an ordinary method: only
    // `HirClassDef::properties` does. Deleting that driver filter publishes
    // `v` as a callable member.
    let mut hir = constructible_module("Grid");
    hir.items.push(func(
        "Grid.v",
        &[("self", Ty::Instance(Box::new("Grid".to_string())))],
        Ty::Int,
    ));
    hir.class_defs[0].1.properties = vec![pycc_hir::PropertyDef {
        name: "v".to_string(),
        getter: "Grid.v".to_string(),
        setter: None,
    }];
    let exports = collect_exports(&hir).expect("not a capability gap");
    assert_eq!(
        exports.iter().map(|e| e.name.as_str()).collect::<Vec<_>>(),
        vec!["Grid.area"]
    );
}

#[test]
fn the_constructor_is_found_past_an_unrelated_item_of_the_same_kind() {
    // `resolved_init` scans `HirModule::items` for the mangled name the MRO
    // walk produced. Every module of any size has functions before it that
    // are `HirItem::Function` too, so the name guard -- not the variant --
    // is what selects the constructor. With the constructor first in the
    // list, a guard that always matched would pass unnoticed.
    let mut hir = constructible_module("Grid");
    hir.items
        .insert(0, func("decoy", &[("n", Ty::Int)], Ty::Int));
    let (name, params, return_ty) =
        resolved_init(&hir, "Grid").expect("the constructor is still resolved");
    assert_eq!(name, "Grid.__init__");
    assert_eq!(params.len(), 3, "{params:?}");
    assert_eq!(*return_ty, Ty::None);
}

#[test]
fn an_implicit_object_init_ranks_below_a_real_one_in_the_same_mro() {
    // D-232/#966: a base whose `__init__` is the D-225 implicit zero-argument
    // constructor must never win over a real one further along the MRO, or a
    // subclass with a two-argument constructor would be published as taking
    // none. The two-pass ranking is what makes the first pass skip it.
    let mut hir = constructible_module("Grid");
    let mut base = constructible_class_def("Base");
    base.implicit_object_init = true;
    hir.class_defs[0].1.mro = vec!["Base".to_string(), "Grid".to_string()];
    hir.class_defs.push(("Base".to_string(), base));
    hir.items.push(init_func("Base", &[], Ty::None));
    let exports = collect_exports(&hir).expect("not a capability gap");
    let ctors = collect_constructors(&hir, &collect_class_publications(&hir, &exports));
    // `Base` is published too since #1450 (it is constructible through its
    // implicit constructor), so the ranking is read off `Grid`'s own row.
    assert_eq!(
        ctors
            .iter()
            .find(|c| c.class == "Grid")
            .map(|c| c.name.as_str()),
        Some("Grid.__init__"),
        "the implicit `object.__init__` outranked the real one: {ctors:?}"
    );
}

#[test]
fn an_implicit_object_init_is_still_resolved_when_it_is_the_only_one() {
    // The second pass of the same ranking: with no real `__init__` anywhere
    // in the MRO, the implicit one is the constructor, and a class with it
    // is constructible with zero carried arguments.
    let mut hir = constructible_module("Grid");
    hir.class_defs[0].1.implicit_object_init = true;
    hir.items[0] = init_func("Grid", &[], Ty::None);
    let exports = collect_exports(&hir).expect("not a capability gap");
    assert_eq!(
        collect_constructors(&hir, &collect_class_publications(&hir, &exports)),
        vec![ExtCtor {
            defaults: Vec::new(),
            class: "Grid".to_string(),
            name: "Grid.__init__".to_string(),
            param_names: crate::ext_build::test_param_names(&[]),
            params: Vec::new(),
            param_writable: Vec::new(),
            slot_names: vec!["w".to_string(), "h".to_string()],
            slot_kinds: "ii".to_string(),
            keyword_names: None,
            keyword_receiver: None,
            vararg: None,
        }]
    );
    // #1442: both `int` slots are carriable, so both are descriptors,
    // indexed in slot order (writable since Part 1 of #1443); since #1448
    // the table is keyed by carrier class rather than held by the ctor.
    assert_eq!(grid_getsets(&hir), grid_slot_getsets());
}

#[test]
fn an_instance_method_of_a_constructible_class_with_an_uncarriable_signature_is_a_gap() {
    // The ordering claim D-244 rule 1 rests on: representation exclusions
    // run *before* `unsupported_boundary_ty`, so only a method that survived
    // them can be a capability gap -- and once it has, it is held to the
    // boundary exactly like a module-level function.
    let mut hir = constructible_module("Grid");
    hir.items[1] = func(
        "Grid.area",
        &[
            ("self", Ty::Instance(Box::new("Grid".to_string()))),
            ("xs", Ty::List(Box::new(Ty::Int))),
        ],
        Ty::Int,
    );
    let gap = collect_exports(&hir).expect_err("an uncarriable parameter is a C0003");
    assert_eq!(gap.len(), 1, "{gap:?}");
    assert_eq!(gap[0].code, "C0003", "{gap:?}");
    assert!(gap[0].message.contains("Grid.area"), "{gap:?}");
}

#[test]
fn an_instance_method_of_a_non_constructible_class_is_never_a_gap() {
    // The other arm of the same ordering: the identical uncarriable
    // signature on a class pycc cannot construct is excluded as
    // representation and reported as nothing at all. This is what bounds
    // the new-failure set to constructible classes.
    let mut hir = constructible_module("Grid");
    hir.class_defs[0].1.is_abstract = true;
    hir.items[1] = func(
        "Grid.area",
        &[
            ("self", Ty::Instance(Box::new("Grid".to_string()))),
            ("xs", Ty::List(Box::new(Ty::Int))),
        ],
        Ty::Int,
    );
    assert!(
        collect_exports(&hir)
            .expect("excluded as representation, not reported as a gap")
            .is_empty()
    );
}

#[test]
fn a_constructor_descriptor_is_emitted_once_per_class_however_many_methods_it_exports() {
    // `collect_constructors` walks the export list, which carries one entry
    // per exported method; the `.inc` must declare one `tp_init` per class.
    let mut hir = constructible_module("Grid");
    hir.items.push(func(
        "Grid.perimeter",
        &[("self", Ty::Instance(Box::new("Grid".to_string())))],
        Ty::Int,
    ));
    declare_member(&mut hir, "Grid", Member::Method("perimeter"));
    let exports = collect_exports(&hir).expect("carriable");
    assert_eq!(exports.len(), 2);
    assert_eq!(
        collect_constructors(&hir, &collect_class_publications(&hir, &exports)).len(),
        1
    );
}

/// `Grid`'s descriptor table in `hir`, as the `--ext` build collects it
/// (#1448).
fn grid_getsets(hir: &HirModule) -> Vec<crate::ext_build::ExtGetset> {
    let tables = collect_carrier_getsets(hir, &collect_carrier_classes(hir));
    assert_eq!(tables.len(), 1, "only `Grid` has a table");
    assert_eq!(tables[0].class, "Grid");
    tables
        .into_iter()
        .next()
        .map(|table| table.getsets)
        .unwrap_or_default()
}

/// The two `int` slot descriptors [`constructible_module`]'s `Grid` exposes
/// (#1442).
fn grid_slot_getsets() -> Vec<crate::ext_build::ExtGetset> {
    ["w", "h"]
        .iter()
        .enumerate()
        .map(|(index, name)| crate::ext_build::ExtGetset::Slot {
            name: (*name).to_string(),
            index,
            ty: Ty::Int,
        })
        .collect()
}
