use super::*;

use pycc_hir::{HirItem, HirModule};

fn lower(source: &str) -> HirModule {
    let module = pycc_parser::parse(source).expect("test fixture must parse");
    pycc_hir::lower_checked(&module).expect("test fixture must lower")
}

/// Walks the body of item `name` with receiver `receiver`.
fn walk(source: &str, name: &str, receiver: Option<&str>) -> (BodyFacts, BTreeSet<String>) {
    let hir = lower(source);
    let body = hir
        .items
        .iter()
        .find_map(|item| match item {
            HirItem::Function { name: n, body, .. } if n == name => Some(body.clone()),
            _ => None,
        })
        .expect("the item exists");
    let mut walker = Walker::new(receiver);
    walker.stmts(&body);
    (walker.facts, walker.observers)
}

fn set(items: &[&str]) -> BTreeSet<String> {
    items.iter().map(|s| (*s).to_string()).collect()
}

#[test]
fn receiver_members_super_members_and_type_tests_are_recorded() {
    let source = "class A:\n    def m(self) -> int:\n        return 1\n    def f(self) -> int:\n        \
                  return 0\nclass K(A):\n    pass\nclass B(A):\n    def f(self) -> int:\n        \
                  if isinstance(self, K):\n            return self.m()\n        \
                  n = super().f() + self.m()\n        return n\n";
    let (facts, observers) = walk(source, "B.f", Some("self"));
    assert_eq!(facts.self_refs, set(&["m"]));
    assert_eq!(facts.super_refs, set(&["f"]));
    assert_eq!(facts.receiver_type_tests, set(&["K"]));
    assert!(!facts.bare_use);
    assert!(!facts.constructs_via_receiver);
    assert_eq!(observers, set(&["K"]));
}

#[test]
fn a_bare_receiver_use_and_a_receiver_construction_are_recorded() {
    let source = "class A:\n    def me(self) -> A:\n        return self\n    @classmethod\n    \
                  def make(cls) -> A:\n        return cls()\n";
    let (facts, _) = walk(source, "A.me", Some("self"));
    assert!(facts.bare_use);
    let (facts, _) = walk(source, "A.make.classmethod", Some("cls"));
    assert!(facts.constructs_via_receiver);
}

#[test]
fn a_type_self_construction_is_recorded_only_with_a_receiver() {
    let source = "class A:\n    def __init__(self, n: int) -> None:\n        self.n = n\n    \
                  def again(self) -> int:\n        return type(self)(self.n).n\n";
    let (facts, _) = walk(source, "A.again", Some("self"));
    assert!(facts.constructs_via_receiver);
    assert_eq!(facts.self_refs, set(&["n"]));
    let (facts, _) = walk(source, "A.again", None);
    assert!(!facts.constructs_via_receiver);
}

#[test]
fn a_type_test_on_another_subject_is_only_an_observer() {
    let source = "class A:\n    pass\nclass B(A):\n    pass\n\
                  def f(x: A) -> bool:\n    return isinstance(x, B)\n";
    let (facts, observers) = walk(source, "f", None);
    assert_eq!(facts, BodyFacts::default());
    assert_eq!(observers, set(&["B"]));
}

#[test]
fn nested_shapes_are_walked_for_receiver_uses() {
    let source = "class A:\n    def __init__(self) -> None:\n        self.xs: list[int] = []\n    \
                  def f(self) -> int:\n        total = 0\n        for i in range(3):\n            \
                  total += i\n        while total > 100:\n            total -= 1\n        \
                  xs = self.xs\n        ys = [v for v in xs]\n        s = f\"{self.xs}\"\n        \
                  d = {\"k\": len(ys)}\n        t = (1, 2)\n        \
                  self.xs.append(d.get(\"k\", 0))\n        \
                  return total + len(s) + t[0] + -1\n";
    let (facts, _) = walk(source, "A.f", Some("self"));
    assert_eq!(facts.self_refs, set(&["xs"]));
    assert!(!facts.bare_use);
}

/// #1395: every part of a conditional expression is walked.
#[test]
fn every_part_of_a_conditional_expression_is_walked() {
    let source = "class A:\n    def __init__(self) -> None:\n        self.a = 1\n        \
                  self.b = 2\n        self.c = True\n    \
                  def f(self) -> int:\n        return self.a if self.c else self.b\n";
    let (facts, _) = walk(source, "A.f", Some("self"));
    assert_eq!(facts.self_refs, set(&["a", "b", "c"]));
    assert!(!facts.bare_use);
}

#[test]
fn merging_unions_every_fact() {
    let mut a = BodyFacts {
        self_refs: set(&["x"]),
        ..BodyFacts::default()
    };
    a.merge(BodyFacts {
        self_refs: set(&["y"]),
        super_refs: set(&["z"]),
        receiver_type_tests: set(&["T"]),
        constructs_via_receiver: true,
        bare_use: true,
    });
    assert_eq!(a.self_refs, set(&["x", "y"]));
    assert_eq!(a.super_refs, set(&["z"]));
    assert_eq!(a.receiver_type_tests, set(&["T"]));
    assert!(a.constructs_via_receiver && a.bare_use);
}

/// Part 1 of #1255: no statement-form comprehension carries a
/// `CompIter::Iterable` today (`comp_assign_stmt` routes it to a plain
/// `Assign`), but the walker still visits its expression, so a receiver use
/// inside one cannot escape the facts. Pinned directly, since no source
/// reaches the arm.
#[test]
fn an_iterable_comprehension_source_is_walked() {
    let stmt = pycc_hir::HirStmt::ListCompAssign {
        target: "xs".to_string(),
        var: "_v0".to_string(),
        iter: pycc_hir::CompIter::Iterable(Box::new(pycc_hir::HirExpr::Name("self".to_string()))),
        cond: None,
        elt: Box::new(pycc_hir::HirExpr::Name("_v0".to_string())),
    };
    let mut walker = Walker::new(Some("self"));
    walker.stmts(&[stmt]);
    assert!(walker.facts.bare_use);
}

/// #1457: the base of an attribute `del` is walked, so a receiver member
/// read there is recorded.
#[test]
fn an_attribute_delete_base_is_walked() {
    let source = "class A:\n    def __init__(self) -> None:\n        self.xs = 1\n    \
                  def f(self) -> None:\n        del self.xs.n\n";
    let (facts, _) = walk(source, "A.f", Some("self"));
    assert_eq!(facts.self_refs, set(&["xs"]));
    assert!(!facts.bare_use);
}
