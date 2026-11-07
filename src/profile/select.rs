//! The selection rule (docs/design/compile-little.md, section 2.3): which
//! profiled functions a build compiles.

use super::*;

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

/// One function [`select`] decides: its current profile entry, whether it is
/// eligible (section 2.1), and its source's length in lines.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Candidate<'p> {
    pub(crate) entry: &'p Entry,
    pub(crate) eligible: bool,
    /// Its module is kept in Python (`--exclude`).
    pub(crate) excluded: bool,
    /// Its place in pass 3's winning set ([`Profile::winning`]).
    pub(crate) winning: Winning<'p>,
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
/// 5. once pass 3 has run (`rule.measured_only`), in the set it measured
///    as the winner. That set is the selection, member for member, and
///    steps 2 to 4 are not applied to it again: they choose the candidates
///    pass 3 measures, not its result. A winning set that is not valid
///    ([`Profile::winner_valid`]: a member changed, is gone or is kept in
///    Python, or the excludes or the interpreter differ) selects nothing.
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
            let rejected = if c.excluded {
                Some("its module is kept in Python (`--exclude`)".to_string())
            } else if !c.eligible {
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
    // Step 5: pass 3's evidence. Its winning set is the selection, member
    // for member: the steps above chose what pass 3 measured, and are not
    // applied to its result again.
    if rule.measured_only {
        for (d, c) in decided.iter_mut().zip(&order) {
            match c.winning {
                Winning::Member => d.rejected = None,
                Winning::Invalid(why) => {
                    d.rejected = Some(format!(
                        "pass 3's winning set is not valid ({why}): profile again to measure it"
                    ));
                }
                Winning::No if d.rejected.is_none() => {
                    d.rejected = Some(match &d.entry.measured {
                        None => "not measured by pass 3 (it was not among the candidates it built)".to_string(),
                        Some(_) => "not in the set pass 3 measured as the winner".to_string(),
                    });
                }
                Winning::No => {}
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
