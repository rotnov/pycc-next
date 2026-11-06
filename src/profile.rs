//! `pycc profile` (docs/design/compile-little.md, section 2.2): the evidence a
//! non-strict `--ext` build selects the functions it compiles from.
//!
//! `pycc profile PKG -o pycc.profile.json -- PYTHON SCRIPT [ARGS]` runs SCRIPT
//! with PYTHON against PKG's sources, nothing compiled, through the driver
//! (`profile/driver.py`):
//!
//! 1. `--runs` (default 5) fresh processes under `cProfile`. Per function of
//!    the package: its calls and its self time, each the median over the runs,
//!    and the run's total self time (of everything `cProfile` saw), the median
//!    too.
//! 2. One process with a `sys.monitoring` `PY_START` hook on the functions the
//!    selection rule ([`select`]) picks from pass 1: the exact types of their
//!    arguments in their first 1,000 calls, and how many caught an exception.
//! 3. Unless `--no-verify`: [`verify`] builds the candidates and times the
//!    workload against them, recording `measured` on each, so only measured
//!    winners are selected.
//!
//! The file holds `version`, `python`, `workload`, `runs`, `total_ns` and one
//! entry per function (`module`, `qualname`, `firstlineno`, the SHA-256 of its
//! source text, `calls`, `self_ns`, `callees`, `arg_types`, `handled`,
//! `measured`), sorted by module, qualified
//! name and line, written by [`Profile::to_json`]: the same samples give the
//! same bytes. A build reads it back with [`Profile::current`], which drops
//! (and names) each entry whose function's source no longer hashes the same.

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::ffi::{OsStr, OsString};
use std::path::Path;

/// The profile format's version.
pub(crate) const VERSION: u32 = 1;

const DRIVER: &str = include_str!("profile/driver.py");

pub(crate) mod verify;

/// Parameter name to exact type name to count.
pub(crate) type ArgTypes = BTreeMap<String, BTreeMap<String, u64>>;

/// One function of the package the workload ran.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Entry {
    pub(crate) module: String,
    pub(crate) qualname: String,
    pub(crate) firstlineno: u32,
    pub(crate) sha256: String,
    pub(crate) calls: u64,
    pub(crate) self_ns: u64,
    /// The calls it made to Python functions, by callee: a function of the
    /// package (`module:qualname:firstlineno`) or `*`, any other.
    #[serde(default)]
    pub(crate) callees: BTreeMap<String, u64>,
    #[serde(default)]
    pub(crate) arg_types: ArgTypes,
    /// Of the calls pass 2 sampled (the first [`TYPE_CALLS`]), how many caught
    /// an exception in the function's own frame.
    #[serde(default)]
    pub(crate) handled: u64,
    /// Pass 2 sampled its calls (it was among the candidates it sampled, and
    /// the workload called it): its `handled` and `arg_types` are evidence.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub(crate) sampled: bool,
    /// Pass 3's verdict (`profile::verify`): whether the workload ran no
    /// slower with this function compiled. Absent when not measured.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) measured: Option<Measured>,
}

/// One pass-3 check's outcome, recorded on each function it compiled.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Measured {
    /// Whether the workload ran faster compiled: the compiled median under
    /// CPython's first quartile.
    pub(crate) compiled: bool,
    /// The functions the check compiled, this one among them.
    pub(crate) with: usize,
    /// The workload's wall time in CPython: first quartile, median, third
    /// quartile, in nanoseconds.
    pub(crate) cpython_ns: [u64; 3],
    /// The same with the functions compiled.
    pub(crate) compiled_ns: [u64; 3],
    /// Instructions of one run compiled minus one in CPython, when
    /// `--verify-instructions` ran it under callgrind: evidence, never the
    /// verdict.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) instructions: Option<i64>,
    /// Why the check's build did not compile this function, when it did
    /// not: it was never timed, and is rejected for this reason.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) refused: Option<String>,
    /// The workload's times (as `compiled_ns`) with the selected set
    /// compiled, when that set leaves this function out of a set it was
    /// measured in, or this function's own set won too: it is rejected,
    /// the selection winning without it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) without: Option<[u64; 3]>,
}

impl Measured {
    /// The verdict on a function the build of a check of `with` functions
    /// did not compile, for `reason`: never timed.
    pub(crate) fn refused(with: usize, reason: String) -> Measured {
        Measured {
            compiled: false,
            with,
            cpython_ns: [0; 3],
            compiled_ns: [0; 3],
            instructions: None,
            refused: Some(reason),
            without: None,
        }
    }

    /// Why a function this check compiled, or would have, is rejected.
    pub(crate) fn rejection(&self) -> String {
        if let Some(reason) = &self.refused {
            return format!("not compiled: {reason}");
        }
        let ms = |ns: u64| ns as f64 / 1e6;
        if let Some(without) = self.without {
            return format!(
                "faster without it: the workload took {:.1} ms with a measured set of candidates that leaves it out compiled, against {:.1} ms with it and {} other functions",
                ms(without[1]),
                ms(self.compiled_ns[1]),
                self.with - 1,
            );
        }
        format!(
            "not faster when compiled: the workload took {:.1} ms with it and {} other functions compiled, against CPython's {:.1} ms (IQR {:.1}-{:.1})",
            ms(self.compiled_ns[1]),
            self.with - 1,
            ms(self.cpython_ns[1]),
            ms(self.cpython_ns[0]),
            ms(self.cpython_ns[2]),
        )
    }
}

/// The calls of a function pass 2 samples.
pub(crate) const TYPE_CALLS: u64 = 1000;

impl Entry {
    /// Its key among callees.
    pub(crate) fn callee_key(&self) -> String {
        format!("{}:{}:{}", self.module, self.qualname, self.firstlineno)
    }
}

/// A profile file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Profile {
    pub(crate) version: u32,
    pub(crate) python: String,
    pub(crate) workload: String,
    pub(crate) runs: u32,
    pub(crate) total_ns: u64,
    pub(crate) functions: Vec<Entry>,
    /// Pass 2 sampled the selection's candidates: only the functions it
    /// sampled are selected ([`Rule::sampled_only`]).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub(crate) sampled: bool,
    /// Pass 3 measured the selection's candidates: only its measured
    /// winners are selected ([`Rule::measured_only`]).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub(crate) verified: bool,
}

/// One function of the package's current sources (the driver's `index`).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub(crate) struct Function {
    pub(crate) module: String,
    pub(crate) qualname: String,
    pub(crate) firstlineno: u32,
    pub(crate) sha256: String,
    /// Its source's length in lines, decorators included.
    #[serde(default)]
    pub(crate) lines: u32,
    /// Module-level functions and methods of module-level classes that are
    /// not generators or coroutines (section 2.1).
    pub(crate) eligible: bool,
}

impl Function {
    /// `entry` (this function's profile entry) as [`select`] decides it.
    pub(crate) fn candidate<'p>(&self, entry: &'p Entry) -> Candidate<'p> {
        Candidate {
            entry,
            eligible: self.eligible,
            lines: self.lines,
        }
    }
}

/// What a call from a compiled function to an interpreted one costs beyond
/// the interpreter calling it itself, in nanoseconds: measured on CPython
/// 3.15.2 (x86-64), a compiled loop calling an identity function took 47.7 ns
/// a call where the interpreter's own loop took 34.1 ns with the call and
/// 22.4 ns without it, so the compiled call costs 47.7 - 11.7 = 36 ns more
/// (the compiled loop alone is under 1 ns).
pub(crate) const CROSSING_NS: u64 = 36;

/// The selection rule's thresholds (section 2.3).
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Rule {
    /// The least share of the run's total self time.
    pub(crate) share: f64,
    /// The fewest calls.
    pub(crate) calls: u64,
    /// The most functions selected, by self time.
    pub(crate) max: usize,
    /// The most source lines selected, by self time: the cap on how far
    /// regions grow (each selected line is compiled code in the `.so`).
    pub(crate) lines: u64,
    /// What one call from a compiled function to a Python function outside
    /// the selection costs beyond the interpreter's own call, in nanoseconds
    /// ([`CROSSING_NS`]).
    pub(crate) crossing_ns: u64,
    /// The share of its sampled calls in which a function catches an
    /// exception must be under this. Exceptions as control flow are slower
    /// compiled than in CPython until PR 9 (section 2.7).
    pub(crate) handled: f64,
    /// Only functions pass 2 sampled are selected: a function that would
    /// fill a slot a rejected candidate left has no `handled` count.
    pub(crate) sampled_only: bool,
    /// Only functions pass 3 measured are selected: a profile whose
    /// candidates were measured keeps exactly its winners, and a function
    /// that would fill a slot a rejected candidate left is not one of them.
    pub(crate) measured_only: bool,
}

impl Default for Rule {
    fn default() -> Self {
        Rule {
            share: 0.005,
            calls: 50,
            max: 64,
            lines: 4000,
            crossing_ns: CROSSING_NS,
            handled: 0.5,
            sampled_only: false,
            measured_only: false,
        }
    }
}

/// What [`select`] decided for one function.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Decision<'p> {
    pub(crate) entry: &'p Entry,
    /// Its share of the run's total self time.
    pub(crate) share: f64,
    /// `None` when selected, else why not.
    pub(crate) rejected: Option<String>,
}

impl Profile {
    /// `rule` as it applies to this profile: a function is selected only
    /// with evidence from every pass that ran (sampled when pass 2 ran,
    /// measured a winner when pass 3 did). A rejection frees a slot but
    /// admits no function a pass did not examine.
    pub(crate) fn rule(&self, rule: Rule) -> Rule {
        Rule {
            sampled_only: self.sampled,
            measured_only: self.verified,
            ..rule
        }
    }

    /// The file's text: pretty JSON, entries sorted, a final newline.
    pub(crate) fn to_json(&self) -> String {
        let mut sorted = self.clone();
        sorted.functions.sort_by(|a, b| {
            (&a.module, &a.qualname, a.firstlineno).cmp(&(&b.module, &b.qualname, b.firstlineno))
        });
        let mut text = serde_json::to_string_pretty(&sorted).expect("a profile serializes");
        text.push('\n');
        text
    }

    /// Reads a profile file.
    pub(crate) fn read(path: &Path) -> Result<Profile, String> {
        let text = std::fs::read_to_string(path)
            .map_err(|e| format!("cannot read the profile {}: {e}", path.display()))?;
        let profile: Profile = serde_json::from_str(&text)
            .map_err(|e| format!("{} is not a pycc profile: {e}", path.display()))?;
        if profile.version != VERSION {
            return Err(format!(
                "{} is a version {} profile; this pycc reads version {VERSION}",
                path.display(),
                profile.version
            ));
        }
        Ok(profile)
    }

    /// The entries whose function still has the source it was profiled with,
    /// each with that function as the sources have it now, and a note per
    /// entry dropped: its function changed or is gone. Functions are matched
    /// by module, qualified name and source hash, so a function that only
    /// moved stays current, and of two `def`s with one qualified name (a
    /// property's getter and setter) each keeps its own entry.
    pub(crate) fn current<'p, 'f>(
        &'p self,
        sources: &'f [Function],
    ) -> (Vec<(&'p Entry, &'f Function)>, Vec<String>) {
        let mut by_name: BTreeMap<(&str, &str), Vec<&Function>> = BTreeMap::new();
        for f in sources {
            by_name
                .entry((f.module.as_str(), f.qualname.as_str()))
                .or_default()
                .push(f);
        }
        let mut current = Vec::new();
        let mut notes = Vec::new();
        for e in &self.functions {
            let same = by_name
                .get(&(e.module.as_str(), e.qualname.as_str()))
                .and_then(|fs| fs.iter().find(|f| f.sha256 == e.sha256));
            match same {
                Some(f) => current.push((e, *f)),
                None => {
                    let why = if by_name.contains_key(&(e.module.as_str(), e.qualname.as_str())) {
                        "its source changed since it was profiled"
                    } else {
                        "it is no longer in the sources"
                    };
                    notes.push(format!(
                        "note: profile entry {}:{} (line {}) dropped: {why}",
                        e.module, e.qualname, e.firstlineno
                    ));
                }
            }
        }
        (current, notes)
    }
}

/// One function [`select`] decides: its current profile entry, whether it is
/// eligible (section 2.1), and its source's length in lines.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Candidate<'p> {
    pub(crate) entry: &'p Entry,
    pub(crate) eligible: bool,
    pub(crate) lines: u32,
}

/// The selection rule (section 2.3) over `entries` (current ones). Every
/// entry is decided, in order of self time (most first; then by name), and a
/// rejection names the first step it failed:
///
/// 1. the function itself: eligible, not measured slower by pass 3, sampled
///    by pass 2 once it ran (`rule.sampled_only`), catching an exception in
///    less than `rule.handled` of its sampled calls;
/// 2. its region, the functions that pass step 1 connected by calls among
///    them, whatever their own heat: together at least `rule.share` of the
///    run's total self time, and one of them called at least `rule.calls`
///    times;
/// 3. its crossings: its calls to Python functions outside the selection
///    (itself and its own nested code are inside), each costing about
///    `rule.crossing_ns` more than the interpreter's own call, must cost less
///    than its self time;
/// 4. the size of the selection: at most `rule.max` functions and
///    `rule.lines` source lines, taken by self time;
/// 5. once pass 3 has run (`rule.measured_only`), measured by it.
///
/// Steps 2 to 4 repeat until no function changes: a rejection can shrink a
/// region below its threshold or turn its callers' calls into crossings.
pub(crate) fn select<'p>(total_ns: u64, entries: &[Candidate<'p>], rule: Rule) -> Vec<Decision<'p>> {
    let mut order: Vec<&Candidate<'p>> = entries.iter().collect();
    order.sort_by(|a, b| {
        let (a, b) = (a.entry, b.entry);
        b.self_ns
            .cmp(&a.self_ns)
            .then_with(|| (&a.module, &a.qualname, a.firstlineno).cmp(&(&b.module, &b.qualname, b.firstlineno)))
    });
    let share = |ns: u64| if total_ns == 0 { 0.0 } else { ns as f64 / total_ns as f64 };
    let percent = |x: f64| format!("{:.2}%", x * 100.0);
    let mut decided: Vec<Decision<'p>> = order
        .iter()
        .map(|c| {
            let entry = c.entry;
            let rejected = if !c.eligible {
                Some("not eligible (a generator, coroutine or nested function)".to_string())
            } else if let Some(m) = entry.measured.as_ref().filter(|m| !m.compiled) {
                Some(m.rejection())
            } else if rule.sampled_only && !entry.sampled {
                Some("not sampled by pass 2 (it was not among the candidates it sampled)".to_string())
            } else if entry.handled > 0
                && entry.handled as f64 >= entry.calls.min(TYPE_CALLS) as f64 * rule.handled
            {
                Some(format!(
                    "it catches an exception in {} of its first {} calls",
                    entry.handled,
                    entry.calls.min(TYPE_CALLS)
                ))
            } else {
                None
            };
            Decision {
                entry,
                share: share(entry.self_ns),
                rejected,
            }
        })
        .collect();
    let lines: Vec<u32> = order.iter().map(|c| c.lines).collect();
    loop {
        let live: Vec<usize> = (0..decided.len()).filter(|&i| decided[i].rejected.is_none()).collect();
        let mut changed = false;
        // Step 2: the regions.
        for region in regions_of(&decided, &live) {
            let ns: u64 = region.iter().map(|&i| decided[i].entry.self_ns).sum();
            let calls = region.iter().map(|&i| decided[i].entry.calls).max().unwrap_or(0);
            let why = if share(ns) < rule.share {
                Some(if region.len() == 1 {
                    format!("self time {} is under {}", percent(share(ns)), percent(rule.share))
                } else {
                    format!(
                        "its region's self time {} ({} functions) is under {}",
                        percent(share(ns)),
                        region.len(),
                        percent(rule.share)
                    )
                })
            } else if calls < rule.calls {
                Some(if region.len() == 1 {
                    format!("{calls} calls is under {}", rule.calls)
                } else {
                    format!(
                        "its region's most called function has {calls} calls, under {}",
                        rule.calls
                    )
                })
            } else {
                None
            };
            if let Some(why) = why {
                for &i in &region {
                    decided[i].rejected = Some(why.clone());
                }
                changed = true;
            }
        }
        if changed {
            continue;
        }
        // Step 3: the crossings.
        let inside: BTreeSet<String> = live.iter().map(|&i| decided[i].entry.callee_key()).collect();
        for &i in &live {
            let e = decided[i].entry;
            let nested = format!("{}:{}.<locals>.", e.module, e.qualname);
            let out: u64 = e
                .callees
                .iter()
                .filter(|(callee, _)| !inside.contains(*callee) && !callee.starts_with(&nested))
                .map(|(_, n)| n)
                .sum();
            let cost = out.saturating_mul(rule.crossing_ns);
            if out > 0 && cost >= e.self_ns {
                decided[i].rejected = Some(format!(
                    "its {out} calls out of the selection would cost about {} ({} ns each), against {} of self time",
                    duration(cost),
                    rule.crossing_ns,
                    duration(e.self_ns)
                ));
                changed = true;
            }
        }
        if changed {
            continue;
        }
        // Step 4: the size.
        let (mut taken, mut taken_lines) = (0usize, 0u64);
        for &i in &live {
            if taken >= rule.max {
                decided[i].rejected = Some(format!("not among the top {} by self time", rule.max));
                changed = true;
            } else if taken_lines + u64::from(lines[i]) > rule.lines {
                decided[i].rejected = Some(format!(
                    "its {} lines would take the selection past {} lines",
                    lines[i], rule.lines
                ));
                changed = true;
            } else {
                taken += 1;
                taken_lines += u64::from(lines[i]);
            }
        }
        if !changed {
            break;
        }
    }
    // Step 5: pass 3's evidence.
    if rule.measured_only {
        for d in decided.iter_mut().filter(|d| d.rejected.is_none()) {
            if d.entry.measured.is_none() {
                d.rejected = Some("not measured by pass 3 (it was not among the candidates it built)".to_string());
            }
        }
    }
    decided
}

/// `ns` for a person: in microseconds under a millisecond, else milliseconds.
fn duration(ns: u64) -> String {
    if ns < 1_000_000 {
        format!("{:.1} us", ns as f64 / 1e3)
    } else {
        format!("{:.1} ms", ns as f64 / 1e6)
    }
}

/// The functions `live` (indexes into `decided`) connected by calls among
/// them, each region in `live`'s order.
fn regions_of(decided: &[Decision<'_>], live: &[usize]) -> Vec<Vec<usize>> {
    let at: BTreeMap<String, usize> = live
        .iter()
        .enumerate()
        .map(|(k, &i)| (decided[i].entry.callee_key(), k))
        .collect();
    let mut parent: Vec<usize> = (0..live.len()).collect();
    fn find(parent: &mut [usize], i: usize) -> usize {
        let mut i = i;
        while parent[i] != i {
            parent[i] = parent[parent[i]];
            i = parent[i];
        }
        i
    }
    for (k, &i) in live.iter().enumerate() {
        for callee in decided[i].entry.callees.keys() {
            if let Some(&j) = at.get(callee) {
                let (a, b) = (find(&mut parent, k), find(&mut parent, j));
                parent[a.max(b)] = a.min(b);
            }
        }
    }
    let mut by_root: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
    for (k, &i) in live.iter().enumerate() {
        let root = find(&mut parent, k);
        by_root.entry(root).or_default().push(i);
    }
    by_root.into_values().collect()
}

/// One pass-1 run: the total self time and, per function key (`module`,
/// `qualname`, `firstlineno`), its calls, self time and calls to Python functions by callee.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub(crate) struct CallsRun {
    pub(crate) total_ns: u64,
    pub(crate) functions: Vec<CallsSample>,
}

/// One function of a [`CallsRun`]: module, qualified name, first line (as the
/// driver prints it), calls, self time, and calls to Python functions by
/// callee.
pub(crate) type CallsSample = (String, String, String, u64, u64, BTreeMap<String, u64>);

fn median(mut v: Vec<u64>) -> u64 {
    v.sort_unstable();
    let n = v.len();
    if n == 0 {
        0
    } else if n % 2 == 1 {
        v[n / 2]
    } else {
        (v[n / 2 - 1] + v[n / 2]) / 2
    }
}

/// A function of a profile: its module, qualified name and first line.
pub(crate) type FunctionKey = (String, String, u32);

/// The medians over pass-1 runs: the total, and per function its calls and
/// self time (a run that did not call a function counts as zero for it).
pub(crate) fn aggregate(runs: &[CallsRun]) -> (u64, BTreeMap<FunctionKey, Aggregated>) {
    let total = median(runs.iter().map(|r| r.total_ns).collect());
    let mut samples: BTreeMap<FunctionKey, Samples> = BTreeMap::new();
    for run in runs {
        for (module, qualname, line, calls, ns, callees) in &run.functions {
            let key = (module.clone(), qualname.clone(), line.parse().unwrap_or(0));
            let s = samples.entry(key).or_default();
            s.calls.push(*calls);
            s.ns.push(*ns);
            for (callee, n) in callees {
                s.callees.entry(callee.clone()).or_default().push(*n);
            }
        }
    }
    let n = runs.len();
    let out = samples
        .into_iter()
        .map(|(k, mut s)| {
            s.calls.resize(n, 0);
            s.ns.resize(n, 0);
            let callees = s
                .callees
                .into_iter()
                .map(|(callee, mut counts)| {
                    counts.resize(n, 0);
                    (callee, median(counts))
                })
                .filter(|(_, n)| *n > 0)
                .collect();
            (
                k,
                Aggregated {
                    calls: median(s.calls),
                    self_ns: median(s.ns),
                    callees,
                },
            )
        })
        .collect();
    (total, out)
}

/// `medians` with the calls to Python functions of each code object that is
/// not a function of `sources` (a generator expression, a lambda) added to
/// its enclosing function's: compiled, that code is part of the function's
/// body. The code object's own entry is dropped.
pub(crate) fn fold_nested_code(
    medians: BTreeMap<FunctionKey, Aggregated>,
    sources: &[Function],
) -> BTreeMap<FunctionKey, Aggregated> {
    let indexed: BTreeSet<(&str, &str, u32)> = sources
        .iter()
        .map(|f| (f.module.as_str(), f.qualname.as_str(), f.firstlineno))
        .collect();
    let (mut kept, nested): (BTreeMap<_, _>, BTreeMap<_, _>) = medians
        .into_iter()
        .partition(|((m, q, l), _)| indexed.contains(&(m.as_str(), q.as_str(), *l)));
    for ((module, qualname, line), aggregated) in nested {
        // The innermost function whose `<locals>` hold it, and of those the
        // last one to start before it.
        let owner = sources
            .iter()
            .filter(|f| {
                f.module == module
                    && f.firstlineno <= line
                    && qualname
                        .strip_prefix(f.qualname.as_str())
                        .is_some_and(|rest| rest.starts_with(".<locals>."))
            })
            .max_by_key(|f| (f.qualname.len(), f.firstlineno));
        let Some(owner) = owner else { continue };
        let key = (owner.module.clone(), owner.qualname.clone(), owner.firstlineno);
        let Some(into) = kept.get_mut(&key) else { continue };
        for (callee, n) in aggregated.callees {
            *into.callees.entry(callee).or_default() += n;
        }
    }
    kept
}

#[derive(Default)]
struct Samples {
    calls: Vec<u64>,
    ns: Vec<u64>,
    callees: BTreeMap<String, Vec<u64>>,
}

/// A function's medians over the runs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Aggregated {
    pub(crate) calls: u64,
    pub(crate) self_ns: u64,
    pub(crate) callees: BTreeMap<String, u64>,
}

/// Where the driver runs and what it runs.
struct Driver<'a> {
    python: &'a OsStr,
    script: &'a Path,
    pkg: &'a Path,
    /// The package `pkg` is pass 3's copy of: its import root stays on
    /// `sys.path` after the copy's, so the modules beside it (a standalone
    /// module's siblings) are still found.
    origin: Option<&'a Path>,
}

impl Driver<'_> {
    /// The driver's command for `mode`, under `wrapper` (a program and its
    /// arguments, e.g. callgrind) when given.
    fn command(
        &self,
        mode: &str,
        out: &Path,
        extra: &[&OsStr],
        workload: &[OsString],
        wrapper: &[&OsStr],
    ) -> std::process::Command {
        let mut command = match wrapper {
            [program, args @ ..] => {
                let mut command = std::process::Command::new(program);
                command.args(args).arg(self.python);
                command
            }
            [] => std::process::Command::new(self.python),
        };
        // `-I` would also ignore `PYTHONHASHSEED`; the `PYTHON*` variables it
        // stands for are removed instead, so the seed is the only one left.
        for (name, _) in std::env::vars_os() {
            if name.to_string_lossy().starts_with("PYTHON") {
                command.env_remove(name);
            }
        }
        command
            .arg("-P")
            .arg("-s")
            .arg(self.script)
            .arg(mode)
            .arg(self.pkg)
            .arg(out)
            .args(extra);
        if !workload.is_empty() {
            command.arg("--").args(workload);
        }
        command.env("PYTHONHASHSEED", "0");
        if let Some(origin) = self.origin {
            command.env("PYCC_PROFILE_ORIGIN", origin);
        }
        command
    }

    fn run(&self, mode: &str, out: &Path, extra: &[&OsStr], workload: &[OsString]) -> Result<(), String> {
        let status = self
            .command(mode, out, extra, workload, &[])
            .status()
            .map_err(|e| format!("cannot run {}: {e}", self.python.to_string_lossy()))?;
        if !status.success() {
            return Err(format!("the profile's {mode} pass failed ({status})"));
        }
        Ok(())
    }

    fn json<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T, String> {
        let text = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
        serde_json::from_str(&text).map_err(|e| format!("the driver's answer: {e}"))
    }
}

/// The package's current functions, from `python` (the driver's `index`).
pub(crate) fn index(python: &OsStr, pkg: &Path) -> Result<Vec<Function>, String> {
    let scratch = pycc_scratch::ScratchDir::new("profile").map_err(|e| e.to_string())?;
    let script = scratch.join("driver.py");
    std::fs::write(&script, DRIVER.replace("\r\n", "\n")).map_err(|e| e.to_string())?;
    let driver = Driver {
        python,
        script: &script,
        pkg,
        origin: None,
    };
    let out = scratch.join("index.json");
    driver.run("index", &out, &[], &[])?;
    Driver::json(&out)
}

/// `pycc profile PKG -o PROFILE --check -- PYTHON`: reads PROFILE against
/// PKG's current sources (indexed with PYTHON) and prints what a build would
/// do with it: each entry dropped as stale, each function the rule selects
/// with its share, and each current entry it rejects with why. Runs no
/// workload.
pub(crate) fn check_profile(
    pkg: &Path,
    profile_path: &Path,
    workload: &[OsString],
    rule: Rule,
) -> Result<(), String> {
    let [python, ..] = workload else {
        return Err("pycc profile --check needs the interpreter: -- PYTHON".to_string());
    };
    let profile = Profile::read(profile_path)?;
    let sources = index(python, pkg)?;
    let (current, notes) = profile.current(&sources);
    for note in &notes {
        eprintln!("{note}");
    }
    let decided: Vec<Candidate> = current.iter().map(|(e, f)| f.candidate(e)).collect();
    for d in select(profile.total_ns, &decided, profile.rule(rule)) {
        let e = d.entry;
        match &d.rejected {
            None => println!(
                "selected: {}:{} ({:.2}%, {} calls)",
                e.module,
                e.qualname,
                d.share * 100.0,
                e.calls
            ),
            Some(why) => println!("rejected: {}:{}: {why}", e.module, e.qualname),
        }
    }
    Ok(())
}

/// `pycc profile`: profiles `workload` (PYTHON SCRIPT [ARGS]) against `pkg`
/// and writes the profile to `out`.
pub(crate) fn run_profile(
    pkg: &Path,
    out: &Path,
    workload: &[OsString],
    runs: u32,
    rule: Rule,
    verify: Option<verify::Options>,
) -> Result<(), String> {
    let [python, rest @ ..] = workload else {
        return Err("pycc profile needs a workload: -- PYTHON SCRIPT [ARGS]".to_string());
    };
    if rest.is_empty() {
        return Err("pycc profile needs a workload: -- PYTHON SCRIPT [ARGS]".to_string());
    }
    if !pkg.exists() {
        return Err(format!("{}: no such package or module", pkg.display()));
    }
    // Without a Windows verbatim prefix: the path goes on the workload's `sys.path`.
    let pkg = crate::canonical::canonicalize(pkg).map_err(|e| e.to_string())?;
    let sources = index(python, &pkg)?;
    let scratch = pycc_scratch::ScratchDir::new("profile").map_err(|e| e.to_string())?;
    let script = scratch.join("driver.py");
    std::fs::write(&script, DRIVER.replace("\r\n", "\n")).map_err(|e| e.to_string())?;
    let driver = Driver {
        python,
        script: &script,
        pkg: &pkg,
        origin: None,
    };
    let mut samples = Vec::new();
    for i in 0..runs.max(1) {
        let path = scratch.join(format!("calls{i}.json"));
        driver.run("calls", &path, &[], rest)?;
        samples.push(Driver::json::<CallsRun>(&path)?);
    }
    let (total_ns, medians) = aggregate(&samples);
    let medians = fold_nested_code(medians, &sources);
    let by_key: BTreeMap<(&str, &str, u32), &Function> = sources
        .iter()
        .map(|f| ((f.module.as_str(), f.qualname.as_str(), f.firstlineno), f))
        .collect();
    let mut functions: Vec<Entry> = medians
        .into_iter()
        .filter_map(|((module, qualname, line), aggregated)| {
            let sha256 = by_key.get(&(module.as_str(), qualname.as_str(), line))?.sha256.clone();
            Some(Entry {
                module,
                qualname,
                firstlineno: line,
                sha256,
                calls: aggregated.calls,
                self_ns: aggregated.self_ns,
                callees: aggregated.callees,
                arg_types: ArgTypes::new(),
                handled: 0,
                sampled: false,
                measured: None,
            })
        })
        .collect();
    // A function an earlier profile in `out` measured keeps its verdict while
    // its source is unchanged (pass 3).
    if let Ok(old) = Profile::read(out) {
        let mut carried = Profile {
            functions: std::mem::take(&mut functions),
            ..old.clone()
        };
        verify::carry_verdicts(&mut carried, &old);
        functions = carried.functions;
    }
    // Every entry has its function: `functions` holds only those `by_key` has.
    fn candidate<'p>(by_key: &BTreeMap<(&str, &str, u32), &Function>, e: &'p Entry) -> Candidate<'p> {
        by_key[&(e.module.as_str(), e.qualname.as_str(), e.firstlineno)].candidate(e)
    }
    let decided: Vec<Candidate> = functions.iter().map(|e| candidate(&by_key, e)).collect();
    let candidates: Vec<(String, String, u32)> = select(total_ns, &decided, rule)
        .into_iter()
        .filter(|d| d.rejected.is_none())
        .map(|d| (d.entry.module.clone(), d.entry.qualname.clone(), d.entry.firstlineno))
        .collect();
    if !candidates.is_empty() {
        let list = scratch.join("candidates.json");
        std::fs::write(&list, serde_json::to_string(&candidates).expect("serializes"))
            .map_err(|e| e.to_string())?;
        let path = scratch.join("types.json");
        driver.run("types", &path, &[list.as_os_str()], rest)?;
        #[derive(Deserialize)]
        struct Types {
            functions: Vec<(String, String, u32, ArgTypes, u64)>,
        }
        let types: Types = Driver::json(&path)?;
        let mut by: BTreeMap<(String, String, u32), (ArgTypes, u64)> = types
            .functions
            .into_iter()
            .map(|(m, q, l, t, h)| ((m, q, l), (t, h)))
            .collect();
        for e in &mut functions {
            if let Some((t, h)) = by.remove(&(e.module.clone(), e.qualname.clone(), e.firstlineno)) {
                e.arg_types = t;
                e.handled = h;
                e.sampled = true;
            }
        }
    }
    let version = std::process::Command::new(python)
        .args(["-I", "-c", "import platform; print(platform.python_version())"])
        .output()
        .map_err(|e| e.to_string())?;
    let mut profile = Profile {
        version: VERSION,
        python: String::from_utf8_lossy(&version.stdout).trim().to_string(),
        workload: rest
            .iter()
            .map(|a| a.to_string_lossy().into_owned())
            .collect::<Vec<_>>()
            .join(" "),
        runs: runs.max(1),
        total_ns,
        functions,
        sampled: !candidates.is_empty(),
        verified: false,
    };
    // Pass 2's samples can reject a candidate (and with it its callers),
    // never admit one it did not sample; pass 3 keeps only the measured
    // winners.
    if let Some(options) = verify {
        let decided: Vec<Candidate> = profile.functions.iter().map(|e| candidate(&by_key, e)).collect();
        let candidates: Vec<FunctionKey> = select(total_ns, &decided, profile.rule(rule))
            .into_iter()
            .filter(|d| d.rejected.is_none())
            .map(|d| (d.entry.module.clone(), d.entry.qualname.clone(), d.entry.firstlineno))
            .collect();
        // The hot ones on their own: pass 3 checks them alone against their
        // regions.
        let core: BTreeSet<FunctionKey> = profile
            .functions
            .iter()
            .filter(|e| {
                let share = if total_ns == 0 { 0.0 } else { e.self_ns as f64 / total_ns as f64 };
                share >= rule.share && e.calls >= rule.calls
            })
            .map(|e| (e.module.clone(), e.qualname.clone(), e.firstlineno))
            .collect();
        let cx = verify::Context {
            driver: &driver,
            workload: rest,
            scratch: &scratch,
            options,
        };
        verify::verify(&mut profile, &candidates, &core, &cx)?;
    }
    std::fs::write(out, profile.to_json())
        .map_err(|e| format!("cannot write {}: {e}", out.display()))?;
    let decided: Vec<Candidate> = profile.functions.iter().map(|e| candidate(&by_key, e)).collect();
    let selected = select(total_ns, &decided, profile.rule(rule))
        .iter()
        .filter(|d| d.rejected.is_none())
        .count();
    eprintln!(
        "pycc profile: {} functions of {} profiled over {} runs; {selected} pass the selection rule",
        profile.functions.len(),
        pkg.display(),
        profile.runs
    );
    Ok(())
}

#[cfg(test)]
mod tests;
