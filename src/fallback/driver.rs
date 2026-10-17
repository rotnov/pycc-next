//! The fallback loop around the frontend: compile, absorb the failures
//! into the plan, render the rewrite, and retry until it converges.

use super::*;

/// Whether a diagnostic is one a fallback can resolve: anything but a
/// syntax error, an interop-policy or embedding refusal.
pub(crate) fn can_fall_back(diagnostic: &Diagnostic) -> bool {
    !matches!(
        diagnostic.code,
        "L0001" | "L0002" | "I0402" | "I0403" | "I0405"
    )
}

/// Runs `attempt` (one frontend pass) with the fallback loop around it,
/// in non-strict mode. `split` turns a failure into the entry file's
/// fallback-resolvable diagnostics, or `None` when the failure cannot be
/// resolved by falling back (another file, a syntax error, an input
/// error).
pub(crate) fn with_fallback<T, E>(
    path: &Path,
    display: &str,
    publishes: Publishes,
    attempt: impl Fn() -> Result<T, E>,
    split: impl Fn(&E) -> Option<Vec<Failure>>,
) -> Result<T, E> {
    set_override(None);
    let first = attempt();
    let failures = match &first {
        Ok(_) if publishes.variables && mode_allows(path) => {
            // Nothing falls back, but Python code reads the module's
            // variables: compile the rewrite that publishes them.
            let Some(mut plan) =
                read_entry(path).and_then(|s| Plan::new(&s, display, publishes))
            else {
                return first;
            };
            plan.set_entry(path);
            let subclassed = plan
                .stmts
                .iter()
                .any(|stmt| matches!(&stmt.kind, Kind::Class(n) if plan.package_bases.contains_key(n)));
            if plan.external.is_empty()
                && !plan.stmts.iter().any(|stmt| stmt.hidden_members)
                && !subclassed
            {
                return first;
            }
            plan.cascade();
            if plan.falls_back_a_project_import() {
                return first;
            }
            let rendered = plan.render();
            if std::env::var_os("PYCC_FALLBACK_DUMP").is_some() {
                eprintln!("--- fallback rewrite ---\n{}", rendered.text);
            }
            install(&rendered);
            let value = match attempt() {
                Ok(value) => Some(value),
                Err(failure) => split(&failure).and_then(|failures| {
                    converge(&mut plan, Some(rendered), failures, &attempt, &split)
                }),
            };
            return match value {
                Some(value) if !plan.falls_back_a_project_import() => {
                    if REPORT.load(Ordering::Relaxed) {
                        for line in plan.report_lines() {
                            eprintln!("{line}");
                        }
                    }
                    Ok(value)
                }
                _ => {
                    set_override(None);
                    attempt()
                }
            };
        }
        // Strict mode: Python code reads the module's variables, which a
        // rewrite copies onto the module object (`crate::ext_globals`).
        // Not under `--foreign-relative-imports`: the artifact is imported
        // under a dotted name its `__name__` may not spell.
        Ok(_)
            if publishes.variables
                && !publishes.foreign_relative_imports
                && !crate::mode::non_strict_for(path) =>
        {
            return publish_strict(path, first, &attempt, &split);
        }
        Ok(_) => return first,
        Err(failure) => match split(failure) {
            Some(failures) if mode_allows(path) =>
            {
                failures
            }
            _ => return first,
        },
    };
    let source = read_entry(path);
    let Some(mut plan) = source.and_then(|s| Plan::new(&s, display, publishes)) else {
        return first;
    };
    plan.set_entry(path);
    // With annotation aliases substituted (`annotations`), the failures of
    // the source as written may be ones the substitution removes: the loop
    // starts from the rewrite's own.
    let (rendered, failures) = if plan.substituted.is_empty() {
        (None, failures)
    } else {
        let rendered = plan.render();
        if std::env::var_os("PYCC_FALLBACK_DUMP").is_some() {
            eprintln!("--- fallback rewrite ---\n{}", rendered.text);
        }
        install(&rendered);
        match attempt() {
            Ok(value) => {
                if REPORT.load(Ordering::Relaxed) {
                    for line in plan.report_lines() {
                        eprintln!("{line}");
                    }
                }
                return Ok(value);
            }
            Err(failure) => match split(&failure) {
                Some(failures) => (Some(rendered), failures),
                None => {
                    set_override(None);
                    return first;
                }
            },
        }
    };
    let Some(mut value) = converge(&mut plan, rendered, failures, &attempt, &split) else {
        set_override(None);
        return first;
    };
    // Fallback code that reaches a project module would need CPython to
    // import it: the program is refused with its original diagnostics.
    if plan.falls_back_a_project_import() {
        set_override(None);
        return first;
    }
    // A statement that fell back for a type error early on may compile now
    // that later rounds changed what it sees (a global kept native, a
    // function compiled with objects, a builtin called through CPython):
    // run the loop once more from a plan without those, and keep whichever
    // result falls back less.
    let mut retry = plan.clone();
    retry.fallback.retain(|_, reason| !retryable(reason));
    retry.prune_lifts();
    // So is a function compiled with objects only for fallback code that
    // may be gone: a compiled class instance it then received as an object
    // would have no attributes Python code can see.
    retry.objectified.clear();
    // What fallback code that stays forces is forced again.
    retry.cascade();
    if retry.fallback.len() < plan.fallback.len() {
        let kept = Snapshot::take();
        let rendered = retry.render();
        install(&rendered);
        let retried = match attempt() {
            Ok(value) => Some(value),
            Err(failure) => split(&failure)
                .and_then(|failures| converge(&mut retry, Some(rendered), failures, &attempt, &split)),
        };
        match retried {
            Some(better)
                if retry.fallback.len() < plan.fallback.len()
                    && !retry.falls_back_a_project_import() =>
            {
                plan = retry;
                value = better;
            }
            _ => kept.install(),
        }
    }
    if REPORT.load(Ordering::Relaxed) {
        for line in plan.report_lines() {
            eprintln!("{line}");
        }
    }
    Ok(value)
}

/// Compiles the strict `--ext` rewrite publishing the entry module's
/// variables (`crate::ext_globals`), dropping each name whose copy fails to
/// compile; `first` (nothing published) when a failure is not one of the
/// copies.
fn publish_strict<T, E>(
    path: &Path,
    first: Result<T, E>,
    attempt: &impl Fn() -> Result<T, E>,
    split: &impl Fn(&E) -> Option<Vec<Failure>>,
) -> Result<T, E> {
    let Some(source) = read_entry(path) else {
        return first;
    };
    let mut dropped = BTreeSet::new();
    loop {
        let Some(rewrite) = crate::ext_globals::rewrite(&source, &dropped) else {
            set_override(None);
            return first;
        };
        if std::env::var_os("PYCC_FALLBACK_DUMP").is_some() {
            eprintln!("--- ext globals rewrite ---\n{}", rewrite.text);
        }
        set_override(Some(rewrite.text.clone()));
        let failure = match attempt() {
            Ok(value) => return Ok(value),
            Err(failure) => failure,
        };
        let failures = split(&failure).unwrap_or_default();
        let mut progress = false;
        let mut attributed = !failures.is_empty();
        for failure in &failures {
            let at = failure.diagnostic.span.map(|s| s.start).or(failure.origin);
            match at.and_then(|at| rewrite.copies.iter().find(|(range, _)| range.contains(&at))) {
                Some((_, name)) => progress |= dropped.insert(name.clone()),
                None => attributed = false,
            }
        }
        if !attributed || !progress {
            set_override(None);
            return first;
        }
    }
}

/// Whether the fallback loop runs for `path`: non-strict mode, and not
/// disabled with `PYCC_NO_FALLBACK`.
fn mode_allows(path: &Path) -> bool {
    crate::mode::non_strict_for(path) && std::env::var_os("PYCC_NO_FALLBACK").is_none()
}

/// Whether a statement that fell back for `reason` may compile in a later
/// plan: a type error may, an unsupported construct (`C0001`) never does.
fn retryable(reason: &str) -> bool {
    !reason.starts_with("C0001")
}

/// The fallback loop: absorbs `failures` into `plan`, renders and compiles
/// until an attempt succeeds (its value, with the rewrite as the entry
/// override) or no failure marks anything new (`None`).
fn converge<T, E>(
    plan: &mut Plan,
    mut rendered: Option<Rendered>,
    mut failures: Vec<Failure>,
    attempt: &impl Fn() -> Result<T, E>,
    split: &impl Fn(&E) -> Option<Vec<Failure>>,
) -> Option<T> {
    let dump = std::env::var_os("PYCC_FALLBACK_DUMP").is_some();
    loop {
        if !plan.absorb(rendered.as_ref(), &failures) {
            if dump {
                eprintln!("--- fallback loop stuck on ---");
                for failure in &failures {
                    eprintln!("{} {:?}", describe(&failure.diagnostic), failure.diagnostic.span);
                }
            }
            return None;
        }
        let next = plan.render();
        if dump {
            eprintln!("--- fallback rewrite ---\n{}", next.text);
        }
        install(&next);
        rendered = Some(next);
        match attempt() {
            Ok(value) => return Some(value),
            Err(failure) => match split(&failure) {
                Some(next) => failures = next,
                None => {
                    if dump {
                        eprintln!("--- fallback loop cannot place a failure ---");
                    }
                    return None;
                }
            },
        }
    }
}

/// The entry file's decoded source.
fn read_entry(path: &Path) -> Option<String> {
    let bytes = std::fs::read(path).ok()?;
    crate::source::decode_python_source(&bytes).ok()
}
