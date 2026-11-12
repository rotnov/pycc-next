use super::super::*;

fn k(q: &str) -> FunctionKey {
    ("m".into(), q.into(), 1)
}

fn compiled(verdicts: &BTreeMap<FunctionKey, Measured>) -> BTreeSet<FunctionKey> {
    verdicts
        .iter()
        .filter(|(_, m)| m.compiled)
        .map(|(key, _)| key.clone())
        .collect()
}

fn set(names: &[&str]) -> BTreeSet<FunctionKey> {
    names.iter().map(|q| k(q)).collect()
}

/// CPython's instructions for the training workload, and what 0.5% of it is
/// in [`CORE`]'s units (100,000 instructions): 78.6.
const CPYTHON: u64 = 1_572_764_362;
const UNIT: i64 = 100_000;

/// pydantic's 9-function hot core, most self time first, each with the
/// instructions (in [`UNIT`]s) it adds to the training workload when
/// compiled beside the others, as pass 3 measured them leaving each out:
/// `mode`, `_config` and `create_schema_validator` each cost less than 0.5%
/// of the workload, together more; the rest pay.
const CORE: [(&str, i64); 9] = [
    ("make_json_hashable", -601),
    ("remap_json_schema", -90),
    ("mode", 41),
    ("create_schema_validator", 24),
    ("sort_recursive", -49),
    ("deduplicate_schemas", -8),
    ("generate_schema_inner", -3),
    ("get_all_json_refs", -35),
    ("config", 41),
];

fn core() -> (Vec<FunctionKey>, BTreeSet<FunctionKey>) {
    let candidates: Vec<FunctionKey> = CORE.iter().map(|(q, _)| k(q)).collect();
    let core = candidates.iter().cloned().collect();
    (candidates, core)
}

fn delta(set: &BTreeSet<FunctionKey>, core: &[(&str, i64)]) -> i64 {
    core.iter()
        .filter(|(q, _)| set.contains(&k(q)))
        .map(|(_, d)| d)
        .sum()
}

/// Counted: each set runs the sum of its members' instructions.
fn counted<'a>(
    core: &'a [(&'a str, i64)],
    asked: &'a mut Vec<BTreeSet<FunctionKey>>,
) -> impl FnMut(&BTreeSet<FunctionKey>, usize) -> Outcome + 'a {
    move |set, _| {
        asked.push(set.clone());
        Ok(Ok(Check {
            cpython: [90, 100, 110],
            compiled: [80, 90, 100],
            instructions: Some(Instructions {
                cpython: CPYTHON,
                compiled: (CPYTHON as i64 + delta(set, core) * UNIT) as u64,
            }),
        }))
    }
}

fn prune_core(
    core: &[(&str, i64)],
    budget: usize,
) -> (
    BTreeMap<FunctionKey, Measured>,
    usize,
    Vec<BTreeSet<FunctionKey>>,
) {
    let candidates: Vec<FunctionKey> = core.iter().map(|(q, _)| k(q)).collect();
    let hot: BTreeSet<FunctionKey> = candidates.iter().cloned().collect();
    let mut asked = Vec::new();
    let mut run = counted(core, &mut asked);
    let (verdicts, builds) = decide(
        &candidates,
        std::slice::from_ref(&candidates),
        &no_regions,
        &hot,
        &BTreeMap::new(),
        8,
        budget,
        &mut run,
    )
    .unwrap();
    drop(run);
    (verdicts, builds, asked)
}

/// pydantic's case: no member costs 0.5% alone, so no single removal is
/// kept, but the three whose removal did better at all, removed together,
/// save 0.67% more: they are dropped, and every member that pays is kept,
/// the one that saves 0.05% among them. The selection is that set,
/// measured as a whole, and a dropped member says why.
#[test]
fn a_loser_is_dropped_and_a_winner_is_kept() {
    let (verdicts, builds, asked) = prune_core(&CORE, MAX_PRUNE_BUILDS);
    // The core once; seven removals, least self time first; the three
    // removed together.
    assert_eq!(builds, 9);
    assert_eq!(asked[1], &set(&CORE.map(|(q, _)| q)) - &set(&["config"]));
    assert!(asked[1..8]
        .iter()
        .all(|s| s.contains(&k("make_json_hashable")) && s.contains(&k("remap_json_schema"))));
    let kept = &set(&CORE.map(|(q, _)| q)) - &set(&["mode", "create_schema_validator", "config"]);
    assert_eq!(asked[8], kept);
    assert_eq!(compiled(&verdicts), kept);
    for loser in ["mode", "create_schema_validator", "config"] {
        let why = verdicts[&k(loser)].rejection();
        assert!(
            why.starts_with("faster without it: the workload ran -5.00% instructions"),
            "{why}"
        );
        assert!(
            why.contains("against -4.32% with it and 8 other functions"),
            "{why}"
        );
    }
    // The selection's own check: 6 functions, 786 units fewer.
    let m = &verdicts[&k("deduplicate_schemas")];
    assert_eq!((m.with, m.instructions), (6, Some(-786 * UNIT)));
}

/// A member that costs 0.5% of the workload alone is dropped by its own
/// removal, with no build for a group of one.
#[test]
fn a_loser_that_costs_enough_alone_is_dropped_alone() {
    let mut big = CORE;
    big[2].1 = -3; // `mode` pays a little.
    big[3].1 = 1;
    big[8].1 = 90; // `_config` costs 0.57%.
    let (verdicts, builds, _) = prune_core(&big, MAX_PRUNE_BUILDS);
    // `create_schema_validator` and `_config` did better without: the two
    // together are measured, and win.
    assert_eq!(builds, 9);
    assert_eq!(
        compiled(&verdicts),
        &set(&big.map(|(q, _)| q)) - &set(&["config", "create_schema_validator"])
    );
    // Only `_config` loses: its own removal is the selection.
    big[3].1 = -1;
    let (verdicts, builds, asked) = prune_core(&big, MAX_PRUNE_BUILDS);
    assert_eq!(builds, 8);
    assert_eq!(
        compiled(&verdicts),
        &set(&big.map(|(q, _)| q)) - &set(&["config"])
    );
    assert!(asked.contains(&compiled(&verdicts)));
}

/// The pruning step makes at most its budget of builds, one of them for
/// the group, the least self time first; with no budget the winner is not
/// pruned, and with too few removals measured to reach the losers that
/// together pay, it is kept whole.
#[test]
fn pruning_makes_at_most_its_budget_of_builds() {
    let all = set(&CORE.map(|(q, _)| q));
    for budget in 0..=12 {
        let (verdicts, builds, asked) = prune_core(&CORE, budget);
        assert!(builds <= 1 + budget, "budget {budget}: {builds}");
        let selected = compiled(&verdicts);
        assert!(asked.contains(&selected), "budget {budget}");
        // `_config` is tried first, `create_schema_validator` sixth, `mode`
        // seventh: with fewer than 7 removals measured and the group, the
        // losers found do not pay together.
        if budget < 8 {
            assert_eq!(selected, all, "budget {budget}");
        } else {
            assert_eq!(selected.len(), 6, "budget {budget}");
        }
    }
    assert_eq!(prune_core(&CORE, 0).1, 1);
    assert_eq!(prune_core(&CORE, 8).1, 9);
    // Every member but the last tried, and the group: 1 + 8 + 1.
    assert_eq!(prune_core(&CORE, 12).1, 11);
}

/// On wall time the members whose removal lowered the median are removed
/// together, and kept out only when that set is faster than the winner
/// beyond the noise of both and by 5% of CPython's median.
#[test]
fn on_wall_time_a_removal_must_be_faster_beyond_the_noise() {
    let (candidates, core) = core();
    // In 0.1 ms: CPython at `cpython`; the core at 82.0 ms, the core
    // without its three losers at 71.4 ms.
    let pick = |cpython: u64| {
        let mut run = |set: &BTreeSet<FunctionKey>, _: usize| -> Outcome {
            let t = (1500 + delta(set, &CORE)) as u64;
            Ok(Ok(Check {
                cpython: [cpython - 100, cpython, cpython + 100],
                compiled: [t - 20, t, t + 20],
                instructions: None,
            }))
        };
        let (verdicts, builds) = decide(
            &candidates,
            std::slice::from_ref(&candidates),
            &no_regions,
            &core,
            &BTreeMap::new(),
            8,
            8,
            &mut run,
        )
        .unwrap();
        assert_eq!(builds, 9);
        verdicts
    };
    // 10.6 ms is over 5% of CPython's 200 ms: dropped.
    let verdicts = pick(2000);
    assert_eq!(
        compiled(&verdicts),
        &core - &set(&["mode", "create_schema_validator", "config"])
    );
    let why = verdicts[&k("mode")].rejection();
    assert!(
        why.starts_with("faster without it: the workload took"),
        "{why}"
    );
    // Under 5% of CPython's 300 ms: the winner whole.
    assert_eq!(compiled(&pick(3000)), core);
}

/// The carried winning set's members are never tried, and a removal whose
/// build refuses a member invalidates the winner: the selection is then the
/// best set still valid, never a removal that was not kept.
#[test]
fn pruning_keeps_the_carried_set_and_stops_on_a_refusal() {
    let (candidates, core) = core();
    let earlier = Measured {
        compiled: true,
        with: 1,
        cpython_ns: [90, 100, 110],
        compiled_ns: [80, 90, 100],
        instructions: Some(-10_000_000),
        cpython_instructions: Some(CPYTHON),
        refused: None,
        without: None,
        without_instructions: None,
    };
    let base: BTreeMap<FunctionKey, Measured> = [(k("x"), earlier)].into_iter().collect();
    let mut asked = Vec::new();
    let mut run = counted(&CORE, &mut asked);
    let (verdicts, _) = decide(
        &candidates,
        std::slice::from_ref(&candidates),
        &no_regions,
        &core,
        &base,
        8,
        8,
        &mut run,
    )
    .unwrap();
    drop(run);
    assert!(asked.iter().all(|set| set.contains(&k("x"))), "{asked:?}");
    assert!(compiled(&verdicts).contains(&k("x")));
    assert_eq!(compiled(&verdicts).len(), 7);
    // The second removal's build refuses `make_json_hashable`.
    let mut n = 0;
    let mut run = |set: &BTreeSet<FunctionKey>, b: usize| -> Outcome {
        n += 1;
        if n == 3 {
            return Ok(Err([(k("make_json_hashable"), "refused".to_string())]
                .into_iter()
                .collect()));
        }
        counted(&CORE, &mut Vec::new())(set, b)
    };
    let (verdicts, _) = decide(
        &candidates,
        std::slice::from_ref(&candidates),
        &no_regions,
        &core,
        &BTreeMap::new(),
        8,
        8,
        &mut run,
    )
    .unwrap();
    assert!(compiled(&verdicts).is_empty(), "{verdicts:?}");
    assert_eq!(
        verdicts[&k("make_json_hashable")].refused.as_deref(),
        Some("refused")
    );
}


/// The regions of `set`'s selected unit when `edges` are its functions'
/// calls among them: the connected components, as `units::static_regions`
/// forms them from the sources.
fn regions_of(edges: &[(&str, &str)], set: &BTreeSet<FunctionKey>) -> Vec<BTreeSet<FunctionKey>> {
    let mut regions: Vec<BTreeSet<FunctionKey>> = Vec::new();
    for (a, b) in edges {
        let (a, b) = (k(a), k(b));
        if !set.contains(&a) || !set.contains(&b) {
            continue;
        }
        let pair: BTreeSet<FunctionKey> = [a, b].into();
        let (joined, mut rest): (Vec<_>, Vec<_>) = regions.into_iter().partition(|r| !r.is_disjoint(&pair));
        rest.push(joined.into_iter().flatten().chain(pair).collect());
        regions = rest;
    }
    regions
}

/// A region of the winner's selected unit (selected-regions.md, section
/// 4.7) is removed whole: `mode` and `_config` call each other, so no set
/// measured holds one without the other. Their removal together saves
/// 0.82%, enough alone; with `create_schema_validator`'s it is the
/// selection. The regions are those of the winner itself.
#[test]
fn a_region_is_pruned_whole() {
    let (candidates, core) = core();
    let asked_regions = std::cell::RefCell::new(Vec::new());
    let units = |set: &BTreeSet<FunctionKey>| {
        asked_regions.borrow_mut().push(set.clone());
        regions_of(&[("mode", "config")], set)
    };
    let mut asked = Vec::new();
    let mut run = counted(&CORE, &mut asked);
    let (verdicts, builds) = decide(
        &candidates,
        std::slice::from_ref(&candidates),
        &units,
        &core,
        &BTreeMap::new(),
        8,
        MAX_PRUNE_BUILDS,
        &mut run,
    )
    .unwrap();
    drop(run);
    assert_eq!(asked_regions.into_inner(), std::slice::from_ref(&core), "the winner's regions");
    // The core; seven removals (of eight units), least self time first; the
    // group.
    assert_eq!(builds, 9);
    for asked in &asked {
        assert_eq!(asked.contains(&k("mode")), asked.contains(&k("config")), "{asked:?}");
    }
    assert_eq!(asked[6], &core - &set(&["mode", "config"]), "the region tried as one");
    let kept = &core - &set(&["mode", "config", "create_schema_validator"]);
    assert_eq!(asked[8], kept);
    assert_eq!(compiled(&verdicts), kept);
    for loser in ["mode", "config"] {
        assert!(verdicts[&k(loser)].rejection().starts_with("faster without it"), "{loser}");
    }
}

/// A set pass 3 measures is built as its own selected unit: its hot
/// functions are measured alone even when they call a cold candidate, and
/// the winner is pruned by its own regions. `hot` and `hot2` both call
/// `cold_callee`, so all three form one region of the candidates; the
/// winner `{hot, hot2}` holds no edge, so each is a unit of its own and
/// pruning drops `hot2`, which costs 0.64% of the workload. Under the
/// candidates' region the two would be one unit, and nothing pruned.
#[test]
fn a_set_forms_its_own_regions() {
    let candidates: Vec<FunctionKey> = ["hot", "hot2", "cold_callee"].map(k).to_vec();
    let core = set(&["hot", "hot2"]);
    let costs = [("hot", -300), ("hot2", 100), ("cold_callee", 0)];
    let edges = [("hot", "cold_callee"), ("hot2", "cold_callee")];
    let units = |set: &BTreeSet<FunctionKey>| regions_of(&edges, set);
    let mut asked = Vec::new();
    let mut run = counted(&costs, &mut asked);
    let (verdicts, _) = decide(
        &candidates,
        std::slice::from_ref(&candidates),
        &units,
        &core,
        &BTreeMap::new(),
        8,
        MAX_PRUNE_BUILDS,
        &mut run,
    )
    .unwrap();
    drop(run);
    assert_eq!(regions_of(&edges, &set(&["hot", "hot2", "cold_callee"])).len(), 1);
    assert!(regions_of(&edges, &core).is_empty());
    assert_eq!(
        asked,
        [
            set(&["hot", "hot2", "cold_callee"]),
            core.clone(),
            set(&["hot"]),
            set(&["hot2"]),
        ],
        "the whole set, the hot ones alone, then each removed on its own"
    );
    assert_eq!(compiled(&verdicts), set(&["hot"]));
}
