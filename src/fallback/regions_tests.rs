use super::*;
use crate::select::{Calls, Key};

/// A fixture module: two components (`loads` with its scanners, `walk`
/// alone recursive), a method edge, and calls that are no edges.
const MODULE: &str = "\
import functools

def skip(src, pos, chars):
    while pos < len(src) and src[pos] in chars:
        pos += 1
    return pos

def parse_key(src, pos):
    pos = skip(src, pos, ' ')
    return pos, src[pos]

def loads(src):
    pos = skip(src, 0, ' ')
    pos, key = parse_key(src, pos)
    return key

def walk(node):
    return [walk(child) for child in node]

def patched(x):
    return x

def uses_patched(x):
    return patched(x)

patched = functools.lru_cache(patched)

def decorated_callee(x):
    return x

decorated_callee = functools.cache(decorated_callee)

def shadowing(skip):
    return skip(1)

def through_lambda(x):
    return (lambda: loads(x))()

def comprehension_target(fs):
    return [loads(1) for loads in fs]

def declares():
    global declared
    declared = 1

def declared():
    return 0

def calls_declared():
    return declared()

def star(*args):
    return args

def calls_star():
    return star(1, 2)

class Parser:
    def parse(self, src):
        return self.value(src) + helper(src)

    def value(self, src):
        return src

    def rebinds(self, src):
        self = Parser()
        return self.value(src)

    def other(self, other):
        return other.value(1)

    @staticmethod
    def static(src):
        return src

    def calls_static(self):
        return self.static(1)

def helper(src):
    return len(src)
";

fn key(source: &str, qualname: &str) -> Key {
    let (owner, member) = qualname.rsplit_once('.').unwrap_or(("", qualname));
    let indent = if owner.is_empty() { "" } else { "    " };
    let needle = format!("\n{indent}def {member}(");
    let at = source.find(&needle).expect("the function is in the fixture") + 1;
    // A decorated `def` starts at its decorator's line.
    let line = line_of(source, at as u32) as u32;
    let decorated = source[..at].trim_end().lines().last().is_some_and(|l| l.trim_start().starts_with('@'));
    (qualname.to_string(), if decorated { line - 1 } else { line })
}

fn select(names: &[&str]) -> BTreeSet<Key> {
    names.iter().map(|name| key(MODULE, name)).collect()
}

fn names(regions: Vec<Vec<Key>>) -> Vec<Vec<String>> {
    regions
        .into_iter()
        .map(|members| members.into_iter().map(|(q, _)| q).collect())
        .collect()
}

#[test]
fn a_region_is_a_component_of_calls_among_selected_functions() {
    let regions = regions_of(MODULE, &select(&["skip", "parse_key", "loads", "walk", "helper"]));
    assert_eq!(
        names(regions),
        [vec!["skip", "parse_key", "loads"], vec!["walk"]],
        "two components, in source order; `helper` is selected but no selected function calls it"
    );
}

#[test]
fn a_member_left_out_of_the_selection_splits_its_region() {
    let regions = regions_of(MODULE, &select(&["skip", "loads", "walk"]));
    assert_eq!(names(regions), [vec!["skip", "loads"], vec!["walk"]]);
    let regions = regions_of(MODULE, &select(&["parse_key", "loads"]));
    assert_eq!(names(regions), [vec!["parse_key", "loads"]]);
    assert!(regions_of(MODULE, &select(&["skip", "walk"]))[0] == [key(MODULE, "walk")]);
}

#[test]
fn a_self_recursive_function_is_a_region_of_one() {
    assert_eq!(names(regions_of(MODULE, &select(&["walk"]))), [vec!["walk"]]);
    assert!(regions_of(MODULE, &select(&["skip"])).is_empty(), "no edge, no region");
}

#[test]
fn a_rebound_global_is_no_edge() {
    // `patched` is rebound by an assignment, `declared` by a `global`
    // declaration; `star` takes `*args`, so no compiled call reaches its
    // body directly.
    for (caller, callee) in [
        ("uses_patched", "patched"),
        ("calls_declared", "declared"),
        ("calls_star", "star"),
    ] {
        assert!(
            regions_of(MODULE, &select(&[caller, callee])).is_empty(),
            "{caller} -> {callee} is no edge"
        );
    }
}

#[test]
fn a_local_a_lambda_or_a_comprehension_target_is_no_edge() {
    for caller in ["shadowing", "through_lambda", "comprehension_target"] {
        assert!(
            regions_of(MODULE, &select(&[caller, "loads", "skip"]))
                .iter()
                .all(|members| !members.contains(&key(MODULE, caller))),
            "{caller} calls no selected function statically"
        );
    }
}

#[test]
fn a_method_edge_follows_the_receiver() {
    assert_eq!(
        names(regions_of(MODULE, &select(&["Parser.parse", "Parser.value", "helper"]))),
        [vec!["Parser.parse", "Parser.value", "helper"]],
        "`self.value(...)` and the module global `helper`"
    );
    // A rebound receiver, another object, a decorated callee.
    for caller in ["Parser.rebinds", "Parser.other"] {
        assert!(regions_of(MODULE, &select(&[caller, "Parser.value"])).is_empty(), "{caller}");
    }
    assert!(regions_of(MODULE, &select(&["Parser.calls_static", "Parser.static"])).is_empty());
}

#[test]
fn an_entry_is_a_member_called_from_outside_its_region() {
    let members = [key(MODULE, "skip"), key(MODULE, "loads")];
    let (skip, loads) = (members[0].clone(), members[1].clone());
    // No profile: every member may be called from bytecode.
    assert_eq!(
        regions::report_line("m", &members, &BTreeMap::new()),
        "region: m: skip, loads (entries: skip, loads)"
    );
    let calls: BTreeMap<Key, Calls> = [
        (
            skip.clone(),
            Calls {
                total: 30,
                from: [(loads.clone(), 30)].into(),
            },
        ),
        (loads.clone(), Calls { total: 2, from: BTreeMap::new() }),
    ]
    .into();
    assert_eq!(regions::report_line("m", &members, &calls), "region: m: skip, loads (entries: loads)");
    let mut never: BTreeMap<Key, Calls> = calls.clone();
    never.insert(loads, Calls::default());
    assert_eq!(regions::report_line("m", &members, &never), "region: m: skip, loads (entries: none)");
    let mut outside = calls;
    outside.get_mut(&skip).unwrap().total = 31;
    assert_eq!(
        regions::report_line("m", &members, &outside),
        "region: m: skip, loads (entries: skip, loads)"
    );
}
