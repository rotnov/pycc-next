use super::*;

fn function(module: &str, qualname: &str, line: u32, eligible: bool) -> Function {
    Function {
        module: module.into(),
        qualname: qualname.into(),
        firstlineno: line,
        sha256: format!("{module}:{qualname}"),
        lines: 10,
        eligible,
        excluded: false,
    }
}

fn request<'a>(mode: SelectMode, compile: &'a [String], no_compile: &'a [String]) -> Request<'a> {
    Request {
        mode,
        profile: None,
        types_profile: None,
        compile,
        no_compile,
        rule: Rule::default(),
        package_sources: &[],
        python: "",
        excludes: None,
        module: None,
    }
}

fn keys(selection: &Selection, module: &str) -> Vec<(String, u32)> {
    selection
        .modules
        .get(module)
        .map(|set| set.iter().cloned().collect())
        .unwrap_or_default()
}

#[test]
fn nothing_is_selected_without_a_profile_and_the_build_says_so() {
    let sources = [function("m", "f", 1, true)];
    let selection = decide(&request(SelectMode::Profile, &[], &[]), &sources).unwrap();
    assert!(selection.modules.is_empty());
    assert_eq!(selection.notes.len(), 1);
    assert!(selection.notes[0].contains("no function is selected"), "{:?}", selection.notes);
    let none = decide(&request(SelectMode::None, &[], &[]), &sources).unwrap();
    assert!(none.modules.is_empty() && none.notes.is_empty());
}

#[test]
fn all_selects_every_eligible_function_and_the_overrides_apply() {
    let sources = [
        function("p.a", "f", 1, true),
        function("p.a", "generator", 4, false),
        function("p.b", "C.m", 2, true),
        function("p.b", "C.x", 5, true),
        function("p.b", "C.x", 9, true),
    ];
    let all = decide(&request(SelectMode::All, &[], &[]), &sources).unwrap();
    assert_eq!(keys(&all, "p.a"), [("f".to_string(), 1)]);
    assert_eq!(
        keys(&all, "p.b"),
        [("C.m".to_string(), 2), ("C.x".to_string(), 5), ("C.x".to_string(), 9)]
    );
    // `--no-compile` names every function of a qualified name (a
    // property's accessors); `--compile` of an ineligible one is a note.
    let no = ["p.b:C.x".to_string()];
    let generator = ["p.a:generator".to_string()];
    let some = decide(&request(SelectMode::All, &generator, &no), &sources).unwrap();
    assert_eq!(keys(&some, "p.b"), [("C.m".to_string(), 2)]);
    assert!(some.notes[0].contains("not eligible"), "{:?}", some.notes);
    let one = ["p.b:C.m".to_string()];
    let added = decide(&request(SelectMode::Profile, &one, &[]), &sources).unwrap();
    assert_eq!(keys(&added, "p.b"), [("C.m".to_string(), 2)]);
    assert!(added.notes.is_empty(), "{:?}", added.notes);
}

#[test]
fn a_bad_or_unknown_override_is_an_error() {
    let sources = [function("p.a", "f", 1, true)];
    let bad = ["f".to_string()];
    let err = decide(&request(SelectMode::All, &bad, &[]), &sources).unwrap_err();
    assert!(err.contains("expected MODULE:QUALNAME"), "{err}");
    let missing = ["p.a:g".to_string()];
    let err = decide(&request(SelectMode::All, &missing, &[]), &sources).unwrap_err();
    assert!(err.contains("no such function"), "{err}");
    // Another build's module is not this build's to check.
    let other = ["q.z:g".to_string()];
    assert!(decide(&request(SelectMode::All, &other, &[]), &sources).is_ok());
    let err = decide(&request(SelectMode::All, &[], &bad), &sources).unwrap_err();
    assert!(err.contains("--no-compile"), "{err}");
}

#[test]
fn a_profile_selects_by_the_rule() {
    let dir = pycc_scratch::ScratchDir::new("select_test").unwrap();
    let path = dir.join("p.json");
    let entry = |qualname: &str, line: u32, calls: u64, ns: u64| profile::Entry {
        module: "m".into(),
        qualname: qualname.into(),
        firstlineno: line,
        sha256: format!("m:{qualname}"),
        calls,
        self_ns: ns,
        callees: Default::default(),
        arg_types: Default::default(),
        handled: 0,
        sampled: true,
        measured: None,
    };
    let p = profile::Profile {
        version: profile::VERSION,
        python: "3.15".into(),
        workload: "w.py".into(),
        runs: 1,
        total_ns: 1000,
        functions: vec![entry("hot", 1, 100, 500), entry("rare", 5, 2, 400), entry("gone", 9, 100, 50)],
        sampled: true,
        verified: false,
        winner: None,
    };
    std::fs::write(&path, p.to_json()).unwrap();
    let sources = [function("m", "hot", 1, true), function("m", "rare", 5, true)];
    let mut req = request(SelectMode::Profile, &[], &[]);
    req.profile = Some(&path);
    let selection = decide(&req, &sources).unwrap();
    assert_eq!(keys(&selection, "m"), [("hot".to_string(), 1)]);
    assert!(selection.notes.iter().any(|n| n.contains("m:gone") && n.contains("no longer")), "{:?}", selection.notes);
}

/// A selected function carries the types its profile entry observed, and a
/// build of `--compile` functions takes them from `--types-profile`; a
/// function whose source changed since carries none.
#[test]
fn a_selected_function_carries_its_observed_types() {
    let dir = pycc_scratch::ScratchDir::new("select_test").unwrap();
    let path = dir.join("p.json");
    let mut hot = profile::Entry {
        module: "m".into(),
        qualname: "hot".into(),
        firstlineno: 1,
        sha256: "m:hot".into(),
        calls: 100,
        self_ns: 500,
        callees: Default::default(),
        arg_types: Default::default(),
        handled: 0,
        sampled: true,
        measured: None,
    };
    hot.arg_types.insert("s".into(), [("str".to_string(), 7)].into());
    hot.arg_types.insert("d".into(), [("dict".to_string(), 7)].into());
    let p = profile::Profile {
        version: profile::VERSION,
        python: "3.15".into(),
        workload: "w.py".into(),
        runs: 1,
        total_ns: 1000,
        functions: vec![hot],
        sampled: true,
        verified: false,
        winner: None,
    };
    std::fs::write(&path, p.to_json()).unwrap();
    let sources = [function("m", "hot", 1, true)];
    let expected: BTreeMap<Key, Types> = [(("hot".to_string(), 1), vec![("s".to_string(), "str".to_string())])].into();
    let mut req = request(SelectMode::Profile, &[], &[]);
    req.profile = Some(&path);
    assert_eq!(decide(&req, &sources).unwrap().types["m"], expected);
    let compile = ["m:hot".to_string()];
    let mut req = request(SelectMode::None, &compile, &[]);
    assert!(decide(&req, &sources).unwrap().types.is_empty());
    req.types_profile = Some(&path);
    assert_eq!(decide(&req, &sources).unwrap().types["m"], expected);
    let mut changed = function("m", "hot", 1, true);
    changed.sha256 = "other".into();
    assert!(decide(&req, &[changed]).unwrap().types.is_empty());
}

#[test]
fn a_members_selection_round_trips_through_its_file() {
    let set: BTreeSet<Key> = [("C.m".to_string(), 3), ("f".to_string(), 10)].into();
    let types: BTreeMap<Key, Types> =
        [(("f".to_string(), 10), vec![("s".to_string(), "str".to_string()), ("n".to_string(), "int".to_string())])].into();
    assert_eq!(from_text(&to_text(&set, &types)).unwrap(), (set.clone(), types));
    assert_eq!(from_text(&to_text(&set, &BTreeMap::new())).unwrap(), (set, BTreeMap::new()));
    assert!(from_text("").unwrap().0.is_empty());
    assert!(from_text("3 f s\n").is_err());
    assert!(from_text("x f\n").is_err());
    assert!(from_text("3\n").is_err());
}

#[test]
fn the_project_table_names_a_profile_and_overrides() {
    let dir = pycc_scratch::ScratchDir::new("select_test").unwrap();
    let entry = dir.join("m.py");
    assert_eq!(Project::read(&entry).unwrap(), Project::default());
    std::fs::write(
        dir.join("pyproject.toml"),
        "[tool.pycc]\nprofile = \"prof/p.json\"\ncompile = [\"m:f\"]\nexclude = [\"m:g\"]\n",
    )
    .unwrap();
    let project = Project::read(&entry).unwrap();
    assert_eq!(project.profile, Some(dir.join("prof/p.json")));
    assert_eq!(project.compile, ["m:f"]);
    assert_eq!(project.exclude, ["m:g"]);
    std::fs::write(dir.join("pyproject.toml"), "[tool.pycc]\ncompile = \"m:f\"\n").unwrap();
    assert!(Project::read(&entry).unwrap_err().contains("list of strings"));
    std::fs::write(dir.join("pyproject.toml"), "[tool.pycc]\nprofile = 1\n").unwrap();
    assert!(Project::read(&entry).unwrap_err().contains("must be a string"));
}
