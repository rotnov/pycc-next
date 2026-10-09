//! Whole-module redefinition/redeclaration validation.
//!
//! Extracted from `pycc_types::lib` (see the file-decomposition review
//! finding on PR #836) to keep the crate root under the ~1,000-line
//! guideline in `AGENTS.md`'s "Keep source files decomposable" section.
//! Both checks below run once per module, before any per-function
//! `Environment`/signature-inference work, and are called from both
//! `check_all` and `checked_function_signatures_all` (both in `module.rs`).

use pycc_diag::{Diagnostic, Span};
use pycc_hir::{HirClassDef, HirItem, HirModule, Ty};
use std::collections::HashMap;

/// Issue #22: reject a redefinition of the same function name with an
/// incompatible signature. The codegen uses the first definition's LLVM
/// function type for indirect calls through the per-name function-pointer
/// slot, so all definitions of the same name must share one signature.
/// CPython allows arbitrary redefinition, but pycc's v0.1/v0.2 scope does
/// not need to support it, and the pre-#22 behavior was already broken
/// (a compile-time LLVM verification failure or silent runtime UB). This
/// check makes the "one signature per name" invariant the codegen already
/// assumes actually true.
///
/// Compares each definition's raw, pre-resolution `(param_tys, return_ty)`
/// shape via `Ty`'s derived structural `PartialEq` -- including any
/// `Ty::Infer` position. `Ty::Infer` is an ordinary unit variant under that
/// derive, so it only ever equals another `Ty::Infer` at the same position,
/// never a concrete type: comparing it unconditionally is correct by
/// construction, not a false positive. An earlier version of this function
/// skipped any function whose signature contained `Ty::Infer`, reasoning
/// that comparing an unresolved `Infer` against a concrete type would be
/// unsound -- that reasoning was wrong (the derived `PartialEq` already
/// handles it correctly) and the skip instead let a same-named,
/// same-arity, `Ty::Infer`-vs-concrete redefinition collapse onto one
/// shared signature and pass silently (issue #402). Do not reintroduce a
/// skip for `Ty::Infer` positions.
///
/// Called from `check_all` and `checked_function_signatures_all` (catches the
/// concrete path before the concrete/solver split), both pre-resolution.
/// `check_and_resolve` no longer needs its own post-resolution recheck:
/// since this function now rejects every raw-shape mismatch up front
/// (arity or `Infer`-vs-concrete), any redefinition pair it accepts is
/// already raw-shape-identical, and `infer_function_signatures_with_solver_all`
/// resolves same-named items through one shared, name-keyed signature
/// entry -- so raw-shape-identical items are guaranteed to resolve to the
/// same concrete signature too. A post-resolution recheck would therefore
/// never observe a case this pre-resolution check didn't already reject.
pub(crate) fn check_incompatible_redefinitions(hir: &HirModule) -> Result<(), Diagnostic> {
    let mut seen: HashMap<String, (Vec<Ty>, Ty)> = HashMap::new();
    for item in &hir.items {
        let HirItem::Function {
            name,
            params,
            return_ty,
            ..
        } = item
        else {
            continue;
        };
        let current = (
            params.iter().map(|(_, ty)| ty.clone()).collect::<Vec<_>>(),
            return_ty.clone(),
        );
        if let Some(prev) = seen.get(name) {
            if prev != &current {
                return Err(Diagnostic::error(
                    "T0021",
                    format!(
                        "cannot redefine function `{name}` with a different signature \
                         (previous: {}, current: {})",
                        format_function_signature(prev),
                        format_function_signature(&current),
                    ),
                    Span::new(0, 0),
                ));
            }
        } else {
            seen.insert(name.clone(), current);
        }
    }
    Ok(())
}

/// #676 (D-210): rejects a cross-MRO attribute redeclaration whose declared
/// type differs from another class's own declaration of the same attribute
/// name, anywhere in one class's own linearized MRO.
///
/// Two symptoms motivate this, both stemming from the same root cause: a
/// base-class method resolves an attribute's declared slot type from its
/// *own* MRO-attribute entry (`pycc_mir`'s `mro_attrs`/`class_def_of`), but
/// a derived class can redeclare that same attribute with an incompatible
/// type. pycc has no per-instance runtime type tag and does not
/// monomorphize a method per subclass (static dispatch, one compiled body
/// per method, `docs/TYPE_SYSTEM.md`), so the one compiled call site cannot
/// distinguish, at run time, which declaration applies -- there is no sound
/// way to coerce at the assignment site. Left unrejected, this produces an
/// undiagnosed mis-decoded read (D-141's tagged representation reads back
/// the wrong Python value) or an outright runtime abort/segfault, entirely
/// silently at `pycc check` time. See D-210 for the full rationale,
/// including why "coerce" is unsound in general (a bare, non-derived
/// instance's own correctly-typed slot would be corrupted by any
/// unconditional coercion inserted at the shared compiled call site).
///
/// The comparison is symmetric and direction-independent: for each class
/// `C`, walk `C`'s own C3-linearized MRO (`HirClassDef::mro`, which
/// includes `C` itself and every ancestor) and compare every attribute
/// name's declaring classes' own (non-inherited) `attrs` entries
/// pairwise. This also catches a diamond conflict between two sibling
/// base classes that neither is the other's ancestor, through their
/// common descendant's own MRO, even when that descendant never
/// redeclares the attribute itself.
///
/// Only a *differing* declared `Ty` triggers `T0052` -- an identical
/// redeclaration across the MRO (the existing `mro_attrs` dedup case) and
/// a same-class attribute assigned a *value* of a different (but
/// admissible, e.g. `bool` into `int`) type at a single declaration site
/// are both unaffected: this check only ever compares two distinct
/// classes' own declared attribute types, never a single class's
/// attribute against a mere assignment.
///
/// This also reaches a `@dataclass` hierarchy's own field-name conflicts
/// in one specific case: a differing-type conflict *between two field
/// declarations already merged by name* is rejected earlier, during HIR
/// lowering itself (`pycc_hir::class`'s dataclass `merged_fields`
/// construction), not here -- that merge walks the MRO least-derived-first
/// and, on encountering a same-named field with a differing declared type,
/// returns `T0052` directly instead of silently keeping the first
/// declaration. Because that HIR-lowering-time check already rejects the
/// conflicting pair, a `@dataclass` class's own `HirClassDef::attrs` never
/// contains two differing-type entries for the same name by the time this
/// module-level MRO walk runs, so this function's own walk cannot observe
/// a divergent pair for it (it would already have been rejected earlier).
/// A conflict between an ordinary (non-dataclass) class and any other
/// class in its MRO is unaffected by this and is still caught here.
///
/// Called from `check`, mirroring `check_incompatible_redefinitions`'s
/// early-return-on-first-conflict shape and call-site timing (before any
/// `Environment`/signature-inference work), since it only needs each
/// class's own already-lowered `attrs` and `mro`, both populated at
/// HIR-lowering time (`pycc_hir::class`) before `pycc_types::check` ever
/// runs.
pub(crate) fn check_incompatible_attribute_redeclarations(
    hir: &HirModule,
) -> Result<(), Diagnostic> {
    let classes: HashMap<&str, &HirClassDef> = hir
        .class_defs
        .iter()
        .map(|(name, def)| (name.as_str(), def))
        .collect();

    for (_, class_def) in &hir.class_defs {
        // First-seen order within this class's own MRO: (attr_name,
        // owner_class_name, declared_ty). A HashMap would make which
        // conflicting pair gets reported (when several attribute names
        // conflict) depend on hash-iteration order; a Vec keeps the
        // reported pair deterministic and tied to MRO order.
        let mut declared: Vec<(&str, &str, &Ty)> = Vec::new();
        for mro_name in &class_def.mro {
            // `.expect()`, not a hand-rolled `if let Some ... else {
            // continue }`, per this crate's own established
            // coverage-gate convention for a provably-unreachable shape
            // (`pycc_hir::class`'s own `.expect(...)` precedent at its
            // `collect_init_attrs`/dataclass-field-merge MRO lookups):
            // `class_def.mro` is C3-linearized from `hir.class_defs`
            // itself, so every name in it already has a registered
            // class def by construction; a hand-rolled skip branch here
            // could never be reached by real compiler input and would
            // otherwise demand a synthetic-only test purely to satisfy
            // D-014's 100%-region gate.
            let mro_def = classes
                .get(mro_name.as_str())
                .expect("every name in a well-formed MRO has a registered class def");
            for (attr_name, ty) in &mro_def.attrs {
                if let Some(&(_, prev_owner, prev_ty)) = declared
                    .iter()
                    .find(|(name, _, _)| *name == attr_name.as_str())
                {
                    if prev_ty != ty {
                        return Err(Diagnostic::error(
                            "T0052",
                            format!(
                                "attribute `{attr_name}` is declared as `{}` in class \
                                 `{prev_owner}` and as `{}` in class `{mro_name}`, both in \
                                 the method resolution order of class `{}`",
                                prev_ty.name(),
                                ty.name(),
                                class_def.name,
                            ),
                            Span::new(0, 0),
                        ));
                    }
                } else {
                    declared.push((attr_name.as_str(), mro_name.as_str(), ty));
                }
            }
        }
    }
    Ok(())
}

/// Formats a `(param_tys, return_ty)` pair as `def name(params) -> return`
/// for diagnostic messages.
fn format_function_signature(sig: &(Vec<Ty>, Ty)) -> String {
    let params = sig
        .0
        .iter()
        .map(|ty| ty.name())
        .collect::<Vec<_>>()
        .join(", ");
    format!("({params}) -> {}", sig.1.name())
}
