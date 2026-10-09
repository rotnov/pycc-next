//! The `--ext` driver's view of receiver-exact inherited-method copies
//! (#1337, D-254).
//!
//! `pycc_types` compiles an inherited body whose behaviour depends on its
//! receiver once more for each subclass that needs it, as an ordinary item
//! spelled like the subclass's own definition (`B.g` for `A.g` compiled for
//! `B`; see `pycc_hir::inherited_copy_name`). The export walk sees those
//! items, but it resolves a published class's members through the class
//! tables, where a copy never appears -- so without this module a host
//! calling `mod.B().g()` would still run `A.g`'s compilation for `A`, the
//! exact miscompile the copies exist to remove.
//!
//! Three questions are answered here, each the ext-side twin of a
//! `pycc_mir` routing site (plus [`source_item`], which names the source a
//! copy inherits its default values from):
//!
//! - which exported item a published class's method table binds for a
//!   member an ancestor owns ([`receiver_exact_export`]);
//! - which compiled `__init__` a host-side construction runs
//!   ([`receiver_exact_init`]), and which compiled `@property` getter a
//!   host-side attribute read runs ([`receiver_exact_member`], #1442);
//! - whether an exported copy is itself publishable ([`copy_export_verdict`]):
//!   a copy is compiled for exactly one receiver class, so it is reachable
//!   only on that class, and a copied `@property` getter is attribute
//!   syntax on the host exactly as an own one is.

use pycc_hir::{HirClassDef, HirItem, HirModule, InheritedCopy, inherited_copy_origin};

use super::{ExtExport, class_constructible, class_publishable, instance_shape_admissible};

fn class_of<'m>(module: &'m HirModule) -> impl Fn(&str) -> Option<&'m HirClassDef> {
    |name| {
        module
            .class_defs
            .iter()
            .find(|(held, _)| held == name)
            .map(|(_, def)| def)
    }
}

/// The copy an item name denotes, when `name` is one.
fn copy_of(module: &HirModule, name: &str) -> Option<InheritedCopy> {
    inherited_copy_origin(name, &class_of(module))
}

/// The item whose source the compiled item `name` was lowered from: a
/// copy's origin, else `name` itself. What a copy inherits from that source
/// beyond its body -- its default parameter values (the method part of
/// #1140) -- is looked up under this name.
pub(crate) fn source_item(module: &HirModule, name: &str) -> String {
    copy_of(module, name).map_or_else(|| name.to_string(), |copy| copy.origin_name)
}

/// What [`copy_export_verdict`] decides for one exported item.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CopyVerdict {
    /// Not a copy: the ordinary export rules apply unchanged.
    NotACopy,
    /// A copy that is published on its receiver class.
    Publish,
    /// A copy nothing may publish.
    Drop,
}

/// Whether the exported item `name` -- a bare `<Class>.<method>` instance
/// spelling -- is a receiver-exact copy, and if so whether it is
/// published.
///
/// A copy of a `@property` getter is dropped exactly as an own getter is
/// (a property is attribute syntax on the host side). Any other instance
/// copy is compiled for its receiver class alone -- a subclass of that
/// class resolves the member to its own copy or to the origin -- so it is
/// published only when that class itself is published and constructible,
/// the same conditions [`super::instance_method_reachable`] asks of the
/// class an ordinary export is reached through.
pub(crate) fn copy_export_verdict(module: &HirModule, name: &str) -> CopyVerdict {
    let Some(copy) = copy_of(module, name) else {
        return CopyVerdict::NotACopy;
    };
    let of = class_of(module);
    let origin = of(&copy.origin_class).expect("a copy's origin class exists");
    if origin
        .properties
        .iter()
        .any(|prop| prop.getter == copy.origin_name)
    {
        return CopyVerdict::Drop;
    }
    let receiver = of(&copy.receiver).expect("a copy's receiver class exists");
    if instance_shape_admissible(receiver, &copy.receiver)
        && class_publishable(receiver, &copy.receiver)
        && class_constructible(module, &copy.receiver)
    {
        CopyVerdict::Publish
    } else {
        CopyVerdict::Drop
    }
}

/// The export `class`'s method table binds for `inherited`, an export owned
/// by one of `class`'s ancestors: the copy compiled for `class` when one
/// was exported, else `inherited` itself.
pub(crate) fn receiver_exact_export<'e>(
    module: &HirModule,
    class: &str,
    inherited: &'e ExtExport,
    exports: &'e [ExtExport],
) -> &'e ExtExport {
    exports
        .iter()
        .find(|export| {
            export.class.as_deref() == Some(class)
                && export.receiver == inherited.receiver
                && copy_of(module, &export.name)
                    .is_some_and(|copy| copy.origin_name == inherited.name)
        })
        .unwrap_or(inherited)
}

/// The compiled item a host-side `mod.<class>(...)` runs, given the
/// MRO-resolved `__init__` item `mangled`: the copy compiled for `class`
/// when the constructor is inherited and needed one, else `mangled`.
pub(crate) fn receiver_exact_init<'m>(
    module: &'m HirModule,
    class: &str,
    mangled: &'m str,
) -> &'m str {
    receiver_exact_member(module, class, "__init__", mangled)
}

/// The compiled item a host-side read of `class`'s member `member` runs,
/// given the MRO-resolved item `mangled` that owns it: the copy compiled for
/// `class` when the member is inherited and needed one, else `mangled`.
/// [`receiver_exact_init`] is this question for `__init__`; a `@property`
/// getter published as a getset descriptor (#1442) asks it for its own name.
pub(crate) fn receiver_exact_member<'m>(
    module: &'m HirModule,
    class: &str,
    member: &str,
    mangled: &'m str,
) -> &'m str {
    let copy = format!("{class}.{member}");
    if copy == mangled {
        return mangled;
    }
    module
        .items
        .iter()
        .find_map(|item| match item {
            HirItem::Function { name, .. }
                if *name == copy
                    && copy_of(module, name).is_some_and(|c| c.origin_name == mangled) =>
            {
                Some(name.as_str())
            }
            _ => None,
        })
        .unwrap_or(mangled)
}

#[cfg(test)]
#[path = "inherited_tests.rs"]
mod tests;
