use super::*;

fn entry(module: &str, qualname: &str, line: u32, sha: &str, calls: u64, ns: u64) -> Entry {
    Entry {
        module: module.into(),
        qualname: qualname.into(),
        firstlineno: line,
        sha256: sha.into(),
        calls,
        self_ns: ns,
        callees: BTreeMap::new(),
        arg_types: ArgTypes::new(),
        handled: 0,
        sampled: true,
        measured: None,
    }
}

fn function(module: &str, qualname: &str, line: u32, sha: &str) -> Function {
    Function {
        module: module.into(),
        qualname: qualname.into(),
        firstlineno: line,
        sha256: sha.into(),
        lines: 10,
        eligible: true,
        excluded: false,
    }
}

/// `e` as [`select`] decides it: ten lines long.
fn candidate(e: &Entry, eligible: bool) -> Candidate<'_> {
    Candidate {
        entry: e,
        eligible,
        excluded: false,
        lines: 10,
        winning: crate::profile::Winning::No,
    }
}

/// Records `names` as `p`'s winning set, measured with no `--exclude`.
fn win(p: &mut Profile, names: &[&str]) {
    let members = p
        .functions
        .iter()
        .filter(|e| names.contains(&e.qualname.as_str()))
        .map(|e| (e.module.clone(), e.qualname.clone(), e.firstlineno, e.sha256.clone()))
        .collect();
    p.winner = Some(Winner { members, excludes: Vec::new(), python: p.python.clone() });
}

/// `p`'s candidates, marked by its winning set (taken as valid).
fn marked(p: &Profile) -> Vec<Candidate<'_>> {
    static VALID: Result<(), String> = Ok(());
    let mut input: Vec<Candidate> = p.functions.iter().map(|e| candidate(e, true)).collect();
    p.mark(&mut input, &VALID);
    input
}

fn profile(functions: Vec<Entry>) -> Profile {
    Profile {
        version: VERSION,
        python: "3.15.2".into(),
        workload: "w.py".into(),
        runs: 5,
        total_ns: 1000,
        functions,
        sampled: true,
        verified: false,
        winner: None,
    }
}

#[test]
fn the_file_is_sorted_and_the_same_samples_give_the_same_bytes() {
    let mut a = entry("pkg.b", "f", 3, "aa", 60, 10);
    a.arg_types
        .entry("x".into())
        .or_default()
        .insert("dict".into(), 7);
    let p = profile(vec![a, entry("pkg.a", "g", 9, "bb", 51, 20), entry("pkg.a", "C.m", 2, "cc", 1, 1)]);
    let text = p.to_json();
    let order: Vec<usize> = ["\"C.m\"", "\"g\"", "\"f\""]
        .iter()
        .map(|q| text.find(q).unwrap())
        .collect();
    assert!(order.windows(2).all(|w| w[0] < w[1]), "{text}");
    let mut reversed = p.clone();
    reversed.functions.reverse();
    assert_eq!(reversed.to_json(), text);
    let dir = pycc_scratch::ScratchDir::new("profile_test").unwrap();
    let path = dir.join("p.json");
    std::fs::write(&path, &text).unwrap();
    assert_eq!(Profile::read(&path).unwrap().to_json(), text);
}

#[test]
fn a_profile_of_another_version_or_not_json_is_refused() {
    let dir = pycc_scratch::ScratchDir::new("profile_test").unwrap();
    let path = dir.join("p.json");
    let mut p = profile(vec![]);
    p.version = 2;
    std::fs::write(&path, p.to_json()).unwrap();
    assert!(Profile::read(&path).unwrap_err().contains("version 2"));
    std::fs::write(&path, "{").unwrap();
    assert!(Profile::read(&path).unwrap_err().contains("not a pycc profile"));
    assert!(Profile::read(&dir.join("none.json")).unwrap_err().contains("cannot read"));
}

#[test]
fn stale_and_missing_entries_are_dropped_with_a_note() {
    let p = profile(vec![
        entry("pkg.m", "same", 1, "s1", 100, 100),
        entry("pkg.m", "moved", 5, "s2", 100, 100),
        entry("pkg.m", "changed", 9, "s3", 100, 100),
        entry("pkg.m", "gone", 12, "s4", 100, 100),
        entry("pkg.m", "C.x", 20, "get", 100, 100),
        entry("pkg.m", "C.x", 24, "set", 100, 100),
    ]);
    let sources = vec![
        function("pkg.m", "same", 1, "s1"),
        function("pkg.m", "moved", 7, "s2"),
        function("pkg.m", "changed", 9, "other"),
        function("pkg.m", "C.x", 21, "get"),
        function("pkg.m", "C.x", 25, "set"),
    ];
    let (current, notes) = p.current(&sources, None);
    let kept: Vec<(&str, u32)> = current
        .iter()
        .map(|(e, f)| (e.qualname.as_str(), f.firstlineno))
        .collect();
    assert_eq!(kept, [("same", 1), ("moved", 7), ("C.x", 21), ("C.x", 25)]);
    assert_eq!(
        notes,
        [
            "note: profile entry pkg.m:changed (line 9) dropped: its source changed since it was profiled",
            "note: profile entry pkg.m:gone (line 12) dropped: it is no longer in the sources",
        ]
    );
}

#[test]
fn the_rule_takes_eligible_hot_and_often_called_functions_by_self_time() {
    let es = [
        entry("m", "hot", 1, "", 1000, 400),
        entry("m", "rare", 2, "", 49, 300),
        entry("m", "gen", 3, "", 1000, 200),
        entry("m", "warm", 4, "", 50, 100),
        entry("m", "cool", 5, "", 50, 5),
        entry("m", "cold", 6, "", 1000, 4),
    ];
    let input: Vec<Candidate> = es.iter().map(|e| candidate(e, e.qualname != "gen")).collect();
    let decided = select(1000, &input, Rule::default());
    let got: Vec<(&str, Option<&str>)> = decided
        .iter()
        .map(|d| (d.entry.qualname.as_str(), d.rejected.as_deref()))
        .collect();
    assert_eq!(
        got,
        [
            ("hot", None),
            ("rare", Some("49 calls is under 50")),
            ("gen", Some("not eligible (a generator, coroutine or nested function)")),
            ("warm", None),
            ("cool", None),
            ("cold", Some("self time 0.40% is under 0.50%")),
        ]
    );
    let one = select(1000, &input, Rule { max: 1, ..Rule::default() });
    assert_eq!(one[3].rejected.as_deref(), Some("not among the top 1 by self time"));
    assert!(select(0, &input, Rule::default()).iter().all(|d| d.share == 0.0));
}

#[test]
fn runs_aggregate_to_medians_and_a_missing_call_counts_zero() {
    type Sample<'a> = (&'a str, u64, u64, &'a [(&'a str, u64)]);
    let run = |total, ns: &[Sample]| CallsRun {
        total_ns: total,
        functions: ns
            .iter()
            .map(|(q, c, t, out)| {
                let out = out.iter().map(|(k, n)| (k.to_string(), *n)).collect();
                ("m".to_string(), q.to_string(), "1".to_string(), *c, *t, out)
            })
            .collect(),
    };
    let runs = [
        run(10, &[("f", 5, 100, &[("*", 4), ("m:g:1", 1)]), ("g", 1, 1, &[])]),
        run(30, &[("f", 5, 300, &[("*", 6)])]),
        run(20, &[("f", 5, 200, &[("*", 5), ("m:g:1", 1)]), ("g", 1, 3, &[])]),
    ];
    let (total, by) = aggregate(&runs);
    assert_eq!(total, 20);
    let f = &by[&("m".into(), "f".into(), 1)];
    assert_eq!((f.calls, f.self_ns), (5, 200));
    assert_eq!(f.callees, BTreeMap::from([("*".to_string(), 5), ("m:g:1".to_string(), 1)]));
    let g = &by[&("m".into(), "g".into(), 1)];
    assert_eq!((g.calls, g.self_ns), (1, 1));
    assert!(g.callees.is_empty());
    // A callee seen in one run of three has a median of zero, so it is gone.
    let (_, by) = aggregate(&[runs[0].clone(), runs[1].clone(), runs[1].clone()]);
    assert!(!by[&("m".into(), "f".into(), 1)].callees.contains_key("m:g:1"));
    let (total, _) = aggregate(&runs[..2]);
    assert_eq!(total, 20);
    assert_eq!(aggregate(&[]).0, 0);
}

#[test]
fn a_region_is_judged_together_and_a_crossing_by_its_cost() {
    let with = |q: &str, calls: u64, ns: u64, out: &[(&str, u64)]| {
        let mut e = entry("m", q, 1, "", calls, ns);
        e.callees = out.iter().map(|(k, n)| (k.to_string(), *n)).collect();
        e
    };
    let es = [
        // Calls only selected functions, itself and its own nested code.
        with("hub", 1000, 500, &[("m:leaf:1", 10), ("m:hub:1", 5), ("m:hub.<locals>.inner:1", 5)]),
        // No calls to Python functions at all.
        with("leaf", 1000, 400, &[]),
        // Hot, calling a cold function: the region pulls it in.
        with("bisect", 1000, 300, &[("m:split:1", 900)]),
        // One call out per call costs 36 ns against 100 ns of self time: kept.
        with("caller", 1000, 100, &[("m:glue:1", 1)]),
        // Three calls out cost 108 ns against 30 ns of self time.
        with("glue", 1000, 30, &[("*", 3), ("m:leaf:1", 1)]),
        // A region of cold functions: 0.40% together.
        with("a", 100, 2, &[("m:b:1", 1)]),
        with("b", 100, 2, &[]),
        // Cold and rarely called, but in `bisect`'s region.
        with("split", 10, 1, &[]),
    ];
    let input: Vec<Candidate> = es.iter().map(|e| candidate(e, true)).collect();
    let decided = select(1000, &input, Rule::default());
    let got: Vec<(&str, Option<&str>)> = decided
        .iter()
        .map(|d| (d.entry.qualname.as_str(), d.rejected.as_deref()))
        .collect();
    assert_eq!(
        got,
        [
            ("hub", None),
            ("leaf", None),
            ("bisect", None),
            ("caller", None),
            (
                "glue",
                Some("its 3 calls out of the selection would cost about 0.1 us (36 ns each), against 0.0 us of self time")
            ),
            ("a", Some("its region's self time 0.40% (2 functions) is under 0.50%")),
            ("b", Some("its region's self time 0.40% (2 functions) is under 0.50%")),
            ("split", None),
        ]
    );
    // A cheaper crossing keeps `glue`; a dearer one drops `caller` too.
    let cheap = select(1000, &input, Rule { crossing_ns: 9, ..Rule::default() });
    assert!(cheap[4].rejected.is_none());
    let dear = select(1000, &input, Rule { crossing_ns: 100, ..Rule::default() });
    assert!(dear[3].rejected.is_some() && dear[0].rejected.is_none());
    // The size cap: 15 lines take `hub` (10 lines) alone, and `hub`'s calls
    // to `leaf` then cross, cheaply enough.
    let small = select(1000, &input, Rule { lines: 15, ..Rule::default() });
    assert_eq!(small[0].rejected, None);
    assert_eq!(small[1].rejected.as_deref(), Some("its 10 lines would take the selection past 15 lines"));
}

/// A rejection by a later pass never hides the step a function failed
/// first: a function its region rejects says so in a verified profile too.
#[test]
fn a_rejection_names_the_first_step_the_function_failed() {
    let mut glue = entry("m", "glue", 1, "", 1000, 30);
    glue.callees = BTreeMap::from([("*".to_string(), 3)]);
    let mut p = profile(vec![glue]);
    p.verified = true;
    let input: Vec<Candidate> = p.functions.iter().map(|e| candidate(e, true)).collect();
    let d = &select(p.total_ns, &input, p.rule(Rule::default()))[0];
    assert!(d.rejected.as_deref().unwrap().starts_with("its 3 calls out of the selection"), "{d:?}");
}

#[test]
fn a_generator_expression_s_calls_count_as_its_function_s() {
    let agg = |calls: u64, out: &[(&str, u64)]| Aggregated {
        calls,
        self_ns: 1,
        callees: out.iter().map(|(k, n)| (k.to_string(), *n)).collect(),
    };
    let key = |q: &str, l: u32| ("m".to_string(), q.to_string(), l);
    let medians = BTreeMap::from([
        (key("C.total", 4), agg(60, &[("*", 1)])),
        (key("C.total.<locals>.<genexpr>", 5), agg(660, &[("m:hot:1", 600), ("*", 2)])),
        (key("C.total.<locals>.inner", 6), agg(1, &[("*", 9)])),
        (key("C.total.<locals>.inner.<locals>.<lambda>", 7), agg(1, &[("*", 3)])),
        (key("<module>", 1), agg(1, &[("*", 5)])),
    ]);
    let sources = [
        function("m", "C.total", 4, ""),
        function("m", "C.total.<locals>.inner", 6, ""),
    ];
    let folded = fold_nested_code(medians, &sources);
    let keys: Vec<&str> = folded.keys().map(|(_, q, _)| q.as_str()).collect();
    assert_eq!(keys, ["C.total", "C.total.<locals>.inner"]);
    let total = &folded[&key("C.total", 4)];
    assert_eq!(total.calls, 60);
    assert_eq!(total.callees, BTreeMap::from([("*".to_string(), 3), ("m:hot:1".to_string(), 600)]));
    assert_eq!(folded[&key("C.total.<locals>.inner", 6)].callees["*"], 12);
}

#[test]
fn a_function_that_mostly_catches_exceptions_is_not_selected() {
    let mut often = entry("m", "often", 1, "", 3000, 300);
    often.handled = 500;
    let mut rare = entry("m", "rare", 2, "", 40, 200);
    rare.handled = 19;
    let mut some = entry("m", "some", 3, "", 100, 100);
    some.handled = 49;
    let es = [often, rare, some];
    let input: Vec<Candidate> = es.iter().map(|e| candidate(e, true)).collect();
    let decided = select(1000, &input, Rule { calls: 1, ..Rule::default() });
    let got: Vec<Option<&str>> = decided.iter().map(|d| d.rejected.as_deref()).collect();
    assert_eq!(
        got,
        [
            // Out of its first 1,000 calls, the ones pass 2 sampled.
            Some("it catches an exception in 500 of its first 1000 calls"),
            None,
            None,
        ]
    );
    let none = select(1000, &input, Rule { calls: 1, handled: 0.0, ..Rule::default() });
    assert!(none.iter().all(|d| d.rejected.is_some()));
}

/// Three functions pass the rule and two fit (`max` 2): pass 3 measures `a`
/// and `b`, rejects `a`, and the selection is then `b` alone. `c`, which
/// would take the slot `a` left, was never built: it is not selected.
#[test]
fn a_verified_profile_selects_only_its_measured_winners() {
    let measured = |compiled| Measured {
        compiled,
        with: if compiled { 1 } else { 2 },
        cpython_ns: [10, 11, 12],
        compiled_ns: if compiled { [9, 10, 11] } else { [20, 21, 22] },
        instructions: None,
        cpython_instructions: None,
        refused: None,
        without: None,
    };
    let mut a = entry("m", "a", 1, "", 100, 300);
    a.measured = Some(measured(false));
    let mut b = entry("m", "b", 2, "", 100, 200);
    b.measured = Some(measured(true));
    let c = entry("m", "c", 3, "", 100, 100);
    let mut p = profile(vec![a, b, c]);
    let rule = Rule { max: 2, ..Rule::default() };
    let picked = |p: &Profile| -> Vec<String> {
        let input = marked(p);
        select(p.total_ns, &input, p.rule(rule))
            .into_iter()
            .filter(|d| d.rejected.is_none())
            .map(|d| d.entry.qualname.clone())
            .collect()
    };
    // Unverified (`--no-verify`), the rule alone decides.
    assert_eq!(picked(&p), ["b", "c"]);
    p.verified = true;
    win(&mut p, &["b"]);
    assert_eq!(picked(&p), ["b"]);
    let input = marked(&p);
    let c = select(p.total_ns, &input, p.rule(rule)).into_iter().find(|d| d.entry.qualname == "c").unwrap();
    assert_eq!(c.rejected.as_deref(), Some("not measured by pass 3 (it was not among the candidates it built)"));
    // The flag is part of the file.
    let dir = pycc_scratch::ScratchDir::new("profile_test").unwrap();
    let path = dir.join("p.json");
    std::fs::write(&path, p.to_json()).unwrap();
    assert!(Profile::read(&path).unwrap().verified);
    assert!(!profile(Vec::new()).to_json().contains("verified"));
}

/// Pass 3's winning set is selected member for member: the rule's region
/// and crossing steps chose its candidates and are not applied again. Here
/// the winning core is `a` and `b`; `a` calls the cold `c`, measured not
/// faster, 1000 times, which as a crossing (36 us) costs more than its
/// 25 us of self time. Rejecting `a` for it would compile `{b}`, a set
/// never measured.
#[test]
fn the_build_selects_exactly_the_set_pass_3_measured_as_the_winner() {
    let won = |with| Measured {
        compiled: true,
        with,
        cpython_ns: [100, 110, 120],
        compiled_ns: [80, 85, 90],
        instructions: None,
        cpython_instructions: None,
        refused: None,
        without: None,
    };
    let mut a = entry("m", "a", 1, "", 100, 25_000);
    a.callees.insert("m:c:3".into(), 1000);
    a.callees.insert("m:b:2".into(), 100);
    a.measured = Some(won(2));
    let mut b = entry("m", "b", 2, "", 100, 900_000);
    b.measured = Some(won(2));
    let mut c = entry("m", "c", 3, "", 1000, 75_000);
    c.measured = Some(Measured {
        compiled: false,
        without: Some([80, 85, 90]),
        compiled_ns: [95, 100, 105],
        with: 3,
        ..won(3)
    });
    let mut p = profile(vec![a, b, c]);
    p.total_ns = 1_000_000;
    p.verified = true;
    win(&mut p, &["a", "b"]);
    let input = marked(&p);
    let decided = select(p.total_ns, &input, p.rule(Rule::default()));
    let selected: Vec<&str> = decided
        .iter()
        .filter(|d| d.rejected.is_none())
        .map(|d| d.entry.qualname.as_str())
        .collect();
    assert_eq!(selected, ["b", "a"], "{decided:?}");
    // Unverified, the crossing rule rejects `a` (the rule chooses what pass
    // 3 measures).
    p.verified = false;
    let input: Vec<Candidate> = p.functions.iter().map(|e| candidate(e, true)).collect();
    let decided = select(p.total_ns, &input, p.rule(Rule::default()));
    assert!(decided.iter().find(|d| d.entry.qualname == "a").unwrap().rejected.is_some(), "{decided:?}");
    // A winning set that is not valid selects nothing, and says why.
    p.verified = true;
    let invalid: Result<(), String> = Err("its member m:a (line 1) changed or is gone".into());
    let mut input: Vec<Candidate> = p.functions.iter().map(|e| candidate(e, true)).collect();
    p.mark(&mut input, &invalid);
    let decided = select(p.total_ns, &input, p.rule(Rule::default()));
    assert!(decided.iter().all(|d| d.rejected.as_deref().unwrap().starts_with(
        "pass 3's winning set is not valid (its member m:a (line 1) changed or is gone)"
    )), "{decided:?}");
}

/// The types a body is specialized for: a parameter's one exact type in
/// every sampled call, when a native body takes it; nothing for a
/// function pass 2 did not sample.
#[test]
fn observed_types_are_a_parameters_one_native_exact_type() {
    let mut e = entry("m", "f", 1, "s", 10, 10);
    e.arg_types.insert("s".into(), [("str".to_string(), 9)].into());
    e.arg_types.insert("n".into(), [("int".to_string(), 5), ("float".to_string(), 4)].into());
    e.arg_types.insert("d".into(), [("dict".to_string(), 9)].into());
    e.arg_types.insert("x".into(), [("float".to_string(), 9)].into());
    assert_eq!(e.observed_types(), [("s".to_string(), "str".to_string()), ("x".to_string(), "float".to_string())]);
    e.sampled = false;
    assert!(e.observed_types().is_empty());
}
