//! Generated-text tests for the `--ext` build seam: the exact C the driver
//! writes into the exports companion, and the properties of the tracked shim
//! the generated wrappers depend on by name.
//!
//! Split out of `ext_build_tests.rs` by #1049 under `AGENTS.md`'s
//! decomposability rule; the tests themselves are unchanged except for the
//! `str` coverage #1049 added.

use super::*;

/// [`generate_exports_inc`] for a program that declares no user exception
/// class -- every case in this file that predates #1066, and the one that
/// asserts both generated class functions are still emitted with empty
/// bodies so an artifact with no such class still links.
pub(super) fn inc_no_classes(module_name: &str, exports: &[ExtExport]) -> String {
    generate_exports_inc(
        module_name,
        exports,
        &[],
        &flat_publications(exports),
        &[],
        &[],
    )
}

/// The publication list of a program whose classes inherit nothing: one
/// entry per exporting class, in first-export order, carrying exactly that
/// class's own exports.
///
/// The inheritance-resolving walk is [`collect_class_publications`]', and it
/// needs a `HirModule` to see an MRO at all. Every case here hands
/// [`generate_exports_inc`] a hand-built export list instead, so this helper
/// states the base-less shape those cases mean, and the resolver's own
/// behaviour is pinned separately in `exports.rs` against real lowered HIR.
fn flat_publications(exports: &[ExtExport]) -> Vec<ExtPublishedClass> {
    let mut published: Vec<ExtPublishedClass> = Vec::new();
    for export in exports {
        let Some(class) = &export.class else {
            continue;
        };
        match published.iter_mut().find(|held| held.class == *class) {
            Some(held) => held.methods.push(export.clone()),
            None => published.push(ExtPublishedClass {
                class: class.clone(),
                methods: vec![export.clone()],
                mro: vec![class.clone()],
                constants: Vec::new(),
                bases: None,
                defaults: Vec::new(),
            }),
        }
    }
    published
}

/// One export taking `count` `memoryview` parameters and returning
/// `return_ty`, as generated C.
fn memoryview_inc(name: &str, count: usize, return_ty: Ty) -> String {
    inc_no_classes(
        "m",
        &[ExtExport {
            defaults: Vec::new(),
            name: name.to_string(),
            class: None,
            method: None,
            returns_buffer_slice: false,
            receiver: ExtReceiver::None,
            params: vec![Ty::MemoryView; count],
            param_writable: vec![false; count],
            return_ty,
            keyword_names: None,
        }],
    )
}

// --- #1145: the constructible type object and its `tp_init` --------------

/// An exported instance method. `params` is the receiver-free tail, exactly
/// as `collect_exports` hands it over.
fn instance_export(class: &str, method: &str, params: Vec<Ty>, return_ty: Ty) -> ExtExport {
    ExtExport {
        defaults: Vec::new(),
        name: format!("{class}.{method}"),
        class: Some(class.to_string()),
        method: Some(method.to_string()),
        returns_buffer_slice: false,
        receiver: ExtReceiver::SelfInstance,
        param_writable: vec![false; params.len()],
        params,
        return_ty,
        keyword_names: None,
    }
}

// Topic submodules split out of this file.
mod exception_classes;
mod wrappers;
mod shim_contract;
mod tuples;
mod memoryview_params;
mod class_methods;
mod carriers_and_buffers;

// --- #1143: the per-class type object and its method table --------------

/// An exported `@staticmethod`, receiver-free.
pub(super) fn static_export(
    class: &str,
    method: &str,
    params: Vec<Ty>,
    return_ty: Ty,
) -> ExtExport {
    ExtExport {
        defaults: Vec::new(),
        name: format!("{class}.{method}.static"),
        class: Some(class.to_string()),
        method: Some(method.to_string()),
        returns_buffer_slice: false,
        receiver: ExtReceiver::None,
        param_writable: vec![false; params.len()],
        params,
        return_ty,
        keyword_names: None,
    }
}
