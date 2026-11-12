//! Pass 3 of `pycc profile` (docs/design/compile-little.md, section 2.3):
//! only measured winners are selected.
//!
//! The selection rule picks candidates from the profile's self times, but a
//! compiled untyped body called from bytecode, or calling back into it, can be
//! slower than CPython's specializing interpreter running it. So the profile
//! builds the candidates and checks the training workload with them compiled
//! against the unchanged sources: its wall time, interleaved over at most
//! `--verify-rounds` fresh processes each after one discarded warm-up run of
//! each (a check stops after [`EARLY_ROUNDS`] once the two interquartile
//! ranges are apart), and, when pass 3 counts them ([`Signal`]), its
//! instructions under callgrind, once per check against one count of CPython
//! for the whole pass. The sets checked:
//!
//! 1. all the candidates together, and the hot ones among them (each at
//!    least the rule's share and calls on its own) alone;
//! 2. when neither wins, each region (the candidates connected by calls
//!    among them), most self time first, alone and its hot functions
//!    alone, the same way;
//! 3. then the regions that won, together, once more: one build is
//!    reserved for this check before any region is measured;
//! 4. then the winner without each of its members in turn, least self time
//!    first, and without those whose removal did better at all, together,
//!    at most [`MAX_PRUNE_BUILDS`] builds more: a removal is kept only when
//!    it beats the winner as a larger set must beat a smaller one
//!    ([`prunes`], `prune.rs`).
//!
//! A check is decided by its instruction count when there is one, else by
//! its wall times ([`verdict`]: [`Check::faster`], [`replaces`]). The
//! selection is exactly one set measured as a whole that won: the smallest,
//! unless a larger one beats it by more than noise ([`replaces`],
//! [`decide`]). It is never a union of sets measured apart, so a region
//! pulls a cold function in only when that pays, and when the regions
//! together cannot be measured, or lose, the best single winner is kept.
//!
//! A win inside the noise adds size and risk for no gain, so it is not
//! kept. At most [`MAX_BUILDS`] builds before pruning. A function a check does not find
//! faster, or one no check could reach within the budget, is not selected;
//! the selection rule rejects it from then on, in `--check` and in builds
//! alike. A later `pycc profile` keeps every verdict, a win as well as a
//! rejection, while the function's source hash is unchanged
//! (`carry_verdicts`), and measures only the candidates without one, so a
//! re-run never flips a recorded verdict.

use super::{Driver, FunctionKey, Measured, Profile};
use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

mod callgrind;
mod copy;
mod prune;
pub(crate) use copy::copy_package;
mod verdict;
pub(crate) use verdict::{
    clear, counts_instructions, lighter, prunes, quartiles, replaces, Check, Instructions, Signal, EARLY_ROUNDS, MAX_ROUNDS, MIN_GAIN_PPM,
    PROBE_RUNS,
};

/// The most builds pass 3 makes.
pub(crate) const MAX_BUILDS: usize = 8;

/// The most builds pass 3's pruning step makes beyond [`MAX_BUILDS`]: one
/// per member it tries to remove from the winning set, the one with the
/// least self time first, and one reserved for the members removed
/// together. So a set of up to this many members less one has every member
/// tried, and a larger one leaves its members with the most self time
/// untried.
pub(crate) const MAX_PRUNE_BUILDS: usize = 8;

/// Pass 3's settings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Options {
    /// Interleaved rounds per check (each one run in CPython and one compiled).
    pub(crate) rounds: u32,
    /// What decides the checks (`--verify-signal`).
    pub(crate) signal: Signal,
    /// Files and directories the copy holds besides the project
    /// (`--verify-include`, [`copy_package`]).
    pub(crate) include: Vec<PathBuf>,
}

/// What pass 3 works with.
pub(super) struct Context<'a> {
    pub(super) driver: &'a Driver<'a>,
    /// The modules kept in Python (`--exclude`), which the winning set's
    /// key records.
    pub(super) excludes: &'a [String],
    pub(super) workload: &'a [OsString],
    pub(super) scratch: &'a Path,
    pub(super) options: Options,
}

/// Copies each verdict a previous profile `old` recorded onto the entry of
/// `profile` with the same module, qualified name and source hash: a function
/// measured keeps its verdict, kept or rejected, until its source changes. A
/// function a build refused is not carried: the next pycc may compile it.
pub(crate) fn carry_verdicts(profile: &mut Profile, old: &Profile) {
    let rejected: BTreeMap<(&str, &str, &str), &Measured> = old
        .functions
        .iter()
        .filter_map(|e| {
            e.measured
                .as_ref()
                .filter(|m| m.refused.is_none())
                .map(|m| ((e.module.as_str(), e.qualname.as_str(), e.sha256.as_str()), m))
        })
        .collect();
    for e in &mut profile.functions {
        if let Some(m) = rejected.get(&(e.module.as_str(), e.qualname.as_str(), e.sha256.as_str())) {
            e.measured = Some((*m).clone());
        }
    }
}

/// The candidates connected by calls among them, each region in order of its
/// self time (most first).
pub(crate) fn regions(profile: &Profile, candidates: &[FunctionKey]) -> Vec<Vec<FunctionKey>> {
    let keys: BTreeMap<String, usize> = candidates
        .iter()
        .enumerate()
        .map(|(i, (m, q, l))| (format!("{m}:{q}:{l}"), i))
        .collect();
    let mut parent: Vec<usize> = (0..candidates.len()).collect();
    fn find(parent: &mut [usize], i: usize) -> usize {
        let mut i = i;
        while parent[i] != i {
            parent[i] = parent[parent[i]];
            i = parent[i];
        }
        i
    }
    let mut self_ns = vec![0u64; candidates.len()];
    for e in &profile.functions {
        let Some(&i) = keys.get(&e.callee_key()) else {
            continue;
        };
        self_ns[i] = e.self_ns;
        for callee in e.callees.keys() {
            if let Some(&j) = keys.get(callee) {
                let (a, b) = (find(&mut parent, i), find(&mut parent, j));
                parent[a.max(b)] = a.min(b);
            }
        }
    }
    let mut by_root: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
    for i in 0..candidates.len() {
        let root = find(&mut parent, i);
        by_root.entry(root).or_default().push(i);
    }
    let mut out: Vec<(u64, Vec<FunctionKey>)> = by_root
        .into_values()
        .map(|members| {
            let ns = members.iter().map(|&i| self_ns[i]).sum();
            (ns, members.into_iter().map(|i| candidates[i].clone()).collect())
        })
        .collect();
    out.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
    out.into_iter().map(|(_, r)| r).collect()
}

/// The file in pass 3's scratch directory holding the profile its builds
/// read observed types from (`--types-profile`).
const TYPES_FILE: &str = "types.json";

/// Pass 3 over `candidates` (the selection rule's picks from `profile`):
/// records a [`Measured`] on each. `core` holds the candidates hot on their
/// own (at least the rule's share and calls): a region pulls its colder
/// functions in only when that beats compiling its hot ones alone.
pub(super) fn verify(
    profile: &mut Profile,
    candidates: &[FunctionKey],
    core: &BTreeSet<FunctionKey>,
    cx: &Context<'_>,
) -> Result<(), String> {
    // The selection is frozen here: from now on only the candidates'
    // measured winners are selected, never a function that would fill the
    // slot of a rejected one.
    profile.verified = true;
    // The observed types every check's build specializes its compiled
    // bodies for, as the build that reads this profile will.
    std::fs::write(cx.scratch.join(TYPES_FILE), profile.to_json()).map_err(|e| e.to_string())?;
    // The winning set an earlier profile measured, when it is still whole
    // (every member unchanged): every set measured now includes it, so the
    // selection stays one set measured as a whole. A broken one is
    // measured again.
    let winners: Vec<usize> = (0..profile.functions.len())
        .filter(|&i| profile.functions[i].measured.as_ref().is_some_and(|m| m.compiled))
        .collect();
    let whole = winners.len()
        == winners
            .first()
            .and_then(|&i| profile.functions[i].measured.as_ref())
            .map_or(0, |m| m.with);
    if !whole {
        for &i in &winners {
            profile.functions[i].measured = None;
        }
    }
    let base: BTreeMap<FunctionKey, Measured> = profile
        .functions
        .iter()
        .filter_map(|e| {
            let m = e.measured.as_ref().filter(|m| m.compiled)?;
            Some(((e.module.clone(), e.qualname.clone(), e.firstlineno), m.clone()))
        })
        .collect();
    // A candidate with a verdict carried from an earlier profile keeps it.
    let recorded: BTreeSet<FunctionKey> = profile
        .functions
        .iter()
        .filter(|e| e.measured.is_some())
        .map(|e| (e.module.clone(), e.qualname.clone(), e.firstlineno))
        .collect();
    let carried = candidates.iter().filter(|key| recorded.contains(*key)).count();
    let mut candidates: Vec<FunctionKey> = candidates.iter().filter(|key| !recorded.contains(*key)).cloned().collect();
    // Most self time first: the pruning step tries the members with the
    // least evidence first.
    let self_ns: BTreeMap<FunctionKey, u64> = profile
        .functions
        .iter()
        .map(|e| ((e.module.clone(), e.qualname.clone(), e.firstlineno), e.self_ns))
        .collect();
    candidates.sort_by(|a, b| self_ns.get(b).cmp(&self_ns.get(a)).then_with(|| a.cmp(b)));
    if carried > 0 {
        eprintln!("pycc profile: pass 3: {carried} candidates keep the verdicts an earlier profile recorded");
    }
    if candidates.is_empty() {
        record_winner(profile, cx.excludes);
        return Ok(());
    }
    let started = Instant::now();
    let baseline = baseline(cx)?;
    let regions = regions(profile, &candidates);
    let mut run = |set: &BTreeSet<FunctionKey>, builds: usize| check(set, cx, builds, baseline);
    let (mut verdicts, builds) = decide(&candidates, &regions, core, &base, MAX_BUILDS, MAX_PRUNE_BUILDS, &mut run)?;
    let kept = verdicts.values().filter(|m| m.compiled).count();
    for e in &mut profile.functions {
        if let Some(m) = verdicts.remove(&(e.module.clone(), e.qualname.clone(), e.firstlineno)) {
            e.measured = Some(m);
        }
    }
    eprintln!(
        "pycc profile: pass 3 kept {kept} of {} candidates, measured in {builds} builds in {:.1}s",
        candidates.len(),
        started.elapsed().as_secs_f64()
    );
    record_winner(profile, cx.excludes);
    Ok(())
}

/// CPython's count of the training workload's instructions, for the whole
/// pass, and the deadline of each check's callgrind run.
#[derive(Debug, Clone, Copy)]
struct Baseline {
    instructions: u64,
    deadline: Duration,
}

/// CPython's instructions for the training workload, once for the whole
/// pass, when its checks count them ([`Signal`]): under `auto`, when the
/// median of [`PROBE_RUNS`] CPython runs is short enough
/// ([`counts_instructions`]) and callgrind runs the workload within its
/// deadline ([`callgrind::baseline_deadline`] of that median); else `None`,
/// and the checks are decided on wall time.
fn baseline(cx: &Context<'_>) -> Result<Option<Baseline>, String> {
    let signal = cx.options.signal;
    if signal == Signal::Wall {
        return Ok(None);
    }
    let out = cx.scratch.join("baseline.json");
    // Under `instructions` too: the median sets the callgrind run's deadline.
    let mut wall = Vec::new();
    for _ in 0..PROBE_RUNS {
        wall.push(time(cx.driver, &out, cx.workload)?);
    }
    let median = quartiles(&wall)[1];
    if !counts_instructions(signal, median) {
        eprintln!(
            "pycc profile: pass 3 decides on wall time: CPython runs the workload in {:.0} ms, too long to count its instructions under callgrind",
            median as f64 / 1e6
        );
        return Ok(None);
    }
    match callgrind::instructions(cx.driver, &out, cx.workload, callgrind::baseline_deadline(median)) {
        Ok((n, took)) => {
            eprintln!(
                "pycc profile: pass 3 decides on instruction counts: CPython runs the workload in {n} instructions ({:.1} s under callgrind)",
                took.as_secs_f64()
            );
            Ok(Some(Baseline {
                instructions: n,
                deadline: callgrind::check_deadline(took),
            }))
        }
        Err(why) if signal == Signal::Auto => {
            let why = why.lines().next().unwrap_or_default();
            eprintln!("pycc profile: pass 3 decides on wall time: its instructions cannot be counted ({why})");
            Ok(None)
        }
        Err(why) => Err(why),
    }
}

/// A check's instructions from CPython's count and the compiled copy's
/// callgrind run (`None` when the pass does not count). Under `auto`, a
/// compiled copy callgrind cannot run (a deadline its slowdown misses, its
/// memory, its callgrind run's own deadline) leaves the check to its wall
/// times, as a CPython run that cannot be counted leaves the whole pass
/// ([`baseline`]); under `instructions` it is an error.
pub(super) fn counted(
    signal: Signal,
    run: Option<(u64, Result<u64, String>)>,
) -> Result<Option<Instructions>, String> {
    match run {
        None => Ok(None),
        Some((cpython, Ok(compiled))) => Ok(Some(Instructions { cpython, compiled })),
        Some((_, Err(why))) if signal == Signal::Auto => {
            let why = why.lines().next().unwrap_or_default();
            eprintln!("pycc profile: pass 3: this check decides on wall time: its instructions cannot be counted ({why})");
            Ok(None)
        }
        Some((_, Err(why))) => Err(why),
    }
}

/// The verdict a check records on each function it compiled.
fn measured(c: &Check, compiled: bool, with: usize) -> Measured {
    Measured {
        compiled,
        with,
        cpython_ns: c.cpython,
        compiled_ns: c.compiled,
        instructions: c.instructions.map(|i| -i.saved()),
        cpython_instructions: c.instructions.map(|i| i.cpython),
        refused: None,
        without: None,
        without_instructions: None,
    }
}

/// The check a recorded verdict carries.
fn carried_check(m: &Measured) -> Check {
    Check {
        cpython: m.cpython_ns,
        compiled: m.compiled_ns,
        instructions: m.instructions.zip(m.cpython_instructions).map(|(delta, cpython)| Instructions {
            cpython,
            compiled: (cpython as i64 + delta).max(0) as u64,
        }),
    }
}

/// Records the winning set's key on `profile` (its members, now marked
/// compiled, with their sources; `excludes`; the interpreter's version).
fn record_winner(profile: &mut Profile, excludes: &[String]) {
    let members = profile
        .functions
        .iter()
        .filter(|e| e.measured.as_ref().is_some_and(|m| m.compiled))
        .map(|e| (e.module.clone(), e.qualname.clone(), e.firstlineno, e.sha256.clone()))
        .collect();
    profile.winner = Some(super::Winner {
        members,
        excludes: super::normalized(excludes),
        python: profile.python.clone(),
    });
}

/// The outcome of a check's build and timing: the check, or the functions
/// the build refused, each with why.
pub(super) type Outcome = Result<Result<Check, BTreeMap<FunctionKey, String>>, String>;

/// One check of a set: its build and timing (`check`). Its second argument
/// is the number of the build.
pub(super) type Run<'r> = dyn FnMut(&BTreeSet<FunctionKey>, usize) -> Outcome + 'r;

/// Pass 3's decision over `candidates` (in `regions`, most self time
/// first) within `budget` builds, each check made by `run`: a verdict for
/// every candidate, and the builds made. `base` is the winning set an
/// earlier profile measured, with its verdicts: every set measured now
/// includes it, and with no new winner it stays the selection.
///
/// The invariant: the functions selected are exactly one set that was
/// measured as a whole and won (its compiled median under CPython's first
/// quartile): the smallest, unless a larger one is faster beyond the noise
/// ([`replaces`]); never a union of sets each measured alone. The sets measured are all the
/// candidates and their hot core; when neither wins, each region and its
/// hot core, most self time first; then the regions that won, together.
/// One build of the budget is reserved for that last check before any
/// region is measured. When it cannot run, or loses, the best set that won
/// as a whole is selected: one region, a core, or nothing. Structurally:
/// the measurements form an append-only list ([`Measurement`]) and the
/// selection is one valid entry of it, whole, or nothing; a refusal
/// invalidates every entry holding the refused function, the carried set
/// included, and a build that compiles anything it was not asked for
/// ([`refusals`]) is never timed.
///
/// The set so selected is then pruned ([`Pass::prune`]) within `prune`
/// builds more: its members are removed one at a time, the one with the
/// least self time first (`candidates` come most self time first), then
/// those whose removal did better, together; a removal is kept only when
/// the set without the members beats the winner ([`prunes`]). Every pruned
/// set is measured as a whole too, so the invariant holds; at most
/// `budget + prune` builds in all.
pub(super) fn decide(
    candidates: &[FunctionKey],
    regions: &[Vec<FunctionKey>],
    core: &BTreeSet<FunctionKey>,
    base: &BTreeMap<FunctionKey, Measured>,
    budget: usize,
    prune: usize,
    run: &mut Run<'_>,
) -> Result<(BTreeMap<FunctionKey, Measured>, usize), String> {
    let mut pass = Pass::new(run, base);
    let all: BTreeSet<FunctionKey> = candidates.iter().cloned().collect();
    if pass.best_of(&all, core, budget)?.is_none() && regions.len() > 1 {
        // The build for the regions that win, together.
        let limit = budget.saturating_sub(1);
        let mut union: BTreeSet<FunctionKey> = BTreeSet::new();
        let mut won = 0;
        for region in regions {
            if pass.builds >= limit {
                break;
            }
            let set: BTreeSet<FunctionKey> = region.iter().cloned().collect();
            if let Some(winner) = pass.best_of(&set, core, limit)? {
                union.extend(pass.entries[winner].set.iter().cloned());
                won += 1;
            }
        }
        if won > 1 {
            pass.measure(union, budget)?;
        }
    }
    let best = pass.best();
    let selected = pass.prune(best, candidates, prune)?;
    Ok(pass.finish(selected))
}

/// One set pass 3 measured, as a whole: the selection is one of these,
/// whole, or nothing.
#[derive(Debug, Clone)]
struct Measurement {
    set: BTreeSet<FunctionKey>,
    /// Its times; `None` when no build compiled exactly it, so it was never
    /// timed.
    check: Option<Check>,
    /// Whether it can still be selected: it was timed and won, and no
    /// member of it was refused since (a refusal makes it invalid whole),
    /// nor is it a removal the pruning step measured and did not keep.
    valid: bool,
}

/// Pass 3's builds, verdicts and its measurements: an append-only list.
struct Pass<'r, 's> {
    run: &'r mut Run<'s>,
    /// The earlier profile's winning set, carried as the first measurement
    /// when there is one; while it is valid, every set measured includes
    /// it.
    carried: Option<usize>,
    builds: usize,
    /// Per function: its verdict from the last set it was measured in.
    verdicts: BTreeMap<FunctionKey, Measured>,
    entries: Vec<Measurement>,
    /// Per function, the last measurement it was timed in.
    last: BTreeMap<FunctionKey, usize>,
    /// Every function a build refused: never asked for again, and no
    /// measurement holding it is selected.
    refused: BTreeSet<FunctionKey>,
}

impl<'r, 's> Pass<'r, 's> {
    fn new(run: &'r mut Run<'s>, base: &BTreeMap<FunctionKey, Measured>) -> Self {
        let mut pass = Pass {
            run,
            carried: None,
            builds: 0,
            verdicts: base.clone(),
            entries: Vec::new(),
            last: BTreeMap::new(),
            refused: BTreeSet::new(),
        };
        // Its members share one check: that check's times.
        if let Some(m) = base.values().next() {
            pass.carried = Some(0);
            pass.entries.push(Measurement {
                set: base.keys().cloned().collect(),
                check: Some(carried_check(m)),
                valid: true,
            });
        }
        pass
    }

    /// The carried set's members, while it is valid.
    fn base(&self) -> BTreeSet<FunctionKey> {
        match self.carried {
            Some(i) if self.entries[i].valid => self.entries[i].set.clone(),
            _ => BTreeSet::new(),
        }
    }

    /// Checks `set` and, when its hot functions (`set` and `core`) are not
    /// all of it, those alone, within `limit` builds: of the two that won,
    /// the hot ones, unless the whole set is faster beyond the noise
    /// ([`replaces`]); or None when neither won.
    fn best_of(
        &mut self,
        set: &BTreeSet<FunctionKey>,
        core: &BTreeSet<FunctionKey>,
        limit: usize,
    ) -> Result<Option<usize>, String> {
        let Some(whole) = self.measure(set.clone(), limit)? else {
            return Ok(None);
        };
        let hot: BTreeSet<FunctionKey> = self.entries[whole].set.intersection(core).cloned().collect();
        let base = self.base();
        let new_hot = hot.difference(&base).count();
        if new_hot == 0 || hot.len() == self.entries[whole].set.len() {
            return Ok(self.winning(whole).then_some(whole));
        }
        let alone = self.measure(hot, limit)?;
        let won = |i: usize| self.winning(i).then(|| self.entries[i].check.as_ref()).flatten();
        Ok(match (won(whole), alone.and_then(|i| won(i).map(|c| (i, c)))) {
            // The hot functions are a subset of the whole set: it replaces
            // them only when it wins beyond the noise.
            (Some(w), Some((i, a))) if replaces(w, self.entries[whole].set.len(), a, self.entries[i].set.len()) => {
                Some(whole)
            }
            (_, Some(_)) => alone,
            (Some(_), None) => Some(whole),
            (None, None) => None,
        })
    }

    /// Whether measurement `i` won and is still valid.
    fn winning(&self, i: usize) -> bool {
        self.entries[i].valid
    }

    /// Checks `set` (with the carried set, while valid) within `limit`
    /// builds. A build that leaves a member bytecode refuses it (recorded,
    /// never timed): it is left out and the rest built again, and every
    /// measurement holding it becomes invalid. A build that compiles
    /// anything it was not asked to refuses the measurement. The
    /// measurement timed, or None.
    fn measure(&mut self, mut set: BTreeSet<FunctionKey>, limit: usize) -> Result<Option<usize>, String> {
        let base = self.base();
        set.retain(|key| !base.contains(key) && !self.refused.contains(key));
        while !set.is_empty() && self.builds < limit {
            self.builds += 1;
            let whole: BTreeSet<FunctionKey> = set.iter().chain(&self.base()).cloned().collect();
            match (self.run)(&whole, self.builds)? {
                Ok(c) => {
                    let i = self.entries.len();
                    let with = whole.len();
                    for key in &whole {
                        self.last.insert(key.clone(), i);
                        self.verdicts.insert(key.clone(), measured(&c, c.faster(), with));
                    }
                    let valid = c.faster() && whole.is_disjoint(&self.refused);
                    self.entries.push(Measurement { set: whole, check: Some(c), valid });
                    return Ok(Some(i));
                }
                Err(refused) if refused.keys().any(|key| !whole.contains(key)) => {
                    // Not the set asked for: never timed, never selected.
                    let extra: Vec<String> = refused
                        .keys()
                        .filter(|key| !whole.contains(*key))
                        .map(|key| format!("{}:{}", key.0, key.1))
                        .collect();
                    eprintln!(
                        "pycc profile: pass 3: the build compiled {} it was not asked for: not timed",
                        extra.join(", ")
                    );
                    self.entries.push(Measurement { set: whole, check: None, valid: false });
                    return Ok(None);
                }
                Err(refused) => {
                    let with = whole.len();
                    for (key, reason) in refused {
                        self.refused.insert(key.clone());
                        for entry in &mut self.entries {
                            if entry.set.contains(&key) {
                                entry.valid = false;
                            }
                        }
                        set.remove(&key);
                        self.verdicts.insert(key, Measured::refused(with, reason));
                    }
                    // A carried set refused a member is invalid whole: it is
                    // no longer in the sets measured from now on.
                }
            }
        }
        Ok(None)
    }

    /// The best of the measurements still valid (each won as a whole):
    /// taken smallest first (the first measured among sets of one size), a
    /// set replaces the one selected so far only when it beats it by more
    /// than noise ([`replaces`]). None when nothing won.
    fn best(&self) -> Option<usize> {
        let mut winners: Vec<usize> = (0..self.entries.len())
            .filter(|&i| self.entries[i].valid && self.entries[i].check.is_some())
            .collect();
        // Stable: among sets of one size, the first measured comes first.
        winners.sort_by_key(|&i| self.entries[i].set.len());
        winners.into_iter().reduce(|best, next| {
            let (b, n) = (&self.entries[best], &self.entries[next]);
            match (n.check.as_ref(), b.check.as_ref()) {
                (Some(nc), Some(bc)) if replaces(nc, n.set.len(), bc, b.set.len()) => next,
                _ => best,
            }
        })
    }

    /// The selection, settled: measurement `selected` is compiled, whole, on
    /// its own check; every other function measured is not. One that the
    /// selection left out of a set it was measured in (the selection is part
    /// of that set), or whose own set won too, records the selection's times
    /// (`without`, and `without_instructions` when both were counted). The
    /// verdicts and the builds made.
    fn finish(self, selected: Option<usize>) -> (BTreeMap<FunctionKey, Measured>, usize) {
        let best = selected.and_then(|i| self.entries[i].check.as_ref().map(|c| (&self.entries[i], c)));
        let mut verdicts = self.verdicts;
        for (key, m) in &mut verdicts {
            if m.refused.is_some() {
                continue;
            }
            match best {
                Some((set, c)) if set.set.contains(key) => {
                    *m = measured(c, true, set.set.len());
                }
                Some((set, c)) => {
                    let within = self.last.get(key).is_some_and(|&i| set.set.is_subset(&self.entries[i].set));
                    if m.compiled || within {
                        m.without = Some(c.compiled);
                        m.without_instructions =
                            c.instructions.filter(|_| m.instructions.is_some()).map(|i| -i.saved());
                    }
                    m.compiled = false;
                }
                None => m.compiled = false,
            }
        }
        (verdicts, self.builds)
    }
}

/// Builds exactly `set` into a copy of the package and times the workload
/// against it and against the sources, and, given CPython's instruction
/// count and the deadline of the compiled copy's callgrind run (`baseline`),
/// counts the compiled copy's; or, when the build leaves
/// a function of `set` bytecode, those functions and why, untimed.
fn check(set: &BTreeSet<FunctionKey>, cx: &Context<'_>, builds: usize, baseline: Option<Baseline>) -> Outcome {
    let dir = cx.scratch.join(format!("verify{builds}"));
    let copy = copy_package(cx.driver.pkg, &dir.join("tree"), &cx.options.include)?;
    let refused = build(&copy, set, cx)?;
    if !refused.is_empty() {
        let _ = std::fs::remove_dir_all(&dir);
        return Ok(Err(refused));
    }
    let compiled = Driver {
        python: cx.driver.python,
        script: cx.driver.script,
        pkg: &copy,
        origin: Some(cx.driver.pkg),
    };
    let out = dir.join("time.json");
    // One discarded run of each first: the copy's bytecode cache is written
    // then, as an installed package's is at install.
    time(cx.driver, &out, cx.workload)?;
    time(&compiled, &out, cx.workload)?;
    let (mut plain, mut built) = (Vec::new(), Vec::new());
    for round in 0..cx.options.rounds.clamp(1, MAX_ROUNDS) {
        // Alternating which goes first, against drift.
        let order: [(&Driver, bool); 2] = if round % 2 == 0 {
            [(cx.driver, false), (&compiled, true)]
        } else {
            [(&compiled, true), (cx.driver, false)]
        };
        for (driver, is_compiled) in order {
            let ns = time(driver, &out, cx.workload)?;
            if is_compiled { &mut built } else { &mut plain }.push(ns);
        }
        if round + 1 >= EARLY_ROUNDS && clear(&plain, &built) {
            break;
        }
    }
    let instructions = counted(
        cx.options.signal,
        baseline.map(|b| {
            let run = callgrind::instructions(&compiled, &out, cx.workload, b.deadline);
            (b.instructions, run.map(|(n, _)| n))
        }),
    )?;
    let c = Check {
        cpython: quartiles(&plain),
        compiled: quartiles(&built),
        instructions,
    };
    let ms = |ns: u64| ns as f64 / 1e6;
    let counted = c.instructions.map_or(String::new(), |i| {
        format!(
            "; {:+.2}% instructions ({} against {})",
            -i.saved() as f64 * 100.0 / i.cpython.max(1) as f64,
            i.compiled,
            i.cpython
        )
    });
    eprintln!(
        "pycc profile: pass 3: {} functions compiled: {:.1} ms (IQR {:.1}-{:.1}) against CPython's {:.1} ms (IQR {:.1}-{:.1}){counted}: {}",
        set.len(),
        ms(c.compiled[1]),
        ms(c.compiled[0]),
        ms(c.compiled[2]),
        ms(c.cpython[1]),
        ms(c.cpython[0]),
        ms(c.cpython[2]),
        if c.faster() { "faster" } else { "not faster" }
    );
    // The copy is not needed past the check.
    let _ = std::fs::remove_dir_all(&dir);
    Ok(Ok(c))
}

/// One run of the workload, its wall time; its output is kept from the
/// terminal, and shown when it fails.
fn time(driver: &Driver<'_>, out: &Path, workload: &[OsString]) -> Result<u64, String> {
    let output = driver
        .command("time", out, &[], workload, &[])
        .env("PYCC_REQUIRE_PATCH", "1")
        .output()
        .map_err(|e| format!("cannot run {}: {e}", driver.python.to_string_lossy()))?;
    if !output.status.success() {
        unpatchable(&output.stderr)?;
        return Err(format!(
            "the profile's time pass failed ({}):\n{}{}",
            output.status,
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    #[derive(serde::Deserialize)]
    struct Timed {
        ns: u64,
    }
    Ok(Driver::json::<Timed>(out)?.ns)
}

/// The error for a timed run whose compiled copy could not patch its
/// functions (`patch.c`, under `PYCC_REQUIRE_PATCH`): its verdicts would
/// compare bytecode with bytecode, so none is recorded.
fn unpatchable(stderr: &[u8]) -> Result<(), String> {
    let stderr = String::from_utf8_lossy(stderr);
    if let Some(line) = stderr.lines().find(|line| line.contains("cannot be patched on this interpreter")) {
        return Err(format!(
            "pass 3 cannot measure the compiled functions with this interpreter: {}\n\
             (profile with CPython 3.13 or 3.15, or pass --no-verify)",
            line.trim()
        ));
    }
    if let Some(line) = stderr.lines().find(|line| line.contains("selected functions could not be patched")) {
        return Err(format!(
            "pass 3 cannot measure the compiled functions: {}\n\
             (a pycc bug: please report it; --no-verify skips the pass)",
            line.trim()
        ));
    }
    Ok(())
}

/// `pycc build COPY --ext --non-strict --release --select none --compile
/// MODULE:QUALNAME ...` of exactly `set`, by this pycc: the functions of
/// `set` it left bytecode, each with why. The build reports what it
/// compiled (`--report-fallbacks`), so a function it refused is never timed
/// as compiled.
fn build(
    copy: &Path,
    set: &BTreeSet<FunctionKey>,
    cx: &Context<'_>,
) -> Result<BTreeMap<FunctionKey, String>, String> {
    let out = if copy.is_dir() {
        copy.join("__init__")
    } else {
        copy.with_extension("")
    };
    // A module of a package imports its siblings as CPython does.
    let member = copy.is_file() && copy.parent().is_some_and(|dir| dir.join("__init__.py").is_file());
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let mut command = std::process::Command::new(exe);
    command
        .arg("build")
        .arg(copy)
        .arg("-o")
        .arg(&out)
        .args(["--ext", "--non-strict", "--release", "--select", "none", "--report-fallbacks", "--no-project-select"])
        .arg("--types-profile")
        .arg(cx.scratch.join(TYPES_FILE));
    // Exactly `set`: no selection rule runs again, so leaving the other
    // candidates out changes nothing else.
    for (module, qualname, _) in set {
        command.arg("--compile").arg(format!("{module}:{qualname}"));
    }
    if member {
        command.arg("--foreign-relative-imports");
    }
    if std::env::var_os("PYCC_PYTHON").is_none() {
        command.env("PYCC_PYTHON", cx.driver.python);
    }
    let output = command.output().map_err(|e| e.to_string())?;
    if !output.status.success() {
        return Err(format!(
            "pass 3 could not build the candidates ({}):\n{}{}",
            output.status,
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    Ok(refusals(set, &String::from_utf8_lossy(&output.stderr)))
}

/// How a build whose report is `report` differs from `set`, the set it was
/// asked to compile, compared as sets: each function of `set` it did not
/// compile, with the reason it gives (`not compiled: MODULE:QUALNAME:
/// REASON`, or none), and each function it compiled that is not in `set`
/// (at line 0, as its report has no line). Empty when it compiled exactly
/// `set`.
pub(crate) fn refusals(set: &BTreeSet<FunctionKey>, report: &str) -> BTreeMap<FunctionKey, String> {
    let mut compiled: BTreeSet<&str> = BTreeSet::new();
    let mut reasons: BTreeMap<&str, &str> = BTreeMap::new();
    for line in report.lines() {
        let line = line.trim_end_matches('\r');
        if let Some(name) = line.strip_prefix("compiled: ") {
            compiled.insert(name);
        } else if let Some(rest) = line.strip_prefix("not compiled: ") {
            // MODULE:QUALNAME, then ": " and the reason; a qualified name
            // has no ':'.
            if let Some((module, rest)) = rest.split_once(':')
                && let Some((qualname, reason)) = rest.split_once(": ")
            {
                let name = &line["not compiled: ".len()..][..module.len() + 1 + qualname.len()];
                reasons.entry(name).or_insert(reason);
            }
        }
    }
    let asked: BTreeSet<String> = set.iter().map(|key| format!("{}:{}", key.0, key.1)).collect();
    let extras = compiled.iter().filter(|name| !asked.contains(**name)).filter_map(|name| {
        let (module, qualname) = name.split_once(':')?;
        let key = (module.to_string(), qualname.to_string(), 0);
        Some((key, "compiled though pass 3 did not ask for it".to_string()))
    });
    set.iter()
        .filter_map(|key| {
            let name = format!("{}:{}", key.0, key.1);
            if compiled.contains(name.as_str()) {
                return None;
            }
            let reason = reasons
                .get(name.as_str())
                .map_or("the build did not compile it", |reason| reason);
            Some((key.clone(), reason.to_string()))
        })
        .chain(extras)
        .collect()
}

#[cfg(test)]
mod tests;
