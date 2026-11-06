//! Pass 3 of `pycc profile` (docs/design/compile-little.md, section 2.3):
//! only measured winners are selected.
//!
//! The selection rule picks candidates from the profile's self times, but a
//! compiled untyped body called from bytecode, or calling back into it, can be
//! slower than CPython's specializing interpreter running it. So the profile
//! builds the candidates and times the training workload with them compiled
//! against the unchanged sources, interleaved over `--verify-rounds` fresh
//! processes each after one discarded warm-up run of each (a check stops after [`EARLY_ROUNDS`] once the two
//! interquartile ranges are apart):
//!
//! 1. all the candidates together, and the hot ones among them (each at
//!    least the rule's share and calls on its own) alone;
//! 2. when neither wins, each region (the candidates connected by calls
//!    among them), most self time first, alone and its hot functions
//!    alone, the same way;
//! 3. then the regions that won, together, once more: one build is
//!    reserved for this check before any region is measured.
//!
//! The selection is exactly one set measured as a whole that won (the
//! compiled median under CPython's first quartile): the one with the lowest
//! median, the smaller on a tie ([`decide`]). It is never a union of sets
//! measured apart, so a region pulls a cold function in only when that
//! pays, and when the regions together cannot be measured, or lose, the
//! best single winner is kept.
//!
//! A win inside CPython's interquartile range is noise, and a function that wins
//! only by noise adds size and risk for no gain, so it is not kept. At most
//! [`MAX_BUILDS`] builds. A function a check does not find faster, or one no
//! check could reach within the budget, is not selected; the
//! selection rule rejects it from then on, in `--check` and in builds alike.
//! A later `pycc profile` keeps every verdict, a win as well as a rejection,
//! while the function's source hash is unchanged (`carry_verdicts`), and
//! measures only the candidates without one, so a re-run never flips a
//! recorded verdict. The verdict is on wall time;
//! `--verify-instructions` adds each check's instruction delta under callgrind
//! as evidence.

use super::{Driver, FunctionKey, Measured, Profile};
use std::collections::{BTreeMap, BTreeSet};
use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};
use std::time::Instant;

/// The most rounds per check.
pub(crate) const MAX_ROUNDS: u32 = 15;
/// Rounds after which a check whose interquartile ranges are apart stops.
pub(crate) const EARLY_ROUNDS: u32 = 5;
/// The most builds pass 3 makes.
pub(crate) const MAX_BUILDS: usize = 8;

/// Pass 3's settings.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Options {
    /// Interleaved rounds per check (each one run in CPython and one compiled).
    pub(crate) rounds: u32,
    /// Also count each check's instructions under callgrind.
    pub(crate) instructions: bool,
}

/// What pass 3 works with.
pub(super) struct Context<'a> {
    pub(super) driver: &'a Driver<'a>,
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

/// First quartile, median and third quartile (each the median of its half).
pub(crate) fn quartiles(samples: &[u64]) -> [u64; 3] {
    let mut v = samples.to_vec();
    v.sort_unstable();
    let median = |v: &[u64]| -> u64 {
        match v.len() {
            0 => 0,
            n if n % 2 == 1 => v[n / 2],
            n => (v[n / 2 - 1] + v[n / 2]) / 2,
        }
    };
    let n = v.len();
    if n < 2 {
        let m = median(&v);
        return [m, m, m];
    }
    [median(&v[..n / 2]), median(&v), median(&v[n.div_ceil(2)..])]
}

/// Whether the two samples' interquartile ranges do not overlap: the verdict
/// will not change with more rounds.
pub(crate) fn clear(plain: &[u64], built: &[u64]) -> bool {
    let (p, b) = (quartiles(plain), quartiles(built));
    b[0] > p[2] || b[2] < p[0]
}

/// The outcome of one check.
#[derive(Debug, Clone)]
pub(super) struct Check {
    cpython: [u64; 3],
    compiled: [u64; 3],
    instructions: Option<i64>,
}

impl Check {
    /// Whether the workload ran faster compiled: the compiled median under
    /// CPython's first quartile, outside the noise of CPython's own runs.
    fn faster(&self) -> bool {
        self.compiled[1] < self.cpython[0]
    }
}

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
    // A candidate with a verdict carried from an earlier profile keeps it.
    let recorded: BTreeSet<FunctionKey> = profile
        .functions
        .iter()
        .filter(|e| e.measured.is_some())
        .map(|e| (e.module.clone(), e.qualname.clone(), e.firstlineno))
        .collect();
    let carried = candidates.iter().filter(|key| recorded.contains(*key)).count();
    let candidates: Vec<FunctionKey> = candidates.iter().filter(|key| !recorded.contains(*key)).cloned().collect();
    if carried > 0 {
        eprintln!("pycc profile: pass 3: {carried} candidates keep the verdicts an earlier profile recorded");
    }
    if candidates.is_empty() {
        return Ok(());
    }
    let started = Instant::now();
    let regions = regions(profile, &candidates);
    let mut run = |set: &BTreeSet<FunctionKey>, builds: usize| check(set, cx, builds);
    let (mut verdicts, builds) = decide(&candidates, &regions, core, MAX_BUILDS, &mut run)?;
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
    Ok(())
}

/// The outcome of a check's build and timing: the check, or the functions
/// the build refused, each with why.
pub(super) type Outcome = Result<Result<Check, BTreeMap<FunctionKey, String>>, String>;

/// One check of a set: its build and timing (`check`). Its second argument
/// is the number of the build.
pub(super) type Run<'r> = dyn FnMut(&BTreeSet<FunctionKey>, usize) -> Outcome + 'r;

/// Pass 3's decision over `candidates` (in `regions`, most self time
/// first) within `budget` builds, each check made by `run`: a verdict for
/// every candidate, and the builds made.
///
/// The invariant: the functions selected are exactly one set that was
/// measured as a whole and won (its compiled median under CPython's first
/// quartile), the one with the lowest median, the smaller on a tie; never
/// a union of sets each measured alone. The sets measured are all the
/// candidates and their hot core; when neither wins, each region and its
/// hot core, most self time first; then the regions that won, together.
/// One build of the budget is reserved for that last check before any
/// region is measured. When it cannot run, or loses, the best set that won
/// as a whole is selected: one region, a core, or nothing.
pub(super) fn decide(
    candidates: &[FunctionKey],
    regions: &[Vec<FunctionKey>],
    core: &BTreeSet<FunctionKey>,
    budget: usize,
    run: &mut Run<'_>,
) -> Result<(BTreeMap<FunctionKey, Measured>, usize), String> {
    let mut pass = Pass {
        run,
        builds: 0,
        verdicts: BTreeMap::new(),
        winners: Vec::new(),
        sets: Vec::new(),
        last: BTreeMap::new(),
    };
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
                union.extend(winner);
                won += 1;
            }
        }
        if won > 1
            && let Some((union, c)) = pass.measure(union, budget)?
        {
            pass.record(&union, c);
        }
    }
    Ok(pass.finish())
}

/// Records `c`'s outcome on each function of `set`.
fn record(verdicts: &mut BTreeMap<FunctionKey, Measured>, set: &BTreeSet<FunctionKey>, c: &Check) {
    for key in set {
        verdicts.insert(
            key.clone(),
            Measured {
                compiled: c.faster(),
                with: set.len(),
                cpython_ns: c.cpython,
                compiled_ns: c.compiled,
                instructions: c.instructions,
                refused: None,
                without: None,
            },
        );
    }
}

/// Pass 3's builds, verdicts and the sets that won so far.
struct Pass<'r, 's> {
    run: &'r mut Run<'s>,
    builds: usize,
    verdicts: BTreeMap<FunctionKey, Measured>,
    /// Each set measured as a whole that was faster than CPython.
    winners: Vec<(BTreeSet<FunctionKey>, Check)>,
    /// Each set measured, and per function the last one it was measured in.
    sets: Vec<BTreeSet<FunctionKey>>,
    last: BTreeMap<FunctionKey, usize>,
}

impl Pass<'_, '_> {
    /// Records `c` on each function of `set`, and `set` as a winner when it
    /// won.
    fn record(&mut self, set: &BTreeSet<FunctionKey>, c: Check) {
        record(&mut self.verdicts, set, &c);
        for key in set {
            self.last.insert(key.clone(), self.sets.len());
        }
        self.sets.push(set.clone());
        if c.faster() {
            self.winners.push((set.clone(), c));
        }
    }

    /// Checks `set` and, when its hot functions (`set` and `core`) are not
    /// all of it, those alone, within `limit` builds: the one of the two
    /// that won with the lower median (the hot ones on a tie), if either
    /// won.
    fn best_of(
        &mut self,
        set: &BTreeSet<FunctionKey>,
        core: &BTreeSet<FunctionKey>,
        limit: usize,
    ) -> Result<Option<BTreeSet<FunctionKey>>, String> {
        let Some((set, whole)) = self.measure(set.clone(), limit)? else {
            return Ok(None);
        };
        let whole_median = whole.faster().then_some(whole.compiled[1]);
        self.record(&set, whole);
        let hot: BTreeSet<FunctionKey> = set.intersection(core).cloned().collect();
        if hot.is_empty() || hot.len() == set.len() {
            return Ok(whole_median.map(|_| set));
        }
        let Some((hot, alone)) = self.measure(hot, limit)? else {
            return Ok(whole_median.map(|_| set));
        };
        let alone_median = alone.faster().then_some(alone.compiled[1]);
        self.record(&hot, alone);
        Ok(match (whole_median, alone_median) {
            (Some(w), Some(a)) if w < a => Some(set),
            (_, Some(_)) => Some(hot),
            (Some(_), None) => Some(set),
            (None, None) => None,
        })
    }

    /// Checks `set` within `limit` builds, leaving out every function a
    /// build refuses (recorded as refused, never timed) until a build
    /// compiles the whole rest: that rest and its check, or None when no
    /// build compiles any of it within the limit.
    fn measure(
        &mut self,
        mut set: BTreeSet<FunctionKey>,
        limit: usize,
    ) -> Result<Option<(BTreeSet<FunctionKey>, Check)>, String> {
        while !set.is_empty() && self.builds < limit {
            self.builds += 1;
            match (self.run)(&set, self.builds)? {
                Ok(c) => return Ok(Some((set, c))),
                Err(refused) => {
                    let with = set.len();
                    for (key, reason) in refused {
                        set.remove(&key);
                        self.verdicts.insert(key, Measured::refused(with, reason));
                    }
                }
            }
        }
        Ok(None)
    }

    /// The verdicts, with the selection settled: the winner with the lowest
    /// median (the smaller on a tie) is compiled, on its own check's times;
    /// every other function measured is not. One that the winner left out of
    /// a set it was measured in (the winner is part of that set), or whose
    /// own set won too, records the winner's times (`without`); any other
    /// keeps its own check's. The builds made.
    fn finish(mut self) -> (BTreeMap<FunctionKey, Measured>, usize) {
        let best = self
            .winners
            .iter()
            .min_by_key(|(set, c)| (c.compiled[1], set.len()))
            .map(|(set, c)| (set.clone(), c.clone()));
        for (key, m) in &mut self.verdicts {
            if m.refused.is_some() {
                continue;
            }
            match &best {
                Some((set, c)) if set.contains(key) => {
                    *m = Measured {
                        compiled: true,
                        with: set.len(),
                        cpython_ns: c.cpython,
                        compiled_ns: c.compiled,
                        instructions: c.instructions,
                        refused: None,
                        without: None,
                    };
                }
                Some((set, c)) => {
                    let within = self.last.get(key).is_some_and(|&i| set.is_subset(&self.sets[i]));
                    if m.compiled || within {
                        m.without = Some(c.compiled);
                    }
                    m.compiled = false;
                }
                None => m.compiled = false,
            }
        }
        (self.verdicts, self.builds)
    }
}

/// Builds exactly `set` into a copy of the package and times the workload
/// against it and against the sources; or, when the build leaves a function
/// of `set` bytecode, those functions and why, untimed.
fn check(
    set: &BTreeSet<FunctionKey>,
    cx: &Context<'_>,
    builds: usize,
) -> Outcome {
    let dir = cx.scratch.join(format!("verify{builds}"));
    let copy = copy_package(cx.driver.pkg, &dir.join("tree"))?;
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
    let instructions = if cx.options.instructions {
        Some(instructions(&compiled, &out, cx.workload)? - instructions(cx.driver, &out, cx.workload)?)
    } else {
        None
    };
    let c = Check {
        cpython: quartiles(&plain),
        compiled: quartiles(&built),
        instructions,
    };
    let ms = |ns: u64| ns as f64 / 1e6;
    eprintln!(
        "pycc profile: pass 3: {} functions compiled: {:.1} ms (IQR {:.1}-{:.1}) against CPython's {:.1} ms (IQR {:.1}-{:.1}): {}",
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

/// One run of the workload under callgrind: its instruction count.
fn instructions(driver: &Driver<'_>, out: &Path, workload: &[OsString]) -> Result<i64, String> {
    let file = format!("--callgrind-out-file={}", out.with_extension("cg").display());
    let wrapper: [&OsStr; 3] = ["valgrind".as_ref(), "--tool=callgrind".as_ref(), file.as_ref()];
    let output = driver
        .command("time", out, &[], workload, &wrapper)
        .env("PYCC_REQUIRE_PATCH", "1")
        .output()
        .map_err(|e| format!("--verify-instructions needs valgrind: {e}"))?;
    let _ = std::fs::remove_file(out.with_extension("cg"));
    let stderr = String::from_utf8_lossy(&output.stderr);
    if !output.status.success() {
        unpatchable(&output.stderr)?;
        return Err(format!("the profile's time pass under callgrind failed ({}):\n{stderr}", output.status));
    }
    stderr
        .lines()
        .find_map(|line| line.split_once("Collected :").and_then(|(_, n)| n.trim().parse().ok()))
        .ok_or_else(|| "callgrind reported no instruction count".to_string())
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
        .args(["--ext", "--non-strict", "--release", "--select", "none", "--report-fallbacks"]);
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

/// The functions of `set` a build whose report is `report` did not compile,
/// each with the reason it gives (`not compiled: MODULE:QUALNAME: REASON`),
/// or that it does not name compiled at all.
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
        .collect()
}

/// Copies the project `pkg` (a package or single module) lives in under
/// `root`, and returns the copy of `pkg`: the whole directory `pkg` is
/// imported from (the parent of its top-level package, or of the module),
/// so the workload's files beside the package (its tests, data,
/// `pyproject.toml`) are found at the same relative paths as in the
/// original. Hidden entries and `__pycache__` are left out, and so is
/// `root` itself when it lies inside the project.
pub(crate) fn copy_package(pkg: &Path, root: &Path) -> Result<PathBuf, String> {
    let mut base = pkg.parent().unwrap_or(Path::new("")).to_path_buf();
    while base.join("__init__.py").is_file() {
        match base.parent() {
            Some(parent) => base = parent.to_path_buf(),
            None => break,
        }
    }
    let rel = pkg.strip_prefix(&base).map_err(|e| e.to_string())?.to_path_buf();
    if base.as_os_str().is_empty() {
        base = PathBuf::from(".");
    }
    std::fs::create_dir_all(root).map_err(|e| e.to_string())?;
    let skip = root.canonicalize().map_err(|e| e.to_string())?;
    copy_dir(&base, root, &skip)?;
    Ok(root.join(rel))
}

fn copy_dir(from: &Path, to: &Path, skip: &Path) -> Result<(), String> {
    std::fs::create_dir_all(to).map_err(|e| e.to_string())?;
    for entry in std::fs::read_dir(from).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        let kind = entry.file_type().map_err(|e| e.to_string())?;
        let name = entry.file_name();
        if name.to_string_lossy().starts_with('.') {
            continue;
        }
        if kind.is_dir() {
            let path = entry.path();
            if name != "__pycache__" && path.canonicalize().map_err(|e| e.to_string())? != skip {
                copy_dir(&path, &to.join(&name), skip)?;
            }
        } else if kind.is_file() {
            std::fs::copy(entry.path(), to.join(&name)).map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;
