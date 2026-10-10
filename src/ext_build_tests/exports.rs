//! Export-set and capability-gap tests for the `--ext` build seam: which
//! public module-level functions `collect_exports` admits, and the `C0003`
//! diagnostic it raises for every signature the boundary cannot carry.
//!
//! Split out of `ext_build_tests.rs` by #1049 under `AGENTS.md`'s
//! decomposability rule; the tests themselves are unchanged except where
//! #1049's own widening required re-pointing a fixture.

use super::*;

// --- #1145: instance methods, and the constructibility predicate ---------

/// The module every constructibility test below varies one field of: a
/// public class with a public instance method and a carriable `__init__`.
pub(super) fn constructible_module(class: &str) -> HirModule {
    let mut hir = module_with_classes(
        vec![
            init_func(class, &[("w", Ty::Int), ("h", Ty::Int)], Ty::None),
            func(
                &format!("{class}.area"),
                &[("self", Ty::Instance(Box::new(class.to_string())))],
                Ty::Int,
            ),
        ],
        vec![(class.to_string(), constructible_class_def(class))],
    );
    declare_member(&mut hir, class, Member::Method("area"));
    hir
}

/// Which of [`pycc_hir::HirClassDef`]'s member tables a fixture's binding
/// belongs in. `collect_class_publications` resolves a name against those
/// tables, so a fixture that pushes a `<Class>.<method>` item without the
/// matching table entry describes a class `pycc_hir::class` could never
/// have lowered -- and would exercise the namespace walk against a class
/// that binds nothing.
enum Member<'a> {
    Method(&'a str),
    Property(&'a str),
    Static(&'a str),
}

/// Record `member` in `class`'s own table, so the class table says what the
/// fixture's [`HirItem`]s already say.
fn declare_member(hir: &mut HirModule, class: &str, member: Member<'_>) {
    let (_, def) = hir
        .class_defs
        .iter_mut()
        .find(|(held, _)| held == class)
        .expect("the fixture defines the class it declares a member on");
    match member {
        Member::Method(name) => def
            .methods
            .push((name.to_string(), format!("{class}.{name}"))),
        Member::Property(name) => def.properties.push(pycc_hir::PropertyDef {
            name: name.to_string(),
            getter: format!("{class}.{name}"),
            setter: None,
        }),
        Member::Static(name) => def
            .static_methods
            .push((name.to_string(), format!("{class}.{name}.static"))),
    }
}

// --- #1145 finding 1: MRO-resolved publication ---------------------------

/// `Ty::Instance` for a receiver parameter, spelled once.
fn inst(class: &str) -> Ty {
    Ty::Instance(Box::new(class.to_string()))
}

/// `Base`, with one instance method, plus `Derived(Base)` with one of its
/// own -- both constructible, wired the way `pycc_hir::class` wires an
/// inheriting program: `Derived`'s `mro` is `[Derived, Base]` and `Base`'s
/// is `[Base]`, most derived first.
fn inheriting_module() -> HirModule {
    let mut hir = constructible_module("Base");
    hir.items[1] = func("Base.value", &[("self", inst("Base"))], Ty::Int);
    hir.class_defs[0]
        .1
        .methods
        .retain(|(name, _)| name != "area");
    declare_member(&mut hir, "Base", Member::Method("value"));
    let mut derived = constructible_class_def("Derived");
    derived.mro = vec!["Derived".to_string(), "Base".to_string()];
    hir.class_defs.push(("Derived".to_string(), derived));
    hir.items.push(init_func(
        "Derived",
        &[("w", Ty::Int), ("h", Ty::Int)],
        Ty::None,
    ));
    hir.items
        .push(func("Derived.twice", &[("self", inst("Derived"))], Ty::Int));
    declare_member(&mut hir, "Derived", Member::Method("twice"));
    hir
}

/// Each published class paired with the *compiled* names of the methods its
/// type object carries -- the compiled name rather than the host-visible one
/// because that is what says which definition won an override.
fn publication_rows(publications: &[ExtPublishedClass]) -> Vec<(&str, Vec<&str>)> {
    publications
        .iter()
        .map(|published| {
            (
                published.class.as_str(),
                published
                    .methods
                    .iter()
                    .map(|export| export.name.as_str())
                    .collect(),
            )
        })
        .collect()
}

fn publications_of(hir: &HirModule) -> Vec<ExtPublishedClass> {
    let exports = collect_exports(hir).expect("a carriable program");
    collect_class_publications(hir, &exports)
}

// Topic submodules split out of this file.
mod functions;
mod methods;
mod inheritance;
mod constructors;
mod shadowing;
mod writability_and_getsets;
