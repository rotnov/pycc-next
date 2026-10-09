//! Receiver-exact inherited methods (#1337, D-254).
//!
//! **Invariant.** Every call to a user method runs the body that the
//! receiver's static class resolves the member to, and that body makes every
//! receiver-dependent decision -- member bodies, class-attribute values,
//! property accessors, `super()` targets, `isinstance(self, ...)`,
//! `cls(...)`, `type(self)(...)` -- for that class.
//!
//! pycc compiles a method body once, with `self` typed as its defining class
//! `D`, and dispatches statically (D-006). A subclass `C` that inherits the
//! body without overriding it used to run `D`'s compilation, so a
//! `self.m()` inside it called `D.m` even when `C` overrides `m` -- a silent
//! miscompile. This pass clones each inherited body whose behaviour depends
//! on the receiver into a new `HirItem::Function` with the receiver retyped
//! to `C` ([`plan`] decides which ones), so the checker, `monomorphize`, MIR
//! and codegen compile it for `C` like any other item. A body whose
//! behaviour cannot differ keeps using the original, with no code growth.
//!
//! The pass runs in [`crate::module`]'s shared pre-pass, after empty
//! container and attribute-slot resolution: the copies must inherit the
//! resolved slot types, and those passes enumerate a class's own methods
//! from the class tables, where copies never appear. Copies are appended
//! after every linked item, in deterministic order; their names follow
//! [`pycc_hir::inherited_copy_name`], the one canonical spelling every later
//! phase parses back with [`pycc_hir::inherited_copy_origin`].

mod facts;
mod plan;

use std::collections::{BTreeSet, HashMap, HashSet};

use pycc_diag::{Diagnostic, Span};

use crate::module::{DiagnosticKey, KeyedDiagnostics};

use pycc_hir::{
    HirItem, HirModule, HirStmt, Ty, dataclass_repr_body, inherited_copy_name,
    inherited_copy_origin, is_builtin_exception_class,
};

use facts::{BodyFacts, Walker};
use plan::{PlannedCopy, Tables};

/// A module with its inherited-method copies appended, plus where each copy
/// came from so the driver can report a copy's diagnostic at its origin.
pub(crate) struct CopiedModule {
    pub(crate) module: HirModule,
    /// Copy item index to `(origin item index, note)`.
    origins: HashMap<usize, (usize, String)>,
}

impl CopiedModule {
    /// Re-keys a diagnostic raised inside a copy to the copied origin item
    /// (the driver maps an item index back to the file that owns it; a
    /// copy's index has no file), notes which subclass the body was being
    /// compiled for, and drops a copy diagnostic that repeats one already
    /// reported for the same origin at the same place (D-254 rule 5).
    pub(crate) fn rekey(&self, diagnostics: KeyedDiagnostics) -> KeyedDiagnostics {
        let mut seen: HashSet<(DiagnosticKey, &'static str, Option<Span>)> = HashSet::new();
        let mut out = Vec::with_capacity(diagnostics.len());
        for (key, mut diagnostic) in diagnostics {
            let copied = match key {
                DiagnosticKey::Function(index) => self.origins.get(&index),
                _ => None,
            };
            let key = match copied {
                Some((origin, note)) => {
                    diagnostic.help = Some(match diagnostic.help.take() {
                        Some(help) => format!("{help}\n{note}"),
                        None => note.clone(),
                    });
                    DiagnosticKey::Function(*origin)
                }
                None => key,
            };
            // Only a copy's diagnostic is ever dropped: two identical
            // diagnostics of one ordinary item are reported as before.
            let fresh = seen.insert((key, diagnostic.code, diagnostic.span));
            if fresh || copied.is_none() {
                out.push((key, diagnostic));
            }
        }
        out
    }
}

/// Returns `hir` with every needed inherited-method copy appended, or
/// `None` when no copy is needed (the common case keeps its no-clone
/// property).
pub(crate) fn add_inherited_copies(hir: &HirModule) -> Option<CopiedModule> {
    let class_of = |name: &str| {
        hir.class_defs
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, d)| d)
    };
    debug_assert!(
        hir.items.iter().all(|item| !matches!(
            item,
            HirItem::Function { name, .. } if inherited_copy_origin(name, &class_of).is_some()
        )),
        "pycc_types: internal error: an item existing before the copy pass is spelled like an \
         inherited-method copy"
    );
    let planned = plan_copies(hir);
    if planned.is_empty() {
        return None;
    }
    let mut module = hir.clone();
    let mut origins = HashMap::new();
    for copy in &planned {
        let receiver = class_of(&copy.receiver).expect("planned receiver is a class");
        let name = inherited_copy_name(receiver, &copy.origin_class, &copy.origin_name, &class_of)
            .expect("planned origin is a copyable item of its class");
        let note = format!(
            "while compiling `{}` inherited by subclass `{}`",
            copy.origin_name
                .split('.')
                .take(2)
                .collect::<Vec<_>>()
                .join("."),
            copy.receiver
        );
        for (origin_index, item) in hir.items.iter().enumerate() {
            let HirItem::Function {
                name: origin,
                params,
                return_ty,
                body,
            } = item
            else {
                continue;
            };
            if *origin != copy.origin_name {
                continue;
            }
            origins.insert(module.items.len(), (origin_index, note.clone()));
            module.items.push(HirItem::Function {
                name: name.clone(),
                params: retype_receiver(params, copy),
                return_ty: return_ty.clone(),
                body: copy_body(hir, body, copy),
            });
        }
    }
    Some(CopiedModule { module, origins })
}

/// The copy-needed plan for `hir` (see [`plan`]).
fn plan_copies(hir: &HirModule) -> Vec<PlannedCopy> {
    let method_items: HashMap<&str, ()> =
        hir.class_defs
            .iter()
            .flat_map(|(_, d)| {
                d.methods
                    .iter()
                    .map(|(_, m)| m.as_str())
                    .chain(d.class_methods.iter().map(|(_, m)| m.as_str()))
                    .chain(d.properties.iter().flat_map(|p| {
                        std::iter::once(p.getter.as_str()).chain(p.setter.as_deref())
                    }))
            })
            .map(|m| (m, ()))
            .collect();
    let mut facts: HashMap<String, BodyFacts> = HashMap::new();
    let mut observers: BTreeSet<String> = BTreeSet::new();
    for item in &hir.items {
        match item {
            HirItem::Function {
                name, params, body, ..
            } => {
                let receiver = method_items
                    .contains_key(name.as_str())
                    .then(|| params.first().map(|(p, _)| p.as_str()))
                    .flatten();
                let mut walker = Walker::new(receiver);
                walker.stmts(body);
                observers.extend(walker.observers);
                if receiver.is_some() {
                    facts.entry(name.clone()).or_default().merge(walker.facts);
                }
            }
            HirItem::TopLevelStmt(stmt) => {
                let mut walker = Walker::new(None);
                walker.stmts(std::slice::from_ref(stmt));
                observers.extend(walker.observers);
            }
        }
    }
    let tables = Tables {
        classes: hir
            .class_defs
            .iter()
            .map(|(n, d)| (n.as_str(), d))
            .collect(),
        user_classes: hir
            .class_defs
            .iter()
            .filter(|(n, _)| {
                !(hir.seeded_builtin_exception_classes && is_builtin_exception_class(n))
            })
            .map(|(_, d)| d)
            .collect(),
        facts: &facts,
        observers: &observers,
    };
    tables.plan()
}

/// The copy's parameters: the receiver retyped to the copy's receiver
/// class, and -- for a dataclass-synthesized `__eq__`, whose `other` is typed
/// as its own class -- `other` too, so comparing against a different class
/// is refused instead of answering from the wrong class's fields.
fn retype_receiver(params: &[(String, Ty)], copy: &PlannedCopy) -> Vec<(String, Ty)> {
    let origin = Ty::Instance(Box::new(copy.origin_class.clone()));
    let receiver = Ty::Instance(Box::new(copy.receiver.clone()));
    let is_eq = copy.origin_name == format!("{}.__eq__", copy.origin_class);
    params
        .iter()
        .enumerate()
        .map(|(i, (name, ty))| {
            let retype = *ty == origin && (i == 0 || (is_eq && i == 1));
            (
                name.clone(),
                if retype { receiver.clone() } else { ty.clone() },
            )
        })
        .collect()
}

/// A dataclass-synthesized `__repr__` spells its class name as a literal;
/// the copy's body is regenerated with the receiver's name, as CPython's
/// `type(self).__qualname__` would print it. Every other body is cloned
/// unchanged.
fn copy_body(hir: &HirModule, body: &[HirStmt], copy: &PlannedCopy) -> Vec<HirStmt> {
    let origin = hir
        .class_defs
        .iter()
        .find(|(n, _)| *n == copy.origin_class)
        .map(|(_, d)| d);
    match origin {
        Some(def)
            if def.is_dataclass
                && copy.origin_name == format!("{}.__repr__", copy.origin_class) =>
        {
            dataclass_repr_body(&copy.receiver, &def.dataclass_fields)
        }
        _ => body.to_vec(),
    }
}

/// #1420: refuses a copy whose resolved return differs from its origin's
/// (see [`copy_return_agrees`]) -- the one way a copy's signature can drift
/// from the origin's.
///
/// A caller types `recv.m()` through the class tables, where copies never
/// appear, so it sees the *origin's* return, while codegen dispatches to the
/// copy. An annotated return is copied verbatim and each copy's body is
/// checked against it, so the two cannot disagree. An *inferred* return is
/// solved per item, and since #1420 it can follow a receiver-dependent
/// method call: `def _t(self): return self.val()` infers `int` for `Base`
/// but `str` for a `Derived` whose `val` returns `str`, and the call
/// `Derived(...)._t()` would be typed `int` but run the `str` body. A
/// narrower copy return (`type(self)(...)` answering the subclass) stays
/// admitted: the origin's type is still a sound description of it.
///
/// The diagnostic is the one an annotated origin raises for the same body
/// (`T0022`, "return type mismatch"); the driver's [`CopiedModule::rekey`]
/// reports it at the origin with the note naming the receiver class.
pub(crate) fn check_copy_return(env: &crate::Environment, name: &str) -> Result<(), Diagnostic> {
    let Some(copy) = inherited_copy_origin(name, &|class| env.lookup_class(class)) else {
        return Ok(());
    };
    // A copy and its origin are both registered functions; a missing
    // signature (none is known) has nothing to compare and is admitted.
    let returns = env
        .lookup_function(name)
        .zip(env.lookup_function(&copy.origin_name));
    let Some(((_, copy_return), (_, origin_return))) =
        returns.filter(|((_, copy), (_, origin))| !copy_return_agrees(env, copy, origin))
    else {
        return Ok(());
    };
    Err(Diagnostic::error(
        "T0022",
        format!(
            "return type mismatch: expected `{}`, found `{}` (inherited `{}` compiled for subclass `{}`)",
            origin_return.name(),
            copy_return.name(),
            copy.origin_name,
            copy.receiver
        ),
        Span::new(0, 0),
    )
    .with_help(format!(
        "the inherited `{}` infers `{}` for `{}`; annotate its return type, or make the overriding members agree",
        copy.member,
        copy_return.name(),
        copy.receiver
    )))
}

/// Whether a copy's return `narrow` may stand where callers expect the
/// origin's `wide`: the same type, or an instance of a class whose MRO
/// contains `wide`'s class. (No `Optional` arm: the solver joins no
/// `T | None` return for an unannotated helper, so an inferred copy return
/// is never one, and an annotated return is copied verbatim.) pycc's
/// assignability is deliberately not used: it admits `bool` where `int` is
/// expected, and a copy returning `bool` while callers read the origin's
/// `int` is the same representation drift this check exists to refuse.
/// A copy answering `type(self)(...)` legitimately narrows the origin's
/// `Base` to the receiver's subclass, an instance pointer either way, and
/// the origin's type still describes every value the copy returns.
fn copy_return_agrees(env: &crate::Environment, narrow: &Ty, wide: &Ty) -> bool {
    match (narrow, wide) {
        (Ty::Instance(sub), Ty::Instance(base)) => env
            .lookup_class(sub)
            .is_some_and(|def| def.mro.iter().any(|class| class == base.as_ref())),
        _ => narrow == wide,
    }
}

#[cfg(test)]
#[path = "inherited_copies/tests.rs"]
mod tests;
