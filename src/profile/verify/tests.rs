use super::*;
use crate::profile::{ArgTypes, Entry, Rule};

fn entry(qualname: &str, ns: u64, callees: &[&str]) -> Entry {
    Entry {
        module: "m".into(),
        qualname: qualname.into(),
        firstlineno: 1,
        sha256: format!("sha-{qualname}"),
        calls: 100,
        self_ns: ns,
        callees: callees.iter().map(|c| (format!("m:{c}:1"), 10)).collect(),
        arg_types: ArgTypes::new(),
        handled: 0,
        sampled: true,
        measured: None,
    }
}

fn profile(functions: Vec<Entry>) -> Profile {
    Profile {
        version: crate::profile::VERSION,
        python: "3.15".into(),
        workload: "w.py".into(),
        runs: 1,
        total_ns: 1000,
        functions,
        sampled: true,
        verified: false,
    }
}

fn k(q: &str) -> FunctionKey {
    ("m".into(), q.into(), 1)
}

fn slower() -> Measured {
    Measured {
        compiled: false,
        with: 2,
        cpython_ns: [90, 100, 110],
        compiled_ns: [115, 120, 125],
        instructions: None,
        refused: None,
        without: None,
    }
}

#[test]
fn quartiles_are_medians_of_halves() {
    assert_eq!(quartiles(&[5, 1, 3, 2, 4]), [1, 3, 4]);
    assert_eq!(quartiles(&[4, 1, 3, 2]), [1, 2, 3]);
    assert_eq!(quartiles(&[7]), [7, 7, 7]);
}

#[test]
fn faster_is_the_compiled_median_under_cpython_s_first_quartile() {
    let c = |compiled| Check {
        cpython: [90, 100, 110],
        compiled: [0, compiled, 0],
        instructions: None,
    };
    // A median inside CPython's interquartile range is noise, not a win.
    assert!(c(89).faster());
    assert!(!c(90).faster());
    assert!(!c(95).faster());
    assert!(!c(111).faster());
}

#[test]
fn regions_are_candidates_connected_by_calls_most_self_time_first() {
    let p = profile(vec![
        entry("a", 10, &["b"]),
        entry("b", 10, &[]),
        entry("c", 50, &["x"]),
        entry("d", 5, &["c"]),
        entry("x", 999, &[]),
    ]);
    let got = regions(&p, &[k("a"), k("b"), k("c"), k("d")]);
    assert_eq!(got, vec![vec![k("c"), k("d")], vec![k("a"), k("b")]]);
}

#[test]
fn a_verdict_carries_over_while_the_source_hash_is_the_same() {
    let mut old_a = entry("a", 1, &[]);
    old_a.measured = Some(slower());
    let mut old_b = entry("b", 1, &[]);
    old_b.measured = Some(slower());
    let mut old_c = entry("c", 1, &[]);
    old_c.measured = Some(Measured {
        compiled: true,
        ..slower()
    });
    let old = profile(vec![old_a, old_b, old_c]);
    let mut changed_b = entry("b", 1, &[]);
    changed_b.sha256 = "edited".into();
    let mut new = profile(vec![entry("a", 1, &[]), changed_b, entry("c", 1, &[])]);
    carry_verdicts(&mut new, &old);
    assert_eq!(new.functions[0].measured, Some(slower()));
    // Its source changed: measured again.
    assert_eq!(new.functions[1].measured, None);
    // A kept function keeps its win too: a re-run never flips a verdict.
    assert_eq!(new.functions[2].measured, Some(Measured { compiled: true, ..slower() }));
}

#[test]
fn the_rule_rejects_a_function_measured_slower() {
    let mut a = entry("a", 500, &[]);
    a.measured = Some(slower());
    let b = entry("b", 400, &[]);
    fn candidate(e: &Entry) -> crate::profile::Candidate<'_> {
        crate::profile::Candidate {
            entry: e,
            eligible: true,
            lines: 10,
        }
    }
    let decided = crate::profile::select(1000, &[candidate(&a), candidate(&b)], Rule::default());
    let why = decided[0].rejected.as_deref().unwrap();
    assert!(why.starts_with("not faster when compiled: the workload took 0.0 ms with it and 1 other functions compiled"), "{why}");
    assert_eq!(decided[1].rejected, None);
    // Rejected because its region's hot functions won without it.
    a.measured = Some(Measured {
        without: Some([70, 80, 90]),
        ..slower()
    });
    let decided = crate::profile::select(1000, &[candidate(&a), candidate(&b)], Rule::default());
    let why = decided[0].rejected.as_deref().unwrap();
    assert!(why.starts_with("faster without it: the workload took 0.0 ms with only its region's hot functions compiled"), "{why}");
}

#[test]
fn a_member_is_copied_with_the_packages_above_it() {
    let dir = pycc_scratch::ScratchDir::new("verify-copy").unwrap();
    let src = dir.join("src");
    std::fs::create_dir_all(src.join("top/pkg/__pycache__")).unwrap();
    std::fs::write(src.join("top/__init__.py"), "").unwrap();
    std::fs::write(src.join("top/other.py"), "").unwrap();
    std::fs::write(src.join("top/pkg/__init__.py"), "x = 1\n").unwrap();
    std::fs::write(src.join("top/pkg/mod.py"), "y = 2\n").unwrap();
    std::fs::write(src.join("top/pkg/__pycache__/mod.pyc"), "").unwrap();
    let copy = copy_package(&src.join("top/pkg"), &dir.join("out")).unwrap();
    assert_eq!(copy, dir.join("out/top/pkg"));
    assert!(dir.join("out/top/__init__.py").is_file());
    // The whole top-level package, for the member's sibling imports.
    assert!(dir.join("out/top/other.py").is_file());
    assert_eq!(std::fs::read_to_string(copy.join("mod.py")).unwrap(), "y = 2\n");
    assert!(!copy.join("__pycache__").exists());
    let member = copy_package(&src.join("top/other.py"), &dir.join("one")).unwrap();
    assert_eq!(member, dir.join("one/top/other.py"));
    assert!(dir.join("one/top/pkg/mod.py").is_file());
    std::fs::write(src.join("alone.py"), "z = 3\n").unwrap();
    let single = copy_package(&src.join("alone.py"), &dir.join("two")).unwrap();
    assert_eq!(single, dir.join("two/alone.py"));
    assert!(!dir.join("two/top").exists());
}

#[test]
fn a_check_is_clear_once_the_interquartile_ranges_are_apart() {
    let plain = [100, 101, 102, 103, 104];
    assert!(clear(&plain, &[110, 111, 112, 113, 114]));
    assert!(clear(&plain, &[90, 91, 92, 93, 94]));
    assert!(!clear(&plain, &[102, 103, 104, 105, 106]));
}

/// What a build's report says it left bytecode: the reason it gives, or,
/// for a function it does not name compiled at all, that.
#[test]
fn a_refusal_is_read_from_the_build_report() {
    let set: BTreeSet<FunctionKey> = [k("a"), k("b"), k("c")].into_iter().collect();
    let report = "selected: m:a\ncompiled: m:a\r\nnot compiled: m:b: C0001: a nested def\nfallback: x\n";
    let refused = refusals(&set, report);
    assert_eq!(
        refused,
        [
            (k("b"), "C0001: a nested def".to_string()),
            (k("c"), "the build did not compile it".to_string()),
        ]
        .into_iter()
        .collect()
    );
    assert!(refusals(&set, "compiled: m:a\ncompiled: m:b\ncompiled: m:c\n").is_empty());
}
