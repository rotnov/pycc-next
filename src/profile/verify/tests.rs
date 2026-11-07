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
        winner: None,
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
            excluded: false,
            lines: 10,
            winning: crate::profile::Winning::No,
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
    assert!(why.starts_with("faster without it: the workload took 0.0 ms with a measured set of candidates that leaves it out compiled"), "{why}");
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
    std::fs::write(src.join("unrelated.txt"), "").unwrap();
    let out = dir.join("out");
    let copy = copy_package(&src.join("top/pkg"), &out, &[]).unwrap();
    assert_eq!(copy, out.canonicalize().unwrap().join("top/pkg"));
    // The whole top-level package, for the member's sibling imports; with
    // no project marker, nothing else.
    assert!(out.join("top/__init__.py").is_file());
    assert!(out.join("top/other.py").is_file());
    assert!(!out.join("unrelated.txt").exists());
    assert_eq!(std::fs::read_to_string(copy.join("mod.py")).unwrap(), "y = 2\n");
    assert!(!copy.join("__pycache__").exists());
    let member = copy_package(&src.join("top/other.py"), &dir.join("one"), &[]).unwrap();
    assert!(member.ends_with("one/top/other.py"));
    assert!(dir.join("one/top/pkg/mod.py").is_file());
    std::fs::write(src.join("alone.py"), "z = 3\n").unwrap();
    let single = copy_package(&src.join("alone.py"), &dir.join("two"), &[]).unwrap();
    assert!(single.ends_with("two/alone.py"));
    assert!(!dir.join("two/top").exists());
    assert!(!dir.join("two/unrelated.txt").exists());
    // The path returned is the copy's own canonical path, whatever form the
    // destination was given in.
    let roundabout = copy_package(&src.join("alone.py"), &dir.join("two/../three"), &[]).unwrap();
    assert_eq!(roundabout, dir.join("three").canonicalize().unwrap().join("alone.py"));
}

/// The workload reads files beside the package by their paths from the
/// import root (a test directory, benchmark data, `pyproject.toml`): the
/// copy pass 3 times is of the project root, the nearest directory with a
/// project marker, not only the package.
#[test]
fn the_copy_holds_the_project_files_beside_the_package() {
    let dir = pycc_scratch::ScratchDir::new("verify-copy-root").unwrap();
    let project = dir.join("project");
    std::fs::create_dir_all(project.join("src/pkg")).unwrap();
    std::fs::create_dir_all(project.join("tests/data")).unwrap();
    std::fs::create_dir_all(project.join(".git")).unwrap();
    std::fs::write(project.join("src/pkg/__init__.py"), "").unwrap();
    std::fs::write(project.join("tests/test_pkg.py"), "import pkg\n").unwrap();
    std::fs::write(project.join("tests/data/case.txt"), "case\n").unwrap();
    std::fs::write(project.join("pyproject.toml"), "[project]\n").unwrap();
    std::fs::write(project.join(".git/HEAD"), "x").unwrap();
    std::fs::write(dir.join("beside_the_project.txt"), "").unwrap();
    let out = dir.join("out");
    let copy = copy_package(&project.join("src/pkg"), &out, &[]).unwrap();
    assert_eq!(copy, out.canonicalize().unwrap().join("src/pkg"));
    assert_eq!(std::fs::read_to_string(out.join("tests/data/case.txt")).unwrap(), "case\n");
    assert!(out.join("tests/test_pkg.py").is_file());
    assert!(out.join("pyproject.toml").is_file());
    assert!(!out.join(".git").exists());
    assert!(!out.join("beside_the_project.txt").exists());
    // A copy made inside the project does not copy itself.
    let inner = copy_package(&project.join("src/pkg"), &project.join("build/tree"), &[]).unwrap();
    assert!(inner.ends_with("build/tree/src/pkg"));
    assert!(project.join("build/tree/tests/data/case.txt").is_file());
    assert!(!project.join("build/tree/build/tree").exists());
}

/// Without a project marker the copy is the package alone, plus the paths
/// `--verify-include` names, at their paths under the import root.
#[test]
fn an_included_path_is_copied_beside_the_package() {
    let dir = pycc_scratch::ScratchDir::new("verify-copy-include").unwrap();
    let tree = dir.join("tree");
    std::fs::create_dir_all(tree.join("pkg")).unwrap();
    std::fs::create_dir_all(tree.join("tests")).unwrap();
    std::fs::create_dir_all(tree.join("benchmark")).unwrap();
    std::fs::write(tree.join("pkg/__init__.py"), "").unwrap();
    std::fs::write(tree.join("tests/test_pkg.py"), "").unwrap();
    std::fs::write(tree.join("benchmark/data.toml"), "a = 1\n").unwrap();
    std::fs::write(tree.join("benchmark/other.toml"), "").unwrap();
    let out = dir.join("out");
    let include = [tree.join("tests"), tree.join("benchmark/data.toml")];
    copy_package(&tree.join("pkg"), &out, &include).unwrap();
    assert!(out.join("pkg/__init__.py").is_file());
    assert!(out.join("tests/test_pkg.py").is_file());
    assert_eq!(std::fs::read_to_string(out.join("benchmark/data.toml")).unwrap(), "a = 1\n");
    assert!(!out.join("benchmark/other.toml").exists());
    let outside = copy_package(&tree.join("pkg"), &dir.join("x"), &[dir.join("tree/../out")]);
    assert!(outside.unwrap_err().contains("not under the package's import root"));
}

/// An installed package is copied alone, whatever project marker lies above
/// its `site-packages`.
#[test]
fn an_installed_package_is_copied_alone() {
    let dir = pycc_scratch::ScratchDir::new("verify-copy-installed").unwrap();
    let site = dir.join("venv/lib/python3.15/site-packages");
    std::fs::create_dir_all(site.join("pkg")).unwrap();
    std::fs::create_dir_all(site.join("other")).unwrap();
    std::fs::write(site.join("pkg/__init__.py"), "").unwrap();
    std::fs::write(site.join("other/__init__.py"), "").unwrap();
    std::fs::write(site.join("setup.py"), "").unwrap();
    std::fs::write(dir.join("pyproject.toml"), "").unwrap();
    let out = dir.join("out");
    let copy = copy_package(&site.join("pkg"), &out, &[]).unwrap();
    assert!(copy.ends_with("out/pkg"));
    assert!(out.join("pkg/__init__.py").is_file());
    assert!(!out.join("other").exists());
    assert!(!out.join("setup.py").exists());
    assert!(!out.join("pyproject.toml").exists());
}

/// Whether this process can read a file whose permissions forbid it (it
/// runs as root): the unreadable-entry tests then prove nothing.
#[cfg(unix)]
fn reads_unreadable(dir: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    let probe = dir.join("probe");
    std::fs::write(&probe, "x").unwrap();
    std::fs::set_permissions(&probe, std::fs::Permissions::from_mode(0o000)).unwrap();
    let readable = std::fs::read(&probe).is_ok();
    std::fs::set_permissions(&probe, std::fs::Permissions::from_mode(0o644)).unwrap();
    std::fs::remove_file(&probe).unwrap();
    readable
}

/// An entry of the project outside the package that cannot be read is left
/// out of the copy (with a note); one inside the package is an error.
#[cfg(unix)]
#[test]
fn an_unreadable_entry_is_skipped_outside_the_package_only() {
    use std::os::unix::fs::PermissionsExt;
    let dir = pycc_scratch::ScratchDir::new("verify-copy-unreadable").unwrap();
    if reads_unreadable(&dir) {
        eprintln!("skipped: this process reads files it has no permission for (root)");
        return;
    }
    let project = dir.join("project");
    std::fs::create_dir_all(project.join("private")).unwrap();
    std::fs::write(project.join("pyproject.toml"), "").unwrap();
    std::fs::write(project.join("m.py"), "x = 1\n").unwrap();
    std::fs::write(project.join("private/secret"), "").unwrap();
    let mode = |path: &Path, mode| std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode)).unwrap();
    mode(&project.join("private"), 0o000);
    let copied = copy_package(&project.join("m.py"), &dir.join("out"), &[]);
    mode(&project.join("private"), 0o755);
    let copy = copied.unwrap();
    assert_eq!(std::fs::read_to_string(&copy).unwrap(), "x = 1\n");
    assert!(!dir.join("out/private/secret").exists());
    // Inside the package: an error.
    std::fs::create_dir_all(project.join("pkg")).unwrap();
    std::fs::write(project.join("pkg/__init__.py"), "").unwrap();
    std::fs::write(project.join("pkg/data.bin"), "").unwrap();
    mode(&project.join("pkg/data.bin"), 0o000);
    let copied = copy_package(&project.join("pkg"), &dir.join("out2"), &[]);
    mode(&project.join("pkg/data.bin"), 0o644);
    let err = copied.unwrap_err();
    assert!(err.contains("pass 3 cannot copy") && err.contains("data.bin"), "{err}");
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

/// A fake check for [`decide`]: CPython at 100 ms (IQR 90-110); each hot
/// function compiled saves 15 and each cold one 5, but each function of
/// region `c` costs 20, and a set that spans more than one region costs
/// `spanning` (their crossings). Records each set it is asked to time.
fn fake(spanning: i64, asked: &mut Vec<BTreeSet<FunctionKey>>) -> impl FnMut(&BTreeSet<FunctionKey>, usize) -> Outcome + '_ {
    move |set, _| {
        asked.push(set.clone());
        let regions: BTreeSet<char> = set.iter().map(|k| k.1.chars().next().unwrap()).collect();
        let mut ms: i64 = 100;
        for key in set {
            ms -= if key.1.starts_with('c') {
                -20
            } else if key.1.ends_with("hot") {
                15
            } else {
                5
            };
        }
        if regions.len() > 1 {
            ms += spanning;
        }
        let ns = ms as u64;
        Ok(Ok(Check {
            cpython: [90, 100, 110],
            compiled: [ns - 1, ns, ns + 1],
            instructions: None,
        }))
    }
}

fn regions_of(names: &[&str]) -> (Vec<FunctionKey>, Vec<Vec<FunctionKey>>, BTreeSet<FunctionKey>) {
    let regions: Vec<Vec<FunctionKey>> = names
        .iter()
        .map(|r| vec![k(&format!("{r}_hot")), k(&format!("{r}_cold"))])
        .collect();
    let candidates = regions.concat();
    let core = candidates.iter().filter(|key| key.1.ends_with("hot")).cloned().collect();
    (candidates, regions, core)
}

fn compiled(verdicts: &BTreeMap<FunctionKey, Measured>) -> BTreeSet<FunctionKey> {
    verdicts.iter().filter(|(_, m)| m.compiled).map(|(key, _)| key.clone()).collect()
}

/// Three regions each with a cold member, under a budget too tight for every
/// region: the union of the regions that won alone loses, and the
/// selection is one region that won as a whole, never that union.
#[test]
fn the_selection_is_always_a_set_measured_as_a_whole_that_won() {
    let (candidates, regions, core) = regions_of(&["a", "b", "c"]);
    let mut asked = Vec::new();
    let mut run = fake(60, &mut asked);
    let (verdicts, builds) = decide(&candidates, &regions, &core, &BTreeMap::new(), 6, &mut run).unwrap();
    drop(run);
    let asked = asked;
    // All (lost), its core (lost), region a and its core, region b (its
    // core: no build left before the reserved one), then a and b together.
    assert_eq!(builds, 6);
    let a: BTreeSet<FunctionKey> = regions[0].iter().cloned().collect();
    let b: BTreeSet<FunctionKey> = regions[1].iter().cloned().collect();
    assert_eq!(asked[2], a);
    assert_eq!(asked[4], b);
    let union: BTreeSet<FunctionKey> = a.union(&b).cloned().collect();
    assert_eq!(asked[5], union);
    let selected = compiled(&verdicts);
    assert_eq!(selected, a, "{verdicts:?}");
    assert!(asked.contains(&selected));
    // b won alone but is left out, with the winner's times; c, past the
    // budget, was measured only within all the candidates and their core,
    // which lost.
    assert_eq!(verdicts[&k("b_hot")].without, Some([79, 80, 81]));
    assert_eq!(asked.len(), 6);
    assert!(asked[2..].iter().all(|set| !set.contains(&k("c_hot"))));
    assert!(!verdicts[&k("c_hot")].compiled);
    // With a budget that leaves no build for the union, still one region.
    let mut asked = Vec::new();
    let mut run = fake(60, &mut asked);
    let (verdicts, builds) = decide(&candidates, &regions, &core, &BTreeMap::new(), 5, &mut run).unwrap();
    drop(run);
    assert_eq!(builds, 4);
    assert_eq!(compiled(&verdicts), a);
    // When the regions together win and are fastest, they are selected,
    // measured as a whole.
    let mut asked = Vec::new();
    let mut run = fake(10, &mut asked);
    let (verdicts, _) = decide(&candidates, &regions, &core, &BTreeMap::new(), 6, &mut run).unwrap();
    drop(run);
    assert_eq!(compiled(&verdicts), union);
    assert!(asked.contains(&union));
}

/// The first check wins: the faster of all the candidates and their core is
/// selected, the core on a tie, and no region is measured.
#[test]
fn the_faster_of_the_whole_set_and_its_core_is_selected() {
    let (candidates, regions, core) = regions_of(&["a", "b"]);
    let mut asked = Vec::new();
    let mut run = fake(0, &mut asked);
    let (verdicts, builds) = decide(&candidates, &regions, &core, &BTreeMap::new(), 8, &mut run).unwrap();
    drop(run);
    // All: 60 ms; the core: 70 ms.
    assert_eq!(builds, 2);
    let all: BTreeSet<FunctionKey> = candidates.iter().cloned().collect();
    assert_eq!(compiled(&verdicts), all);
    // A cold member left out by a faster core records the core's times.
    let mut asked = Vec::new();
    let mut run = move |set: &BTreeSet<FunctionKey>, n: usize| {
        let all = set.len() == 4;
        fake(0, &mut asked)(set, n).map(|r| {
            r.map(|mut c| {
                if all {
                    c.compiled = [84, 85, 86];
                }
                c
            })
        })
    };
    let (verdicts, _) = decide(&candidates, &regions, &core, &BTreeMap::new(), 8, &mut run).unwrap();
    assert_eq!(compiled(&verdicts), core);
    assert_eq!(verdicts[&k("a_cold")].without, Some([69, 70, 71]));
    assert_eq!(verdicts[&k("a_cold")].compiled_ns, [84, 85, 86]);
}

/// The winning set an earlier profile measured is in every set measured
/// now: the new selection is that set with what wins beside it, measured
/// as a whole, or, when nothing new wins, that set alone, as it was.
#[test]
fn an_earlier_winning_set_is_in_every_set_measured_and_stays_when_nothing_wins() {
    let (candidates, regions, core) = regions_of(&["a", "b"]);
    let earlier = Measured {
        compiled: true,
        with: 1,
        cpython_ns: [90, 100, 110],
        compiled_ns: [94, 95, 96],
        instructions: None,
        refused: None,
        without: None,
    };
    let base: BTreeMap<FunctionKey, Measured> = [(k("x"), earlier.clone())].into_iter().collect();
    let mut asked = Vec::new();
    let mut run = fake(0, &mut asked);
    let (verdicts, _) = decide(&candidates, &regions, &core, &base, 8, &mut run).unwrap();
    drop(run);
    assert!(asked.iter().all(|set| set.contains(&k("x"))), "{asked:?}");
    let mut all: BTreeSet<FunctionKey> = candidates.iter().cloned().collect();
    all.insert(k("x"));
    assert_eq!(compiled(&verdicts), all);
    assert_eq!(verdicts[&k("x")].with, 5);
    // Nothing new wins: the earlier set alone, its verdict unchanged.
    let mut slower = |set: &BTreeSet<FunctionKey>, n: usize| {
        let _ = (set, n);
        Ok(Ok(Check {
            cpython: [90, 100, 110],
            compiled: [149, 150, 151],
            instructions: None,
        }))
    };
    let (verdicts, _) = decide(&candidates, &regions, &core, &base, 8, &mut slower).unwrap();
    assert_eq!(compiled(&verdicts), [k("x")].into_iter().collect());
    assert_eq!(verdicts[&k("x")], earlier);
    assert!(!verdicts[&k("a_hot")].compiled);
}
