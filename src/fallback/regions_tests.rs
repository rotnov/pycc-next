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

@functools.cache
def decorated_callee(x):
    return x

def calls_decorated(x):
    return decorated_callee(x)

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
    // Found in the source after a newline, so a first-line `def` is found.
    let at = format!("\n{source}").find(&needle).expect("the function is in the fixture");
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
    // A rebound receiver, another object, a static method.
    for caller in ["Parser.rebinds", "Parser.other"] {
        assert!(regions_of(MODULE, &select(&[caller, "Parser.value"])).is_empty(), "{caller}");
    }
    assert!(regions_of(MODULE, &select(&["Parser.calls_static", "Parser.static"])).is_empty());
}

#[test]
fn a_decorated_callee_is_no_edge() {
    assert!(
        regions_of(MODULE, &select(&["calls_decorated", "decorated_callee"])).is_empty(),
        "the module binds `decorated_callee` to what `functools.cache` returns"
    );
}

/// The key of `qualname`, whose `def` is the first line of `source` that
/// contains `needle`.
fn key_at(source: &str, qualname: &str, needle: &str) -> Key {
    let at = source.find(needle).expect("the needle is in the source");
    (qualname.to_string(), line_of(source, at as u32) as u32)
}

#[test]
fn an_implicit_static_or_class_method_has_no_receiver() {
    for method in ["__new__", "__init_subclass__", "__class_getitem__"] {
        let source = format!(
            "class Node:\n    def {method}(cls, o, n):\n        cls.setup(o, n)\n        return o\n\n    \
             def setup(self, n):\n        return n\n\n    def make(self, o, n):\n        return self.setup(n)\n"
        );
        let caller = key(&source, &format!("Node.{method}"));
        let setup = key(&source, "Node.setup");
        assert!(
            regions_of(&source, &[caller, setup.clone()].into()).is_empty(),
            "`cls` of {method} is no instance"
        );
        // An instance method's `self.setup(...)` is an edge.
        assert_eq!(regions_of(&source, &[key(&source, "Node.make"), setup].into()).len(), 1, "{method}");
    }
}

#[test]
fn a_method_edge_stays_within_its_own_class_statement() {
    // Two top-level classes of one name.
    let source = "class C:\n    def a(self):\n        return self.b()\n\nclass C:\n    def b(self):\n        return 1\n";
    let selected: BTreeSet<Key> = [key(source, "C.a"), key(source, "C.b")].into();
    assert!(regions_of(source, &selected).is_empty(), "two top-level `class C`");
    // A class in a block and a top-level one of the same name.
    let source = "import os\nif os.sep:\n    class C:\n        def a(self):\n            return self.b()\n\n\
                  class C:\n    def b(self):\n        return 1\n";
    let selected: BTreeSet<Key> = [key_at(source, "C.a", "def a("), key_at(source, "C.b", "def b(")].into();
    assert!(regions_of(source, &selected).is_empty(), "`class C` in a block");
    // Two nested classes of one name in one class.
    let source = "class Outer:\n    class Inner:\n        def b(self):\n            return 1\n\n    \
                  class Inner:\n        def a(self):\n            return self.b()\n";
    let selected: BTreeSet<Key> =
        [key_at(source, "Outer.Inner.a", "def a("), key_at(source, "Outer.Inner.b", "def b(")].into();
    assert!(regions_of(source, &selected).is_empty(), "two nested `class Inner`");
    // A nested class bound once is that class.
    let source = "class Outer:\n    class Inner:\n        def b(self):\n            return 1\n\n        \
                  def a(self):\n            return self.b()\n";
    let selected: BTreeSet<Key> =
        [key_at(source, "Outer.Inner.a", "def a("), key_at(source, "Outer.Inner.b", "def b(")].into();
    assert_eq!(regions_of(source, &selected).len(), 1, "one nested `class Inner`");
}

#[test]
fn a_walrus_in_a_default_binds_in_the_enclosing_scope() {
    // In the caller's scope: `f` is the caller's local.
    let source = "def other():\n    return 1\n\ndef f():\n    return 2\n\n\
                  def caller():\n    g = lambda x=(f := other): x\n    return f()\n\n\
                  def caller_def():\n    def g(x=(f := other)):\n        return x\n    return f()\n";
    for caller in ["caller", "caller_def"] {
        let selected: BTreeSet<Key> = [key(source, caller), key(source, "f")].into();
        assert!(regions_of(source, &selected).is_empty(), "{caller}: `f` is local");
    }
    // At module scope: `f` is rebound.
    let source = "def f():\n    return 2\n\ndef g(a=(f := len)):\n    return a\n\n\
                  def caller():\n    return f()\n";
    let selected: BTreeSet<Key> = [key(source, "caller"), key(source, "f")].into();
    assert!(regions_of(source, &selected).is_empty(), "`f` is rebound by `g`'s default");
}

#[test]
fn what_a_nested_scope_evaluates_outside_its_body_is_the_compiled_bodys() {
    let source = "def deco(x):\n    return x\n\ndef base():\n    return object\n\n\
                  def dflt():\n    return 0\n\n\
                  def decorates():\n    @deco\n    def g():\n        return 1\n    return g\n\n\
                  def calls_in_decorator():\n    @(deco(1) and deco)\n    def g():\n        return 1\n    return g\n\n\
                  def calls_in_default():\n    def g(x=dflt()):\n        return x\n    return g\n\n\
                  def calls_in_lambda_default():\n    return lambda x=dflt(): x\n\n\
                  def calls_in_base():\n    class K(base()):\n        pass\n    return K\n\n\
                  def calls_in_body():\n    def g():\n        return dflt()\n    return g\n";
    for (caller, callee) in [
        ("calls_in_decorator", "deco"),
        ("calls_in_default", "dflt"),
        ("calls_in_lambda_default", "dflt"),
        ("calls_in_base", "base"),
    ] {
        let selected: BTreeSet<Key> = [key(source, caller), key(source, callee)].into();
        assert_eq!(regions_of(source, &selected).len(), 1, "{caller} -> {callee}");
    }
    // A decorator that is no call, and a call in the nested body, are no edges.
    for (caller, callee) in [("decorates", "deco"), ("calls_in_body", "dflt")] {
        let selected: BTreeSet<Key> = [key(source, caller), key(source, callee)].into();
        assert!(regions_of(source, &selected).is_empty(), "{caller} -> {callee}");
    }
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
