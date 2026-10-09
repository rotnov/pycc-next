//! Which compiled classes the `--ext` artifact publishes a type object
//! for, and the MRO-resolved method set each one carries (#1145, #1146,
//! #1450).
//!
//! Split out of `src/ext_build.rs` (`AGENTS.md`'s "Keep source files
//! decomposable") when #1450 widened the publication predicate: the
//! namespace walk, its member-kind table and the slot test move together
//! because [`collect_class_publications`] is their only caller apart from
//! the export predicate's per-method witness check
//! (`super::instance_method_reachable`, through [`namespace_owner`]).

use super::{ExtExport, class_constructible, class_publishable, inherited};
use pycc_hir::{HirClassDef, HirModule, ProtocolMember};

/// One published class: the host-visible type object's name and the exact
/// `PyMethodDef` rows it carries.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ExtPublishedClass {
    /// The class's own (unqualified) Python name.
    pub(crate) class: String,
    /// The methods published on this class's type object, MRO-resolved:
    /// every exported member of the class and of its bases, first MRO hit
    /// winning, in the order [`collect_class_publications`] resolves them.
    /// Each entry is an [`ExtExport`] the export set already holds, so every
    /// row names a `pycc_ext_wrap_` that [`generate_exports_inc_with_slots`] really
    /// emits.
    ///
    /// [`generate_exports_inc_with_slots`]: super::generate_exports_inc_with_slots
    pub(crate) methods: Vec<ExtExport>,
    /// The class's MRO, most derived first (`HirClassDef::mro`): the
    /// classes an object `isinstance` against which this class's type
    /// object answers (Part 7 of #1371, `compiled_class_isinstance_c`).
    pub(crate) mro: Vec<String>,
}

/// The published classes and, for each, its MRO-resolved method set.
///
/// **MRO-resolved, not own-declared.** A method is lowered once against its
/// own class's slot layout and inherited unchanged, so `mod.Derived(21)`
/// must answer `value()` as well as `twice()`. Resolving the set here is
/// what publishes it: the walk is `class_def.mro`, most derived first --
/// the same order [`resolved_init`] walks for `__init__`.
///
/// **The walk resolves the namespace, not the export set.** This is the
/// canonical statement of that rule: a name any `__init__` along the MRO
/// assigns to `self` is answered by the instance and belongs to no class
/// at all ([`mro_binds_slot`]); every other name is answered by the
/// *first* MRO entry that binds it in the class namespace --
/// [`class_member_names`] is what "binds" means -- and that entry alone
/// decides the outcome. If its binding is an
/// export, the method is published; if it is anything the export set does
/// not hold (a `@property` getter, an `@abstractmethod`'s stub, a private
/// or uncarriable member), the name is simply absent from the published
/// class, and the walk never falls through to a base that happens to
/// export the same name. Stopping at the first *exportable* hit instead
/// would publish `Base.value`'s compiled body on a `Derived` whose own
/// `@property value` shadows it -- an artifact that silently disagrees
/// with Python's own attribute lookup (#1146). The rule is kind-blind in
/// both directions, so a derived ordinary method still shadows a base
/// `@property`, and a derived `@staticmethod` still shadows a base
/// instance method, each published under its own receiver kind.
///
/// That direction is the opposite of [`collect_exports_with_hooks`]' `(class, method)`
/// dedup, which keeps the *last* binding because a rebound name is what
/// `Grid.f` means in one class body.
///
/// **Why an inherited method may be published at all.** A base method's
/// compiled body addresses its own class's slot indices, and
/// `crates/pycc_hir/src/class/mro.rs`'s `validate_mro_slot_layout` (#969)
/// rejects with
/// `C0001`, during HIR lowering, every multiple-inheritance shape whose
/// ancestor layout is not a name-wise prefix of the derived one -- see that
/// function's own doc for why. So a base method invoked with a derived
/// instance addresses the same attributes, and this path inherits that
/// invariant rather than restating it.
///
/// **Which classes get a type object.** A publishable class (below) is
/// published when its MRO-resolved set is non-empty *or* it is
/// constructible ([`class_constructible`], D-244 #1145 clause (b)), so an
/// `__init__`-only class -- lark's `ParseConf` (#1450), or a fields-only
/// dataclass, whose generated `__init__` is its only member -- and
/// a class with only the implicit `object.__init__` (`class Empty: pass`)
/// are published with no exported method: the host can name
/// them, construct them, and pass the instance to a compiled function. A
/// publishable class that resolves nothing and is not constructible --
/// an `__init__` with an uncarriable parameter, an enum, a Protocol -- still
/// gets no type object, since there is nothing the host could do with one.
/// Neither does a monomorphized generic specialization (`0gen_<Class>__...`)
/// that resolves nothing: it has no `fnptr_` global for a generated
/// `tp_init` to call, and its instances cross as a carrier named after the
/// generic class.
///
/// The class list is the export set's classes in first-export order, then
/// every remaining published class, in `HirModule::class_defs`
/// order: a class with no export of its own has no first-export position,
/// and appending is the only deterministic slot for it. Deterministic is the
/// requirement -- the generated `.inc` must be byte-identical across runs,
/// so neither list is ever built from a hash map.
///
/// A class is publishable when [`is_public_name`] accepts its name and it is
/// not an exception class: [`register_class_c`] already publishes a user
/// exception class under its bare name, and a second `PyModule_AddObjectRef`
/// under that name would replace it. Both are already true of every class in
/// the export set -- [`classify_export_name`] and [`collect_exports_with_hooks`]'
/// exception filter see to that -- so the test bites only on an inheriting
/// class that exports nothing itself. Abstractness is deliberately *not*
/// tested: Part 1 published a `@staticmethod` on an abstract class, and an
/// abstract class exports no instance method to begin with
/// ([`instance_method_reachable`]).
///
/// [`resolved_init`]: super::resolved_init
/// [`class_constructible`]: super::class_constructible
/// [`collect_exports_with_hooks`]: super::collect_exports_with_hooks
/// [`is_public_name`]: pycc_hir::is_public_name
/// [`register_class_c`]: super::register_class_c
/// [`classify_export_name`]: super::classify_export_name
/// [`instance_method_reachable`]: super::instance_method_reachable
pub(crate) fn collect_class_publications(
    module: &HirModule,
    exports: &[ExtExport],
) -> Vec<ExtPublishedClass> {
    let mut order: Vec<&str> = Vec::new();
    for export in exports {
        if let Some(class) = &export.class
            && !order.contains(&class.as_str())
        {
            order.push(class.as_str());
        }
    }
    for (class, _) in &module.class_defs {
        if !order.contains(&class.as_str()) {
            order.push(class.as_str());
        }
    }
    let mut published: Vec<ExtPublishedClass> = Vec::new();
    for class in order {
        let Some((_, class_def)) = module.class_defs.iter().find(|(held, _)| held == class) else {
            continue;
        };
        if !class_publishable(class_def, class) {
            continue;
        }
        let mut methods: Vec<ExtExport> = Vec::new();
        for ancestor in &class_def.mro {
            // [`ExtExport::method`] is `Some` exactly when
            // [`ExtExport::class`] is, so the `?` rejects only the
            // module-level functions this filter drops anyway.
            for (export, method) in exports.iter().filter_map(|export| {
                let method = export.method.as_deref()?;
                (export.class.as_deref() == Some(ancestor.as_str())).then_some((export, method))
            }) {
                // No dedup pass is needed beside this test: exactly one MRO
                // entry owns a given name, and `collect_exports`' own
                // `(class, method)` dedup leaves that entry at most one
                // export under it.
                if namespace_owner(module, &class_def.mro, method) == Some(ancestor.as_str()) {
                    // #1337 (D-254): an inherited member binds the copy
                    // compiled for this class when one exists.
                    methods.push(
                        inherited::receiver_exact_export(module, class, export, exports).clone(),
                    );
                }
            }
        }
        // #1450: a constructible class is published even when it resolves
        // no method -- lark's `__init__`-only `ParseConf` -- so a host can
        // name it, build it and hand the result to a compiled function. A
        // monomorphized specialization (`0gen_`) is never published on that
        // ground: its `__init__` has no `fnptr_` global for a `tp_init` to
        // call (codegen dispatches specializations directly).
        let constructible = !class.starts_with("0gen_") && class_constructible(module, class);
        if !methods.is_empty() || constructible {
            published.push(ExtPublishedClass {
                class: class.to_string(),
                methods,
                mro: class_def.mro.clone(),
            });
        }
    }
    published
}

/// Every name `class_def`'s own body binds **in the class namespace**, in
/// a deterministic order, whatever kind of member binds it.
///
/// This is the canonical statement of "does this class define this name"
/// for [`collect_class_publications`]' namespace walk. The kinds are read
/// off the HIR class table rather than recognized by a name pattern:
/// `methods` (an ordinary method, a `@dataclass`-generated one, and an
/// `@abstractmethod`, which `crates/pycc_hir/src/class/body.rs` enters
/// there *and* into `abstract_methods` -- so the latter adds nothing here),
/// `properties` by [`pycc_hir::PropertyDef::name`] (one entry covers a
/// getter and its optional setter: that file refuses a `@<name>.setter`
/// without a preceding `@property` getter, so a setter never binds a name
/// on its own), `static_methods`, `class_methods`, and `class_attrs` -- a
/// `ClassVar` or bare class-level assignment, whose constant is an
/// ordinary entry in the class object's namespace.
///
/// **`attrs` is deliberately not one of them.** An instance-attribute slot
/// is bound on the instance, not on the type, so it is not a namespace
/// entry and has no position in the MRO walk at all; [`mro_binds_slot`]
/// is where it is seen instead.
///
/// `class_attrs` is here for a reason the sibling predicate
/// `crates/pycc_hir/src/class/shadow.rs`'s `declares_name_outside_class_attrs`
/// documents from the other side: that one answers a *class-name-qualified*
/// read (`Derived.LIMIT`) and excludes `class_attrs` because its callers
/// check them separately, while this walk answers an *instance* read and
/// must treat a class attribute as the ordinary namespace entry it is.
/// The two lists differ a second way, which is not an oversight: that
/// predicate also carries `enum_members`, which cannot appear on the MRO
/// this walk is given, because an enum is a terminal leaf --
/// `validate_bases` refuses to extend one with a `C0001` (#941).
///
/// The `ProtocolMember::Method` half of `protocol_members` *is* carried
/// here, for the reason that predicate states: a `Protocol` class's
/// declaration-style `def f(self) -> int: ...` is a real function object in
/// its namespace, and CPython resolves it like any other. A protocol base
/// does reach this walk -- `crates/pycc_hir/src/class.rs` propagates
/// `is_protocol` to an inheritor and gives it the base's
/// `protocol_members`, but such a class is still published, so for
/// `class Q(P, A)` with `P` declaring `f` and `A` exporting a
/// `@staticmethod f`, `P`'s binding is the one CPython answers and `A`'s
/// export must not be published under that name. The binding is not itself
/// exportable, so the name is published by no one -- lossy in the
/// direction this walk is always willing to be wrong in, where resolving
/// the export set instead published a callable Python does not give.
/// Nothing rejects the shape that makes the difference visible:
/// `crates/pycc_hir/src/class/attrs.rs`'s `reject_class_attr_collisions`
/// checks a class's *own* newly declared `class_attrs` against its own MRO
/// and never runs for a class that declares none, so two independent bases
/// -- one binding `f` as a method, the other as a class attribute -- are
/// combined without complaint by a third class that declares neither.
///
/// Slices are walked in table order and never through a hash map: the
/// generated `.inc` must be byte-identical across runs.
pub(super) fn class_member_names(class_def: &HirClassDef) -> impl Iterator<Item = &str> {
    class_def
        .methods
        .iter()
        .map(|(name, _)| name.as_str())
        .chain(class_def.properties.iter().map(|prop| prop.name.as_str()))
        .chain(
            class_def
                .static_methods
                .iter()
                .map(|(name, _)| name.as_str()),
        )
        .chain(
            class_def
                .class_methods
                .iter()
                .map(|(name, _)| name.as_str()),
        )
        .chain(
            class_def
                .class_attrs
                .iter()
                .map(|(name, _, _)| name.as_str()),
        )
        .chain(class_def.protocol_members.iter().filter_map(|member| {
            match member {
                ProtocolMember::Method { name, .. } => Some(name.as_str()),
                // An annotation-only protocol attribute declares a type,
                // not a binding: `x: int` in a class body leaves the class
                // namespace without an `x`, exactly as it does anywhere
                // else, so it shadows nothing.
                ProtocolMember::Attribute { .. } => None,
            }
        }))
}

/// Whether any class linearized in `mro` assigns `name` to `self` in its
/// `__init__` -- that is, declares it as an instance-attribute slot.
///
/// **Position in the walk is irrelevant, which is the whole point.** An
/// instance slot is not a namespace binding that competes with the class
/// namespace at its own MRO index. CPython consults the instance
/// `__dict__` *before* the type's namespace for everything that is not a
/// data descriptor, so a slot contributed by the *least* derived base
/// still wins over a method defined on the most derived class. Modelling a
/// slot as one more kind inside [`class_member_names`] would answer only
/// the cases where the slot's own class happens to precede the method's
/// (#1146), and would silently publish a callable for
/// `class Base: def __init__(self, n): self.value = n` combined with
/// `class Derived(Base): def value(self): ...`, where CPython answers the
/// integer and raises `TypeError: 'int' object is not callable`.
///
/// **What this predicate actually tests is broader than that, on purpose.**
/// CPython's precedence is per instance and per construction: a name is in
/// the instance `__dict__` only once an `__init__` that assigns it has
/// run. This predicate asks a static question instead -- does any class
/// linearized in `mro` declare the slot at all -- because a compiled
/// instance has no `__dict__`. Every ancestor layout is a name-wise
/// prefix of the derived one: in a single-inheritance chain by
/// construction, since `crates/pycc_hir/src/class/mro.rs`'s
/// `flat_attr_layout` assigns slots most-base-first, and under multiple
/// inheritance because `validate_mro_slot_layout` (#969) rejects every
/// shape where that would not hold, a single base onto an
/// already-validated ancestor inheriting the property transitively. A
/// slot declared anywhere on the MRO
/// therefore occupies a fixed offset in every subclass whether or not the
/// `__init__` that assigns it is the one a given construction reaches.
/// The two conditions differ exactly where an override's `__init__` skips
/// its base's: for `class Base: def __init__(self): self.value = 5` with a
/// `class Derived(Base)` whose `__init__` calls no `super()` and which
/// declares `def value`, CPython answers `Derived().value()` with `99`
/// while the artifact publishes nothing. That is the direction this walk
/// is willing to be wrong in -- lossy, never a callable Python would not
/// give -- and publishing on a guess is what it exists to avoid.
///
/// The same conservatism covers the one class-namespace kind that beats an
/// instance slot, a `@property`: it is a data descriptor, so it would win
/// the name, and suppressing it anyway costs at most a getter+setter
/// property whose class also assigns the name in `__init__`. A read-only
/// property is not that case -- `crates/pycc_types/src/class.rs`'s
/// `check_attr_set` rejects `self.<name> = ...` against it with a `T0044`
/// before such a class compiles at all -- so nothing is lost there.
fn mro_binds_slot(module: &HirModule, mro: &[String], name: &str) -> bool {
    mro.iter().any(|ancestor| {
        module.class_defs.iter().any(|(held, def)| {
            held == ancestor && def.attrs.iter().any(|(held_name, _)| held_name == name)
        })
    })
}

/// The MRO entry that answers `method` for a class linearized as `mro`:
/// the first entry, most derived first, that binds the name in the class
/// namespace ([`class_member_names`]), or `None` when nothing publishable
/// answers it.
///
/// Python's own attribute lookup, stated as a mechanism rather than as a
/// list of kinds. Two rules, in this order:
///
/// 1. A name any `__init__` along the MRO assigns to `self` is answered by
///    the instance, never by the type, so no class owns it and the result
///    is `None` ([`mro_binds_slot`]).
/// 2. Otherwise the first MRO entry binding the name in the class
///    namespace owns it, kind-blind: that entry decides the outcome
///    whether it binds a regular method, a `@property`, an
///    `@abstractmethod`'s stub, a `@staticmethod`, a `@classmethod` or a
///    class attribute.
///
/// Callers ask whether the entry they hold is the winner, never whether an
/// entry further down the walk could also answer.
///
/// `None` from rule 2 alone is unreachable for a `method` some export
/// names: `pycc_hir`'s class lowering records a table entry for every
/// method it mangles, so an export's own class always binds its name. A
/// fixture that pushes a `<Class>.<method>` item without the matching
/// table entry describes a class that lowering could not have produced,
/// and is treated as binding nothing.
pub(super) fn namespace_owner<'a>(
    module: &HirModule,
    mro: &'a [String],
    method: &str,
) -> Option<&'a str> {
    if mro_binds_slot(module, mro, method) {
        return None;
    }
    mro.iter().map(String::as_str).find(|ancestor| {
        module.class_defs.iter().any(|(held, def)| {
            held == ancestor && class_member_names(def).any(|name| name == method)
        })
    })
}
