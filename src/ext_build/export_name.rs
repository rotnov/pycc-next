//! The export-name grammar: which mangled `HirItem::Function` name is an
//! `--ext` export, and what host-visible identity it carries.
//!
//! Extracted from `src/ext_build.rs` by #1143 under `AGENTS.md`'s
//! decomposability rule -- that file crossed the ~1,000-line threshold and
//! this pull request's own work touches it. The functions themselves are
//! unchanged by the move.

use super::ExtExport;
use pycc_hir::is_public_name;

/// The host-visible identity an export is deduplicated on: `(None, name)`
/// for a module-level function, `(Some(class), method)` for a method.
pub(crate) fn export_dedup_key(export: &ExtExport) -> (Option<&str>, &str) {
    match (&export.class, &export.method) {
        (Some(class), Some(method)) => (Some(class.as_str()), method.as_str()),
        _ => (None, export.name.as_str()),
    }
}

/// Which receiver, if any, a compiled method's signature leads with, and
/// what the generated wrapper must pass for it.
///
/// Three-state rather than the `bool` #1143 used, because "the declaration
/// leads with a `void *`" and "the call passes a real pointer" are two
/// different questions and #1145 added a member that answers them
/// differently. [`ExtReceiver::None`] declares no receiver at all;
/// [`ExtReceiver::NullCls`] declares one and passes `NULL`;
/// [`ExtReceiver::SelfInstance`] declares one and passes the carrier
/// object's inner `PyInstanceObj`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ExtReceiver {
    /// A module-level `def` or a `@staticmethod`: the compiled signature is
    /// exactly the declared parameter list.
    None,
    /// A `@classmethod`, whose compiled signature leads with an injected
    /// `cls: Ty::Instance(Class)` that never crosses the boundary --
    /// `MirExpr::NullInstance` records that every native call site passes a
    /// null pointer for it, and the wrapper does the same.
    NullCls,
    /// An instance method, whose compiled signature leads with `self:
    /// Ty::Instance(Class)` and which *does* dereference it. The wrapper
    /// unwraps the host carrier object and passes the real pointer.
    SelfInstance,
}

/// What a compiled function's name says about its place in the export set.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ExportName {
    /// A public module-level `def`. [`ExtExport::name`] is also its
    /// `PyMethodDef` `ml_name`.
    ModuleLevel,
    /// A public `@staticmethod`, `@classmethod` or instance method of a
    /// public class, whose `ml_name` is `method` inside `class`'s own
    /// table.
    Method {
        class: String,
        method: String,
        /// Which receiver the compiled signature leads with.
        receiver: ExtReceiver,
    },
}

/// The **purely lexical** half of the export verdict: what, if anything,
/// `name` may be exported as, disregarding its signature, its class's
/// exception tag and everything else the name itself does not say.
///
/// This is the mirror of `pycc_codegen::is_ext_exportable_name`, which
/// answers the same question from the other side of a deliberate
/// duplication (`pycc_codegen` does not depend on `pycc_hir`). The two are
/// pinned equal over a shared table of names by
/// `ext_build_tests::exports`' parity test: drift is silent, because
/// [`wrapper_for`] asks `pycc_codegen::ext_thunk_required` -- which calls
/// that mirror -- to choose between the thunk `extern` and the `fnptr_`
/// `extern`, so a disagreement emits the wrong C declaration for a
/// `tuple`-carrying method rather than failing the build.
///
/// The two PEP 562 module hooks (#1467) and the comparison and hash slot
/// dunders (#1427) are deliberately **not** admitted here: the mirror
/// admits `__getattr__`, `__dir__` and `<Class>.<slot dunder>` lexically,
/// the driver admits the hooks through [`super::is_module_hook`] only when
/// the entry module defines them and the dunders through
/// `super::collect_slot_dunders`, and the parity test compares the mirror
/// with this verdict `|| is_module_hook(name) || is_slot_dunder_method(name)`.
///
/// **Nothing outside `name` may enter this verdict.** The mirror receives a
/// bare `&str` and cannot see `HirModule::class_defs`, so a verdict that
/// consulted the class table would have no mirror-comparable form and the
/// parity test would stop being well-formed. `class_defs` may still be read
/// for *diagnostics*, and [`collect_exports`] layers every HIR-dependent
/// exclusion on top of this verdict -- which makes the mirror a superset of
/// the driver's admitted set, and a superset is harmless here: at worst a
/// thunk is emitted for a name no wrapper calls.
///
/// **Refused as representation, here, lexically:** a monomorphized generic
/// specialization (`0gen_`), a private class or method name (a private
/// class's slot dunder, `_Grid.__eq__`, is the mirror's one exception, per
/// the paragraph above), an empty
/// segment, a fourth segment, and `<Class>.<property>.setter` -- a
/// `@property` is attribute syntax on the host side, not a method table
/// entry. #1145 moved the *rest* of the old refusal list into the driver.
/// The bare `<Class>.<method>` spelling covers `MethodKind::Regular`,
/// `PropertyGetter` and `AbstractMethod` at once, and telling them apart
/// needs `HirModule::class_defs`, which this verdict may not see. So it is
/// admitted here and narrowed there: a property getter by
/// `HirClassDef::properties`, and an abstract stub by its class failing
/// [`collect_exports`]' constructibility predicate -- an
/// `@abstractmethod` that survives lowering always belongs to an
/// `is_abstract` class, which is never constructible.
///
/// The `0gen_` refusal is applied to the whole name before the split and
/// takes precedence. That is defense in depth rather than a live hazard:
/// `pycc_types::monomorphize` puts the substitution suffix *last*
/// (`0gen_<Class>.<method>__<P>_<C>`), so a `0gen_` name's final segment is
/// never `static` and a split-first rule would refuse it anyway.
pub(crate) fn classify_export_name(name: &str) -> Option<ExportName> {
    if name.starts_with("0gen_") {
        return None;
    }
    let mut segments = name.split('.');
    let class_or_fn = segments.next()?;
    // An empty segment is not a name `is_public_name` was written to judge:
    // `!"".starts_with('_')` is `true`, which would admit `""` and `".static"`
    // as public. Neither is a name the lowerer can emit, so both are refused
    // fail-closed here -- and the codegen mirror refuses them too, which is
    // what keeps the two predicates equal.
    if class_or_fn.is_empty() || !is_public_name(class_or_fn) {
        return None;
    }
    let Some(method) = segments.next() else {
        return Some(ExportName::ModuleLevel);
    };
    if method.is_empty() || !is_public_name(method) {
        return None;
    }
    match segments.next() {
        // `<Class>.<method>`: `MethodKind::Regular`, `PropertyGetter` or
        // `AbstractMethod`. This spelling cannot tell them apart, and since
        // #1145 it does not have to: it is admitted lexically as an
        // instance method and the two non-`Regular` kinds are removed by
        // [`collect_exports`]' driver filters, which can see the class
        // table this verdict may not.
        None => Some(ExportName::Method {
            class: class_or_fn.to_string(),
            method: method.to_string(),
            receiver: ExtReceiver::SelfInstance,
        }),
        Some(kind) => {
            // The only four-segment name is a `super()`-target copy of a
            // setter or classmethod (`C.k.0super_D.classmethod`, D-254),
            // which is never hosted: `pycc_hir` refuses a class nested in a
            // class or in a function, so no `A.B.method` name exists.
            // Refusing every fourth segment is the fail-closed reading.
            if segments.next().is_some() {
                return None;
            }
            match kind {
                "static" => Some(ExportName::Method {
                    class: class_or_fn.to_string(),
                    method: method.to_string(),
                    receiver: ExtReceiver::None,
                }),
                "classmethod" => Some(ExportName::Method {
                    class: class_or_fn.to_string(),
                    method: method.to_string(),
                    receiver: ExtReceiver::NullCls,
                }),
                // `<Class>.<property>.setter`: refused as representation,
                // for the same reason the bare spelling is. A `@property`
                // is attribute syntax on the host side, not a method.
                _ => None,
            }
        }
    }
}

/// The source-level spelling of a compiled function name: the name the user
/// actually wrote, for a message a user reads.
///
/// `Grid.scale.static` renders `Grid.scale`; `f` renders `f`. Two messages
/// need it and must not diverge: the `C0003` subject
/// ([`capability_gap`]) and the generated wrapper's arity and keyword
/// `TypeError`s (#1461), which sit on the same object CPython itself
/// describes as `Grid.scale() takes no keyword arguments` when the export
/// is not keyword-enabled.
///
/// This repeats only the *first* step of `pycc_mir`'s `source_frame_name`
/// (its `without_suffix` local), which is the canonical decomposition of a
/// mangled method name; that function then splits off the class and returns
/// the method alone, which is not what a message naming a class member
/// wants. A separate copy is deliberate rather than making that private
/// `fn` public: `crates/pycc_mir/src/lib.rs` is itself well over the
/// decomposability threshold, and exporting it would add a `cargo doc`
/// public-API obligation for one prefix strip.
pub(crate) fn source_level_name(name: &str) -> &str {
    for suffix in [".static", ".classmethod", ".setter"] {
        if let Some(stripped) = name.strip_suffix(suffix) {
            return stripped;
        }
    }
    name
}
