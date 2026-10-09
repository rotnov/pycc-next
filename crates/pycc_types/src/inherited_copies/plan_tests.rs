use crate::inherited_copies::plan_copies;

fn plan(source: &str) -> Vec<(String, String)> {
    let module = pycc_parser::parse(source).expect("test fixture must parse");
    let hir = pycc_hir::lower_checked(&module).expect("test fixture must lower");
    plan_copies(&hir)
        .into_iter()
        .map(|c| (c.receiver, c.origin_name))
        .collect()
}

fn pairs(items: &[(&str, &str)]) -> Vec<(String, String)> {
    items
        .iter()
        .map(|(a, b)| ((*a).to_string(), (*b).to_string()))
        .collect()
}

#[test]
fn reach_through_an_overridden_member_needs_a_copy() {
    assert_eq!(
        plan(
            "class A:\n    def m(self) -> int:\n        return 1\n    def g(self) -> int:\n        \
             return self.m()\nclass B(A):\n    def m(self) -> int:\n        return 2\n"
        ),
        pairs(&[("B", "A.g")])
    );
}

#[test]
fn reach_is_transitive_through_another_needed_body() {
    assert_eq!(
        plan(
            "class A:\n    def m(self) -> int:\n        return 1\n    def g(self) -> int:\n        \
             return self.m()\n    def h(self) -> int:\n        return self.g()\n\
             class B(A):\n    def m(self) -> int:\n        return 2\n"
        ),
        pairs(&[("B", "A.g"), ("B", "A.h")])
    );
}

#[test]
fn a_class_attribute_override_is_a_difference() {
    assert_eq!(
        plan(
            "class A:\n    k = 1\n    def g(self) -> int:\n        return self.k\n\
             class B(A):\n    k = 2\n"
        ),
        pairs(&[("B", "A.g")])
    );
}

#[test]
fn a_diamond_super_target_is_planned_for_the_receiver() {
    // `D`'s own `f` reaches `B.f` through `super()`, whose own `super()`
    // selects `C` for `D` but `A` for `B`.
    let planned = plan(
        "class A:\n    def f(self) -> str:\n        return \"A\"\n\
         class B(A):\n    def f(self) -> str:\n        return \"B\" + super().f()\n\
         class C(A):\n    def f(self) -> str:\n        return \"C\"\n\
         class D(B, C):\n    def f(self) -> str:\n        return \"D\" + super().f()\n",
    );
    assert_eq!(planned, pairs(&[("D", "B.f")]));
}

#[test]
fn identity_arms_need_a_copy() {
    assert_eq!(
        plan(
            "class A:\n    def t(self) -> bool:\n        return isinstance(self, B)\n    \
             @classmethod\n    def make(cls) -> A:\n        return cls()\nclass B(A):\n    pass\n"
        ),
        pairs(&[("B", "A.t"), ("B", "A.make.classmethod")])
    );
}

#[test]
fn an_escape_needs_a_copy_only_when_something_differs() {
    let base = "class A:\n    def m(self) -> int:\n        return 1\n    def me(self) -> A:\n        \
                return self\n";
    assert!(plan(&format!("{base}class B(A):\n    pass\n")).is_empty());
    assert_eq!(
        plan(&format!(
            "{base}class B(A):\n    def m(self) -> int:\n        return 2\n"
        )),
        pairs(&[("B", "A.me")])
    );
}

#[test]
fn receivers_without_instances_are_never_planned() {
    // An abstract receiver, and a user exception class without its own
    // constructor, are never instantiated as a plain object.
    assert!(
        plan(
            "from abc import ABC, abstractmethod\nclass A:\n    def m(self) -> int:\n        \
             return 1\n    def g(self) -> int:\n        return self.m()\n\
             class B(A, ABC):\n    def m(self) -> int:\n        return 2\n    @abstractmethod\n    \
             def h(self) -> int:\n        ...\n"
        )
        .is_empty()
    );
    assert!(
        plan(
            "class E(ValueError):\n    def m(self) -> int:\n        return 1\n    \
             def g(self) -> int:\n        return self.m()\nclass F(E):\n    \
             def m(self) -> int:\n        return 2\n"
        )
        .is_empty()
    );
}

#[test]
fn class_level_hooks_and_abstract_stubs_are_never_copied() {
    // `C` differs from `A` in `k`, but neither the hook nor the abstract
    // stub is a candidate; only `g`, which reads `k`, is copied.
    assert_eq!(
        plan(
            "from abc import ABC, abstractmethod\nclass A(ABC):\n    k = 1\n    \
             def __init_subclass__(self) -> None:\n        pass\n    @abstractmethod\n    \
             def h(self) -> int:\n        ...\n    def g(self) -> int:\n        return self.k\n\
             class C(A):\n    k = 2\n    def h(self) -> int:\n        return self.k\n"
        ),
        pairs(&[("C", "A.g")])
    );
}
