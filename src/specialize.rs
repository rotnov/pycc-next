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
//! diagnostic) drops every specialization that fails alone with it, or
//! when none does is narrowed down to one specialization by halving
//! ([`culprit`]); and the attempt is repeated. A specialization that calls
//! a builtin its original calls through CPython
//! (`Specialized::restored_builtins`) is not dropped at its first failure
//! but planned again calling builtins as the original does
//! ([`KEEP_BUILTINS`]). What fails without
//! specializations too is the program's own failure, which the caller (the
//! fallback loop) handles as before. Attempts that do not depend on each
//! other run in parallel ([`run_all`], v0.5 accept 4).

use crate::frontend::FrontendFailure;
use pycc_ast::Specialized;
use std::cell::{Cell, RefCell};
use std::collections::{BTreeMap, BTreeSet};

thread_local! {
    /// Whether the loader specializes the entry module now.
    static ACTIVE: Cell<bool> = const { Cell::new(false) };
    /// The qualnames whose specialization the checker refused, and why.
    static DROPPED: RefCell<BTreeMap<String, String>> = const { RefCell::new(BTreeMap::new()) };
    /// The qualnames whose specialization calls builtins as its original
    /// does, having failed calling them itself.
    static KEEP_BUILTINS: RefCell<BTreeSet<String>> = const { RefCell::new(BTreeSet::new()) };
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
    KEEP_BUILTINS.with(|keep| keep.borrow_mut().clear());
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
    let keep = KEEP_BUILTINS.with(|keep| keep.borrow().clone());
    let planned = DROPPED.with(|dropped| {
        let dropped = dropped.borrow();
        pycc_ast::specialize_untyped(
            parsed,
            &|qualname| {
                dropped.contains_key(qualname)
                    || only.as_ref().is_some_and(|only| !only.contains(qualname))
            },
            &|qualname| keep.contains(qualname),
        )
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

/// Whether `key` is one of the program's own failures (`known`): the same
/// diagnostic, or the same one without its span. A specialized attempt can
/// report an original's diagnostic without a span (both copies hold its
/// source range, so locating it is ambiguous); it is not the copy's.
fn is_known(known: &BTreeSet<Key>, key: &Key) -> bool {
    known.contains(key)
        || (key.3.is_none()
            && known
                .iter()
                .any(|k| (&k.0, k.1, &k.2) == (&key.0, key.1, &key.2)))
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

/// One attempt of the loop: the program as written (`Plain`), or with the
/// loader specializing (`Specialized`, `Some` limiting to those qualnames).
#[derive(Clone)]
enum Job {
    Plain,
    Specialized(Option<BTreeSet<String>>),
}

/// Runs one job on the current thread.
fn run<T>(
    job: Job,
    attempt: &impl Fn() -> Result<T, FrontendFailure>,
) -> (Result<T, FrontendFailure>, Vec<Specialized>) {
    match job {
        Job::Plain => (attempt(), Vec::new()),
        Job::Specialized(only) => specialized(attempt, only),
    }
}

/// The stack of a thread running an attempt: the frontend recurses over
/// the program's syntax, as deep as on the main thread.
const ATTEMPT_STACK: usize = 64 * 1024 * 1024;

/// Runs `jobs`, as many at once as this process may use threads
/// (`crate::threads::budget`, every core unless it is a package member's
/// build; v0.5 accept 4):
/// each attempt is a whole frontend run, independent of the others, so
/// guesses the loop would try one by one cost one attempt's time per wave
/// rather than per guess. A job runs on a thread of its own with this
/// thread's loop state (dropped specializations, the fallback rewrite)
/// installed; the first job of a wave, and any job no thread could be
/// started for, runs here. Results come back in job order.
fn run_all<T: Send>(
    jobs: Vec<Job>,
    attempt: &(impl Fn() -> Result<T, FrontendFailure> + Sync),
) -> Vec<(Result<T, FrontendFailure>, Vec<Specialized>)> {
    let threads = crate::threads::budget();
    let mut results = Vec::with_capacity(jobs.len());
    let mut jobs = jobs.into_iter().peekable();
    while jobs.peek().is_some() {
        let mut wave = jobs.by_ref().take(threads.max(1)).collect::<Vec<_>>().into_iter();
        let here = wave.next();
        let dropped = DROPPED.with(|dropped| dropped.borrow().clone());
        let keep = KEEP_BUILTINS.with(|keep| keep.borrow().clone());
        let fallback = crate::fallback::Snapshot::take();
        crate::timing::span("frontend: attempt wave", || std::thread::scope(|scope| {
            let workers: Vec<_> = wave
                .map(|job| {
                    let (dropped, keep) = (dropped.clone(), keep.clone());
                    let fallback = fallback.clone();
                    let moved = job.clone();
                    let worker = std::thread::Builder::new()
                        .stack_size(ATTEMPT_STACK)
                        .spawn_scoped(scope, move || {
                            DROPPED.with(|cell| *cell.borrow_mut() = dropped);
                            KEEP_BUILTINS.with(|cell| *cell.borrow_mut() = keep);
                            fallback.install();
                            run(moved, attempt)
                        });
                    (job, worker.ok())
                })
                .collect();
            if let Some(job) = here {
                results.push(run(job, attempt));
            }
            for (job, worker) in workers {
                let joined = worker.and_then(|handle| handle.join().ok());
                results.push(joined.unwrap_or_else(|| run(job, attempt)));
            }
        }));
    }
    results
}

/// Runs `attempt` with the entry module specialized, dropping each
/// specialization the attempt fails on until it passes or fails only as it
/// does without specializations.
///
/// With `speculate` (a non-strict build, where specializations are
/// planned) and a thread to spare ([`speculates`]), the first round also
/// runs the program without specializations, in parallel ([`run_all`]):
/// its diagnostics are what every later failure is compared with, and its
/// result is the answer when the program fails on its own. Without a spare
/// thread that attempt runs only once a failure needs it: the answer is the
/// same, and an attempt passing with its specializations (the common case)
/// costs one frontend run instead of two. A failure no specialization's
/// `def` holds is pinned on every specialization that fails alone with it,
/// all tried at once ([`alone`]); only when none does (it takes two of them
/// together) is one found by halving ([`culprit`]).
pub(crate) fn with_specialization<T: Send>(
    entry: &str,
    speculate: bool,
    attempt: impl Fn() -> Result<T, FrontendFailure> + Sync,
) -> Result<T, FrontendFailure> {
    // The program's result without specializations, and its keys.
    let mut known: Option<(Result<T, FrontendFailure>, BTreeSet<Key>)> = None;
    let with_keys = |plain: Result<T, FrontendFailure>| {
        let keys = match &plain {
            Ok(_) => BTreeSet::new(),
            Err(failure) => keys(failure).into_iter().collect(),
        };
        (plain, keys)
    };
    loop {
        let mut jobs = vec![Job::Specialized(None)];
        if known.is_none() && speculates(speculate, crate::threads::budget()) {
            jobs.push(Job::Plain);
        }
        let mut results = run_all(jobs, &attempt).into_iter();
        let (result, planned) = results.next().expect("one result per job");
        if let Some((plain, _)) = results.next() {
            known = Some(with_keys(plain));
        }
        let failure = match result {
            Err(failure @ FrontendFailure::Compile { .. }) if !planned.is_empty() => failure,
            result => {
                if result.is_ok() {
                    PASSED.with(|passed| *passed.borrow_mut() = planned);
                }
                return result;
            }
        };
        let (_, known_keys) = known.get_or_insert_with(|| with_keys(attempt()));
        let new: Vec<Key> = keys(&failure)
            .into_iter()
            .filter(|key| !is_known(known_keys, key))
            .collect();
        if new.is_empty() {
            // The program's own failure, reported as the program alone
            // fails (a copy would repeat each diagnostic of its original).
            // A plain result is reused only as a failure: a program that
            // passes is compiled by the main thread's own attempt, whose
            // thread-local state codegen reads.
            return match known.expect("computed above").0 {
                Err(failure) => Err(failure),
                Ok(_) => attempt(),
            };
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
            blamed = alone(&planned, known_keys, &attempt);
        }
        if blamed.is_empty() {
            let suspects = planned.iter().map(|spec| spec.qualname.clone()).collect();
            blamed.insert(culprit(suspects, known_keys, &attempt), reason(&new[0]));
        }
        for (qualname, reason) in blamed {
            let restored = planned
                .iter()
                .any(|spec| spec.qualname == qualname && spec.restored_builtins);
            if restored {
                KEEP_BUILTINS.with(|keep| keep.borrow_mut().insert(qualname));
            } else {
                DROPPED.with(|dropped| dropped.borrow_mut().insert(qualname, reason));
            }
        }
    }
}

/// Whether the first round runs the program without specializations
/// beside the specialized attempt ([`with_specialization`]): only to save
/// time, so only when there is a thread to run it on.
fn speculates(speculate: bool, threads: usize) -> bool {
    speculate && threads > 1
}

/// Every specialization among `planned` that fails alone with a diagnostic
/// `known` lacks, with that diagnostic as the reason: all of them tried at
/// once ([`run_all`]).
fn alone<T: Send>(
    planned: &[Specialized],
    known: &BTreeSet<Key>,
    attempt: &(impl Fn() -> Result<T, FrontendFailure> + Sync),
) -> BTreeMap<String, String> {
    let jobs = planned
        .iter()
        .map(|spec| Job::Specialized(Some(BTreeSet::from([spec.qualname.clone()]))))
        .collect();
    let mut blamed = BTreeMap::new();
    for (spec, (result, _)) in planned.iter().zip(run_all(jobs, attempt)) {
        if let Err(failure) = &result
            && let Some(key) = keys(failure).iter().find(|key| !is_known(known, key))
        {
            blamed.insert(spec.qualname.clone(), reason(key));
        }
    }
    blamed
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
            Err(failure) => keys(failure).iter().any(|key| !is_known(known, key)),
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

#[cfg(test)]
mod tests {
    /// The plain attempt runs ahead of a failure only beside the
    /// specialized one, never instead of a thread's own work.
    #[test]
    fn the_plain_attempt_is_speculative_only_with_a_spare_thread() {
        assert!(super::speculates(true, 2));
        assert!(!super::speculates(true, 1));
        assert!(!super::speculates(false, 4));
    }

    /// `pycc_mir` calls a specialization by the name `pycc_ast` gives it.
    #[test]
    fn the_two_crates_spell_a_specialization_alike() {
        assert_eq!(pycc_ast::NATIVE_PREFIX, pycc_mir::NATIVE_PREFIX);
    }
}
