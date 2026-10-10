//! Publication of inherited members, abstract and private classes.

use super::*;

/// Drop both `class.method`'s compiled item and its method-table entry --
/// the inverse of [`declare_member`] for a regular method, used by the
/// fixtures that remove a class's only own export.
fn undeclare_method(hir: &mut HirModule, class: &str, method: &str) {
    let mangled = format!("{class}.{method}");
    hir.items
        .retain(|item| !matches!(item, HirItem::Function { name, .. } if *name == mangled));
    let (_, def) = hir
        .class_defs
        .iter_mut()
        .find(|(held, _)| held == class)
        .expect("the fixture defines the class it undeclares a method on");
    def.methods.retain(|(name, _)| name != method);
}

#[test]
fn an_inherited_instance_method_is_published_on_the_derived_class() {
    // The defect this fixes: `mod.Derived(21).twice()` worked while
    // `mod.Derived(21).value()` raised `AttributeError`, because the table
    // was built by declaring class. `Derived`'s own method comes first
    // because the walk is most-derived-first.
    assert_eq!(
        publication_rows(&publications_of(&inheriting_module())),
        vec![
            ("Base", vec!["Base.value"]),
            ("Derived", vec!["Derived.twice", "Base.value"]),
        ]
    );
}

#[test]
fn a_derived_override_shadows_its_base_at_the_first_mro_hit() {
    // First-MRO-hit-wins, which is the *opposite* direction from
    // `collect_exports`' own `(class, method)` dedup: that one keeps the
    // last binding, because a rebound name is what the class body means.
    // Here the derived definition is the one Python's attribute lookup
    // finds, so `Base.value` must not appear in `Derived`'s table at all.
    let mut hir = inheriting_module();
    hir.items
        .push(func("Derived.value", &[("self", inst("Derived"))], Ty::Int));
    declare_member(&mut hir, "Derived", Member::Method("value"));
    assert_eq!(
        publication_rows(&publications_of(&hir)),
        vec![
            ("Base", vec!["Base.value"]),
            ("Derived", vec!["Derived.twice", "Derived.value"]),
        ]
    );
}

#[test]
fn a_static_method_of_a_base_is_published_on_the_derived_class_too() {
    // One filter serves all three receiver kinds, so MRO-resolved
    // publication closes Part 1's identical gap for
    // `mod.Derived.static_from_base()`. Additive at the host surface: an
    // attribute appears, none disappears.
    let mut hir = inheriting_module();
    hir.items.push(func("Base.tag.static", &[], Ty::Int));
    declare_member(&mut hir, "Base", Member::Static("tag"));
    assert_eq!(
        publication_rows(&publications_of(&hir)),
        vec![
            ("Base", vec!["Base.value", "Base.tag.static"]),
            (
                "Derived",
                vec!["Derived.twice", "Base.value", "Base.tag.static"]
            ),
        ]
    );
}

#[test]
fn a_class_whose_only_exportable_members_are_inherited_is_published_and_constructible() {
    // The class list can no longer be read off the export set: `Derived`
    // declares no exportable member of its own, so the old loop gave it no
    // type object at all -- and therefore no `tp_init` either, by absence.
    let mut hir = inheriting_module();
    undeclare_method(&mut hir, "Derived", "twice");
    assert_eq!(
        publication_rows(&publications_of(&hir)),
        vec![
            ("Base", vec!["Base.value"]),
            ("Derived", vec!["Base.value"])
        ]
    );
    let ctors = collect_constructors(&hir, &publications_of(&hir));
    assert_eq!(
        ctors.iter().map(|c| c.class.as_str()).collect::<Vec<_>>(),
        vec!["Base", "Derived"]
    );
}

#[test]
fn an_unconstructible_base_still_exports_its_instance_method_for_a_constructible_subclass() {
    // Publication and constructibility are separate predicates. A `tuple`
    // parameter makes `Base` unconstructible (condition 4), but a `Derived`
    // instance still reaches `Base.value`'s compiled body, so the method is
    // exported and published -- while only `Derived` gets a `tp_init`.
    let mut hir = inheriting_module();
    hir.items[0] = init_func(
        "Base",
        &[("p", Ty::Tuple(Box::new(vec![Ty::Int, Ty::Int])))],
        Ty::None,
    );
    assert_eq!(
        publication_rows(&publications_of(&hir)),
        vec![
            ("Base", vec!["Base.value"]),
            ("Derived", vec!["Derived.twice", "Base.value"]),
        ]
    );
    let ctors = collect_constructors(&hir, &publications_of(&hir));
    assert_eq!(
        ctors.iter().map(|c| c.class.as_str()).collect::<Vec<_>>(),
        vec!["Derived"]
    );
}

/// [`inheriting_module`] with `Base` made unconstructible by a `tuple`
/// `__init__` and its only constructible subclass renamed to a **private**
/// spelling, so that subclass is never published.
///
/// `Derived.twice` goes with the rename: an inheriting class that exports
/// nothing of its own is the one shape that reaches publication without
/// having already passed `collect_exports`' own name filter, which is
/// exactly the shape whose witness has to be filtered here.
fn privately_derived_module() -> HirModule {
    let mut hir = inheriting_module();
    hir.items[0] = init_func(
        "Base",
        &[("p", Ty::Tuple(Box::new(vec![Ty::Int, Ty::Int])))],
        Ty::None,
    );
    hir.items[2] = init_func("_Priv", &[("w", Ty::Int), ("h", Ty::Int)], Ty::None);
    hir.items.truncate(3);
    hir.class_defs[1] = (
        "_Priv".to_string(),
        pycc_hir::HirClassDef {
            mro: vec!["_Priv".to_string(), "Base".to_string()],
            ..constructible_class_def("_Priv")
        },
    );
    hir
}

#[test]
fn a_privately_named_constructible_subclass_witnesses_nothing_for_its_base() {
    // The witness search has to hold the *publishability* condition
    // `collect_class_publications` applies, not constructibility alone.
    // `_Priv` is constructible but is published under no name, so the host
    // can never build an instance that reaches `Base.value`'s compiled body
    // -- exporting it would emit a `PyMethodDef` row with no obtainable
    // receiver. Drop `class_publishable` from the witness and `Base` starts
    // publishing `Base.value` again.
    let hir = privately_derived_module();
    assert!(
        collect_exports(&hir)
            .expect("a carriable program")
            .is_empty(),
        "an unpublished subclass makes its base's instance method unreachable"
    );
    assert!(publications_of(&hir).is_empty());
}

#[test]
fn an_unreachable_instance_method_with_an_uncarriable_signature_is_not_a_c0003() {
    // The same shape with a signature the boundary cannot carry. The
    // exclusion is representational and lands *before*
    // `unsupported_boundary_ty` is consulted, so a method nothing can ever
    // call must not fail the whole `--ext` build. Before the fix this exact
    // program exited 1 with `error[C0003]: ... parameter `xs: list`".
    let mut hir = privately_derived_module();
    hir.items[1] = func(
        "Base.value",
        &[("self", inst("Base")), ("xs", Ty::List(Box::new(Ty::Int)))],
        Ty::Int,
    );
    assert!(
        collect_exports(&hir)
            .expect("an unreachable method is excluded as representation, never a C0003")
            .is_empty()
    );
}

#[test]
fn a_published_abstract_class_is_refused_a_constructor_by_its_shape_alone() {
    // Publication is deliberately wider than constructibility: an abstract
    // class that exports a `@staticmethod` is published (Part 1's shape), so
    // it reaches `collect_constructors` even though nothing may ever
    // instantiate it. The refusal is `instance_shape_admissible`, not the
    // `__init__` conditions -- this fixture gives the class a perfectly
    // carriable `__init__` so only the shape half can answer.
    let mut hir = module_with_classes(
        vec![
            init_func("Grid", &[("w", Ty::Int)], Ty::None),
            func("Grid.scale.static", &[("n", Ty::Int)], Ty::Int),
        ],
        vec![("Grid".to_string(), constructible_class_def("Grid"))],
    );
    declare_member(&mut hir, "Grid", Member::Static("scale"));
    hir.class_defs[0].1.is_abstract = true;
    let publications = publications_of(&hir);
    assert_eq!(
        publications
            .iter()
            .map(|p| p.class.as_str())
            .collect::<Vec<_>>(),
        vec!["Grid"],
        "the staticmethod still publishes the type object"
    );
    assert!(
        collect_constructors(&hir, &publications).is_empty(),
        "an abstract class is never constructible, however carriable its `__init__` is"
    );
}

#[test]
fn an_abstract_base_exports_no_instance_method_however_constructible_its_subclass_is() {
    // The half of the old constructibility filter a constructible subclass
    // must *not* rescue: an `@abstractmethod`'s stub body returns nothing
    // while its `return_ty` says otherwise, so a wrapper over it would
    // return indeterminate storage. Only `Derived`'s own method is exported.
    let mut hir = inheriting_module();
    hir.class_defs[0].1.is_abstract = true;
    let exports = collect_exports(&hir).expect("excluded as representation, never a gap");
    assert_eq!(
        exports.iter().map(|e| e.name.as_str()).collect::<Vec<_>>(),
        vec!["Derived.twice"]
    );
}

#[test]
fn a_private_or_exception_inheriting_class_is_not_published() {
    // Both filters bite only here: every class in the export set already
    // passed them, so an inheriting class that exports nothing of its own
    // is the one shape that can reach publication without them.
    let mut private = inheriting_module();
    undeclare_method(&mut private, "Derived", "twice");
    private.class_defs[1].0 = "_Derived".to_string();
    private.class_defs[1].1.name = "_Derived".to_string();
    private.class_defs[1].1.mro = vec!["_Derived".to_string(), "Base".to_string()];
    assert_eq!(
        publication_rows(&publications_of(&private)),
        vec![("Base", vec!["Base.value"])]
    );

    let mut raising = inheriting_module();
    undeclare_method(&mut raising, "Derived", "twice");
    raising.class_defs[1].1.exception_type_tag = Some(FIRST_USER_EXCEPTION_TYPE_TAG);
    assert_eq!(
        publication_rows(&publications_of(&raising)),
        vec![("Base", vec!["Base.value"])]
    );
}

#[test]
fn a_class_absent_from_the_class_table_publishes_nothing() {
    // Neither predicate can answer without the class definition -- there is
    // no MRO to resolve and no tag to read -- so such a spelling exports no
    // instance method and publishes no type object. Unreachable from a
    // program `pycc check` accepts: `pycc_hir::class` records a definition
    // for every class it lowers a method of.
    let hir = module(vec![
        func("Grid.tag.static", &[], Ty::Int),
        func("Grid.area", &[("self", inst("Grid"))], Ty::Int),
    ]);
    let exports = collect_exports(&hir).expect("a carriable program");
    assert_eq!(
        exports.iter().map(|e| e.name.as_str()).collect::<Vec<_>>(),
        vec!["Grid.tag.static"]
    );
    assert!(collect_class_publications(&hir, &exports).is_empty());
}

#[test]
#[should_panic(expected = "lists `Ghost` in its method resolution order")]
fn a_ghost_class_in_the_mro_fails_fast_instead_of_under_allocating_an_instance() {
    // #1145 finding 2: the MIR counterpart (`pycc_mir::class`' `mro_attrs`)
    // panics on the same input, so skipping the entry here would allocate an
    // instance too small for the slots an inherited compiled method indexes.
    let mut hir = constructible_module("Grid");
    hir.class_defs[0].1.mro = vec!["Grid".to_string(), "Ghost".to_string()];
    let _ = class_constructible(&hir, "Grid");
}
