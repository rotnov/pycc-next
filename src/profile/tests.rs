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
        measured: None,
    }
}

fn function(module: &str, qualname: &str, line: u32, sha: &str) -> Function {
    Function {
        module: module.into(),
        qualname: qualname.into(),
        firstlineno: line,
        sha256: sha.into(),
        eligible: true,
    }
}

fn profile(functions: Vec<Entry>) -> Profile {
    Profile {
        version: VERSION,
        python: "3.15.2".into(),
        workload: "w.py".into(),
        runs: 5,
        total_ns: 1000,
        functions,
        verified: false,
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
    let (current, notes) = p.current(&sources);
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
    let input: Vec<(&Entry, bool)> = es.iter().map(|e| (e, e.qualname != "gen")).collect();
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
fn the_region_keeps_functions_whose_calls_stay_inside_the_selection() {
    let with = |q: &str, ns: u64, out: &[(&str, u64)]| {
        let mut e = entry("m", q, 1, "", 1000, ns);
        e.callees = out.iter().map(|(k, n)| (k.to_string(), *n)).collect();
        e
    };
    let es = [
        // Calls only selected functions, itself and its own nested code.
        with("hub", 500, &[("m:leaf:1", 10), ("m:hub:1", 5), ("m:hub.<locals>.inner:1", 5)]),
        // No calls to Python functions at all.
        with("leaf", 400, &[]),
        // Half of its calls leave: not most inside, rejected.
        with("even", 300, &[("m:leaf:1", 1), ("*", 1)]),
        // A third leave: kept.
        with("most", 250, &[("m:leaf:1", 2), ("*", 1)]),
        // Mostly glue: rejected.
        with("glue", 200, &[("*", 3), ("m:leaf:1", 1)]),
        // Only selected through `glue`: rejected once `glue` is.
        with("caller", 100, &[("m:glue:1", 1)]),
    ];
    let input: Vec<(&Entry, bool)> = es.iter().map(|e| (e, true)).collect();
    let decided = select(1000, &input, Rule { share: 0.0, ..Rule::default() });
    let got: Vec<(&str, Option<&str>)> = decided
        .iter()
        .map(|d| (d.entry.qualname.as_str(), d.rejected.as_deref()))
        .collect();
    assert_eq!(
        got,
        [
            ("hub", None),
            ("leaf", None),
            ("even", Some("1 of its 2 calls to Python functions leave the selection")),
            ("most", None),
            ("glue", Some("3 of its 4 calls to Python functions leave the selection")),
            ("caller", Some("1 of its 1 calls to Python functions leave the selection")),
        ]
    );
    // At 0 no call may leave; at 1 only a function all of whose calls leave
    // is rejected.
    let strict = select(1000, &input, Rule { share: 0.0, outside: 0.0, ..Rule::default() });
    assert!(strict[3].rejected.is_some() && strict[0].rejected.is_none() && strict[1].rejected.is_none());
    let loose = select(1000, &input, Rule { share: 0.0, outside: 1.0, ..Rule::default() });
    assert!(loose[2].rejected.is_none() && loose[4].rejected.is_none());
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
    let input: Vec<(&Entry, bool)> = es.iter().map(|e| (e, true)).collect();
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
        with: 2,
        cpython_ns: [10, 11, 12],
        compiled_ns: if compiled { [9, 10, 11] } else { [20, 21, 22] },
        instructions: None,
        refused: None,
    };
    let mut a = entry("m", "a", 1, "", 100, 300);
    a.measured = Some(measured(false));
    let mut b = entry("m", "b", 2, "", 100, 200);
    b.measured = Some(measured(true));
    let c = entry("m", "c", 3, "", 100, 100);
    let mut p = profile(vec![a, b, c]);
    let rule = Rule { max: 2, ..Rule::default() };
    let picked = |p: &Profile| -> Vec<String> {
        let input: Vec<(&Entry, bool)> = p.functions.iter().map(|e| (e, true)).collect();
        select(p.total_ns, &input, p.rule(rule))
            .into_iter()
            .filter(|d| d.rejected.is_none())
            .map(|d| d.entry.qualname.clone())
            .collect()
    };
    // Unverified (`--no-verify`), the rule alone decides.
    assert_eq!(picked(&p), ["b", "c"]);
    p.verified = true;
    assert_eq!(picked(&p), ["b"]);
    let input: Vec<(&Entry, bool)> = p.functions.iter().map(|e| (e, true)).collect();
    let c = select(p.total_ns, &input, p.rule(rule)).into_iter().find(|d| d.entry.qualname == "c").unwrap();
    assert_eq!(c.rejected.as_deref(), Some("not measured by pass 3 (it was not among the candidates it built)"));
    // The flag is part of the file.
    let dir = pycc_scratch::ScratchDir::new("profile_test").unwrap();
    let path = dir.join("p.json");
    std::fs::write(&path, p.to_json()).unwrap();
    assert!(Profile::read(&path).unwrap().verified);
    assert!(!profile(Vec::new()).to_json().contains("verified"));
}
