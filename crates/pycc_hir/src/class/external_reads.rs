//! Instance attributes code outside the class reads through CPython (v0.8).
//! In a non-strict `--ext` module, code holding a compiled instance as a
//! CPython object -- fallback code, a lifted function body, and in a
//! `--foreign-relative-imports` package (every module its own extension)
//! the other modules -- reaches it only through CPython's attribute
//! protocol, which sees a slot only through the published type's getset
//! descriptor (`src/ext_build/getset.rs`). A native
//! `list`, `dict`, `set` or `tuple` slot has none: it cannot be shared with
//! CPython without a copy, which would break aliasing (`state.tokens.append`
//! in another module would append to the copy). So a container slot whose
//! name such code uses as an attribute (on anything but a method's own
//! `self`) is a CPython object instead -- the container is CPython's, shared by both sides --
//! while the instance keeps its native layout and its other slots.
//!
//! The names come from the build driver (`src/fallback/package_bases.rs`),
//! which reads the module and the package's other modules; an empty set
//! (the default: strict mode, a native program) changes nothing.

use crate::Ty;
use std::collections::BTreeSet;
use std::sync::Mutex;

static READS: Mutex<BTreeSet<String>> = Mutex::new(BTreeSet::new());

/// Records the attribute names other modules of the package use (module
/// doc); the empty set forgets them.
pub fn set_external_attribute_reads(names: BTreeSet<String>) {
    *READS.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = names;
}

/// Retypes each container slot of `attrs` such code reads to `object`.
pub(super) fn widen_external_container_slots(attrs: &mut [(String, Ty)]) {
    for (name, ty) in attrs.iter_mut() {
        *ty = external_slot_ty(name, std::mem::replace(ty, Ty::Infer));
    }
}

/// `ty`, or `object` when it is a container type and code outside the
/// class uses an attribute named `name` (module doc): what a slot of
/// that name may hold.
pub fn external_slot_ty(name: &str, ty: Ty) -> Ty {
    let container = matches!(
        ty,
        Ty::List(_) | Ty::Dict(_) | Ty::Set(_) | Ty::FrozenSet(_) | Ty::Tuple(_) | Ty::VarTuple(_)
    );
    if container
        && READS
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .contains(name)
    {
        Ty::Object
    } else {
        ty
    }
}
