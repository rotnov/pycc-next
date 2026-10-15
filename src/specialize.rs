//! Guarded native specializations of an `--ext --non-strict` entry module's
//! unannotated functions and methods (`pycc_ast::specialize_untyped`), and
//! the loop that keeps only those the checker accepts.
//!
//! The loader adds every specialization the AST pass plans
//! ([`specialize_entry`]). A specialization is a guess: its body may not
//! type-check with the guessed types (a method no native `str` has, an
//! `int` parameter that later holds a `float`). Such a failure must not
//! make the function, or its class, fall back to CPython, since the
//! original runs fine compiled. [`with_specialization`] therefore compares
//! a failing attempt with the same attempt without specializations: a
//! diagnostic only the specialized program has drops the specialization
//! whose `def` holds it; one no `def` holds (a span-less solver
//! diagnostic) is narrowed down to one specialization by halving
//! ([`culprit`]); and the attempt is repeated. What fails without
//! specializations too is the program's own failure, which the caller (the
//! fallback loop) handles as before.

use crate::frontend::FrontendFailure;
use pycc_ast::Specialized;
use std::cell::{Cell, RefCell};
use std::collections::{BTreeMap, BTreeSet};

thread_local! {
    /// Whether the loader specializes the entry module now.
    static ACTIVE: Cell<bool> = const { Cell::new(false) };
    /// The qualnames whose specialization the checker refused, and why.
    static DROPPED: RefCell<BTreeMap<String, String>> = const { RefCell::new(BTreeMap::new()) };
    /// What the loader specialized in the current attempt.
    static PLANNED: RefCell<Vec<Specialized>> = const { RefCell::new(Vec::new()) };
    /// What the last attempt that passed specialized, for the report.
    static PASSED: RefCell<Vec<Specialized>> = const { RefCell::new(Vec::new()) };
    /// While a failure is narrowed down ([`culprit`]): the only qualnames
    /// the loader specializes.
    static ONLY: RefCell<Option<BTreeSet<String>>> = const { RefCell::new(None) };
}

/// Forgets every dropped specialization, before a new program is built.
pub(crate) fn reset() {
    DROPPED.with(|dropped| dropped.borrow_mut().clear());
    PLANNED.with(|planned| planned.borrow_mut().clear());
    PASSED.with(|passed| passed.borrow_mut().clear());
}

/// Adds the specializations of the entry module `parsed`, when the current
/// attempt specializes ([`with_specialization`]).
pub(crate) fn specialize_entry(parsed: &mut pycc_ast::ModModule) {
    if !ACTIVE.with(Cell::get) {
        return;
    }
    let only = ONLY.with(|only| only.borrow().clone());
    let planned = DROPPED.with(|dropped| {
        let dropped = dropped.borrow();
        pycc_ast::specialize_untyped(parsed, &|qualname| {
            dropped.contains_key(qualname)
                || only.as_ref().is_some_and(|only| !only.contains(qualname))
        })
    });
    PLANNED.with(|cell| *cell.borrow_mut() = planned);
}

/// What identifies one diagnostic across two attempts: its file, code,
/// message and span.
type Key = (String, &'static str, String, Option<(u32, u32)>);

fn keys(failure: &FrontendFailure) -> Vec<Key> {
    let FrontendFailure::Compile { files } = failure else {
        return Vec::new();
    };
    files
        .iter()
        .flat_map(|file| {
            file.diagnostics.iter().map(|diagnostic| {
                let span = diagnostic
                    .span
                    .filter(|s| (s.start, s.end) != (0, 0))
                    .map(|s| (s.start, s.end));
                (
                    file.path.clone(),
                    diagnostic.code,
                    diagnostic.message.clone(),
                    span,
                )
            })
        })
        .collect()
}

/// A dropped specialization's reason, from the diagnostic that dropped it.
fn reason((_, code, message, _): &Key) -> String {
    format!("{code} {message}")
}

/// Runs `attempt` once with the loader specializing (`only` limiting which).
fn specialized<T>(
    attempt: &impl Fn() -> Result<T, FrontendFailure>,
    only: Option<BTreeSet<String>>,
) -> (Result<T, FrontendFailure>, Vec<Specialized>) {
    PLANNED.with(|planned| planned.borrow_mut().clear());
    ONLY.with(|cell| *cell.borrow_mut() = only);
    ACTIVE.with(|active| active.set(true));
    let result = attempt();
    ACTIVE.with(|active| active.set(false));
    ONLY.with(|cell| *cell.borrow_mut() = None);
    let planned = PLANNED.with(|planned| std::mem::take(&mut *planned.borrow_mut()));
    (result, planned)
}

/// Runs `attempt` with the entry module specialized, dropping each
/// specialization the attempt fails on until it passes or fails only as it
/// does without specializations.
pub(crate) fn with_specialization<T>(
    entry: &str,
    attempt: impl Fn() -> Result<T, FrontendFailure>,
) -> Result<T, FrontendFailure> {
    let mut known: Option<BTreeSet<Key>> = None;
    loop {
        let (result, planned) = specialized(&attempt, None);
        let failure = match result {
            Err(failure @ FrontendFailure::Compile { .. }) if !planned.is_empty() => failure,
            result => {
                if result.is_ok() {
                    PASSED.with(|passed| *passed.borrow_mut() = planned);
                }
                return result;
            }
        };
        // What fails without specializations does not depend on which
        // were dropped: computed once.
        let known = known.get_or_insert_with(|| match attempt() {
            Ok(_) => BTreeSet::new(),
            Err(failure) => keys(&failure).into_iter().collect(),
        });
        let new: Vec<Key> = keys(&failure)
            .into_iter()
            .filter(|key| !known.contains(key))
            .collect();
        if new.is_empty() {
            // The program's own failure, reported as the program alone
            // fails (a copy would repeat each diagnostic of its original).
            return attempt();
        }
        let mut blamed: BTreeMap<String, String> = BTreeMap::new();
        for spec in &planned {
            let range = spec.range.start().to_u32()..spec.range.end().to_u32();
            if let Some(key) = new.iter().find(|(path, _, _, span)| {
                path == entry && span.is_some_and(|(start, _)| range.contains(&start))
            }) {
                blamed.insert(spec.qualname.clone(), reason(key));
            }
        }
        if blamed.is_empty() {
            let suspects = planned.iter().map(|spec| spec.qualname.clone()).collect();
            blamed.insert(culprit(suspects, known, &attempt), reason(&new[0]));
        }
        DROPPED.with(|dropped| dropped.borrow_mut().extend(blamed));
    }
}

/// One specialization among `suspects` (which together fail with a
/// diagnostic `known` lacks, that no specialization's `def` holds): halves
/// the suspects, keeping a half that fails alone, until one is left.
fn culprit<T>(
    mut suspects: Vec<String>,
    known: &BTreeSet<Key>,
    attempt: &impl Fn() -> Result<T, FrontendFailure>,
) -> String {
    while suspects.len() > 1 {
        let second = suspects.split_off(suspects.len() / 2);
        let (result, _) = specialized(attempt, Some(suspects.iter().cloned().collect()));
        let fails = match &result {
            Ok(_) => false,
            Err(failure) => keys(failure).iter().any(|key| !known.contains(key)),
        };
        if !fails {
            suspects = second;
        }
    }
    suspects.pop().unwrap_or_default()
}

/// `--report-fallbacks`: what runs natively when called with the types it
/// was specialized for in the build that passed, and which guesses the
/// checker refused.
pub(crate) fn report() {
    if !crate::fallback::reporting() {
        return;
    }
    for spec in PASSED.with(|passed| passed.borrow().clone()) {
        let params = spec
            .params
            .iter()
            .map(|(name, ty)| format!("{name}: {ty}"))
            .collect::<Vec<_>>()
            .join(", ");
        eprintln!(
            "specialized: `{}` runs natively when called with {params}",
            spec.qualname
        );
    }
    for (qualname, reason) in DROPPED.with(|dropped| dropped.borrow().clone()) {
        eprintln!("not specialized: `{qualname}`: {reason}");
    }
}
