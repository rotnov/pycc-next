//! The build-side half of `copy.copy` on an `--ext` carrier (#1455): the
//! per-class table the shim's `pycc_ext_instance_copy` consults before it
//! clones a compiled instance.
//!
//! **Why a table.** Every carrier type -- published, or created on demand
//! by `pycc_ext_carrier_type` -- carries the one shared `__copy__`, so the
//! shim learns *which* class it is copying only from the instance's layout
//! descriptor at run time. The slot words carry no kind either, so the
//! build hands the shim, per class, one kind byte per slot in
//! `pycc_hir::flat_attr_layout` order (the order `collect_getsets` and the
//! checked slot accessor use): `s` for `str`, `i` for `int`, `o` for an
//! opaque CPython object, `w` for a plain word. `pycc_rt_ext_instance_copy`
//! (`crates/pycc_rt/src/instance/copy.rs`) documents what each kind does.
//!
//! **Refusals.** A class is never copied slot-wise when CPython would run
//! user code for its copy: any class on its MRO binding one of
//! [`COPY_PROTOCOL_DUNDERS`] is refused, naming the dunder. A PEP 695
//! `class G[T]` is refused because its specializations share the template's
//! layout name, so a run-time lookup cannot tell `G[int]` from `G[str]`; the
//! `0gen_` specializations get no row at all, since their name never appears
//! in a layout descriptor. A slot whose type has no kind fails closed.
//! `docs/RUNTIME.md` ("Copying an instance through `copy.copy`") and the
//! D-244 #1455 amendment own the contract.

use pycc_hir::{HirClassDef, HirModule, Ty, flat_attr_layout};

use super::ExtCarrierClass;
use super::publication::class_member_names;

/// The exact C declaration of the generated kind lookup, which the shim's
/// `pycc_ext_instance_copy` calls. The shim is defined below the point the
/// companion is included at, so it needs no forward declaration; the shim
/// test pins the call text, and the C compiler checks it against this one
/// spelling when the artifact builds.
pub(crate) const CARRIER_CLASS_COPY_KINDS_DECL: &str = "static int \
     pycc_ext_carrier_class_copy_kinds(const unsigned char *cls, size_t len, \
     const char **kinds, size_t *nkinds, const char **refused)";

/// The names that put a class's copy in the user's hands. CPython's
/// `copy.copy` calls `__copy__` directly and reaches the others through
/// `object.__reduce_ex__`. `__deepcopy__` is deliberately absent: a shallow
/// copy never consults it.
pub(crate) const COPY_PROTOCOL_DUNDERS: [&str; 7] = [
    "__copy__",
    "__reduce__",
    "__reduce_ex__",
    "__getstate__",
    "__setstate__",
    "__getnewargs__",
    "__getnewargs_ex__",
];

/// How the shim copies one carrier class.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum CarrierCopy {
    /// Copyable: one kind byte per slot, in flat layout order.
    Kinds(String),
    /// Refused with a `TypeError`.
    Refused(CopyRefusal),
}

/// Why a class's `copy.copy` is refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum CopyRefusal {
    /// A class on the MRO binds this copy-protocol name.
    Dunder(&'static str),
    /// A slot's declared type has no copy kind.
    UncopyableSlot,
    /// A PEP 695 template, whose specializations share its layout name.
    GenericLayout,
}

impl CopyRefusal {
    /// The clause after `cannot copy '<mod>.<Class>' object: `.
    pub(crate) fn reason(&self) -> String {
        match self {
            CopyRefusal::Dunder(name) => format!("its compiled {name} is not published"),
            CopyRefusal::UncopyableSlot => "a slot has an uncopyable kind".to_string(),
            CopyRefusal::GenericLayout => {
                "its generic specializations share one instance layout".to_string()
            }
        }
    }
}

/// The kind byte of a slot declared as `ty`, or `None` for a type the
/// clone does not know how to own (the class is then refused).
pub(crate) fn copy_kind(ty: &Ty) -> Option<u8> {
    match ty {
        Ty::Str => Some(b's'),
        Ty::Int => Some(b'i'),
        Ty::Object => Some(b'o'),
        Ty::Float | Ty::Bool | Ty::List(_) | Ty::Dict(_) | Ty::Instance(_) => Some(b'w'),
        _ => None,
    }
}

/// How `def`, a carrier class of `module`, is copied. MRO entries the
/// module does not define (`object`, `Generic`) bind nothing and hold no
/// slot, so they are skipped.
pub(crate) fn carrier_copy(module: &HirModule, def: &HirClassDef) -> CarrierCopy {
    if def.type_param.is_some() {
        return CarrierCopy::Refused(CopyRefusal::GenericLayout);
    }
    let mro_defs: Vec<&HirClassDef> = def
        .mro
        .iter()
        .filter_map(|entry| {
            module
                .class_defs
                .iter()
                .find(|(held, _)| held == entry)
                .map(|(_, held)| held)
        })
        .collect();
    for mro_def in &mro_defs {
        let bound = class_member_names(mro_def)
            .find_map(|name| COPY_PROTOCOL_DUNDERS.into_iter().find(|d| *d == name));
        if let Some(dunder) = bound {
            return CarrierCopy::Refused(CopyRefusal::Dunder(dunder));
        }
    }
    let kinds: Option<Vec<u8>> = flat_attr_layout(&mro_defs)
        .iter()
        .map(|(_, ty)| copy_kind(ty))
        .collect();
    match kinds {
        Some(kinds) => CarrierCopy::Kinds(String::from_utf8(kinds).expect("kind bytes are ASCII")),
        None => CarrierCopy::Refused(CopyRefusal::UncopyableSlot),
    }
}

/// The generated kind lookup, declared as [`CARRIER_CLASS_COPY_KINDS_DECL`]:
/// for the class named `cls`/`len` (a carrier's run-time class, read from
/// its layout descriptor, not NUL-terminated) it answers `1` with the kind
/// string, `0` with the refusal clause, or `-1` for a class the table does
/// not hold, which the shim turns into a `SystemError`. Emitted
/// unconditionally, so every artifact links.
pub(crate) fn carrier_class_copy_kinds_c(classes: &[ExtCarrierClass]) -> String {
    let mut out = format!("{CARRIER_CLASS_COPY_KINDS_DECL}\n{{\n");
    let rows: Vec<&ExtCarrierClass> = classes
        .iter()
        .filter(|carrier| !carrier.class.starts_with("0gen_"))
        .collect();
    if rows.is_empty() {
        out.push_str(
            "    (void)cls;\n    (void)len;\n    (void)kinds;\n    (void)nkinds;\n    \
             (void)refused;\n",
        );
    }
    for carrier in rows {
        let class = &carrier.class;
        let body = match &carrier.copy {
            CarrierCopy::Kinds(kinds) => format!(
                "        *kinds = \"{kinds}\";\n        *nkinds = {n};\n        return 1;\n",
                n = kinds.len()
            ),
            CarrierCopy::Refused(refusal) => format!(
                "        *refused = \"{reason}\";\n        return 0;\n",
                reason = refusal.reason()
            ),
        };
        out.push_str(&format!(
            "    if (len == {len} && memcmp(cls, \"{class}\", {len}) == 0) {{\n{body}    }}\n",
            len = class.len(),
        ));
    }
    out.push_str("    return -1;\n}\n\n");
    out
}
