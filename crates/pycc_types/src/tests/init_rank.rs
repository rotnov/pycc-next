//! #966: the checker ranks a D-225 implicit `__init__` last.
//!
//! Three seams in this crate rank constructors by MRO position, and all
//! three must agree with `pycc_mir`'s lowering or the checker and the
//! lowering resolve different constructors:
//!
//! - `class::binding::resolve_instantiation` -- `C()`;
//! - `class::resolve_super_method_call` -- `super().__init__()`;
//! - `exception::reject_own_constructor` -- D-189 rule 4 raisability.
//!
//! These are in-crate tests because the integration tests under `tests/` do
//! not count toward this crate's own regions (D-014, `docs/TESTING.md`).
//! The end-to-end behaviour they stand for lives in
//! `tests/issue_966_inherited_init_rank.rs` and
//! `tests/issue_912_no_init_class.rs`.

use super::check_source;

/// `class A: pass` / `class B: __init__(self, v: int)` / `class C(A, B)`.
///
/// The arity is the discriminator: `C(5)` type-checks only if `B`'s real
/// constructor is the one resolved. Before #966 `A`'s implicit stub won and
/// this was `T0021`.
#[test]
fn resolve_instantiation_skips_an_implicit_constructor_for_a_later_real_one() {
    check_source(
        "class A:\n    def ping(self) -> int:\n        return 9\n\n\nclass B:\n    def __init__(self, v: int) -> None:\n        self.z = v\n\n\nclass C(A, B):\n    pass\n\n\nc = C(5)\nprint(c.z)\n",
    )
    .expect("`B.__init__` is the resolved constructor, so one argument is correct");
}

/// The fallback pass in `resolve_instantiation`: an MRO whose every
/// constructor is implicit still resolves, and still takes zero arguments.
#[test]
fn resolve_instantiation_falls_back_when_every_constructor_is_implicit() {
    check_source(
        "class A:\n    pass\n\n\nclass B:\n    pass\n\n\nclass C(A, B):\n    pass\n\n\nc = C()\n",
    )
    .expect("with only implicit constructors, the implicit one is correct");

    let err = check_source(
        "class A:\n    pass\n\n\nclass B:\n    pass\n\n\nclass C(A, B):\n    pass\n\n\nc = C(1)\n",
    )
    .expect_err("the implicit constructor still takes no arguments");
    assert_eq!(
        err.code, "T0021",
        "an arity error, not a resolution failure"
    );
}

/// The same skip on the `super()` seam: `super().__init__(5)` must check
/// against `B`'s signature, not `A`'s implicit zero-parameter one.
#[test]
fn resolve_super_method_call_skips_an_implicit_constructor() {
    check_source(
        "class A:\n    pass\n\n\nclass B:\n    def __init__(self, v: int) -> None:\n        self.z = v\n\n\nclass C(A, B):\n    def __init__(self) -> None:\n        super().__init__(5)\n\n\nc = C()\n",
    )
    .expect("`super().__init__(5)` resolves to `B.__init__`, which takes one argument");
}

/// The mandatory fallback pass on the `super()` seam. `class A: pass` /
/// `class C(A)` calling `super().__init__()` has only the implicit
/// constructor above it; without the second pass this is a spurious
/// `T0044`.
#[test]
fn resolve_super_method_call_falls_back_to_the_only_implicit_constructor() {
    check_source(
        "class A:\n    pass\n\n\nclass C(A):\n    def __init__(self) -> None:\n        self.q = 4\n        super().__init__()\n\n\nc = C()\n",
    )
    .expect("the implicit constructor is the only candidate and must be reached");
}

/// The `__init__` skip is gated on the method name, so a flagged class is
/// still a perfectly ordinary owner of any other method reached through
/// `super()`.
#[test]
fn resolve_super_method_call_leaves_other_methods_on_a_flagged_class_alone() {
    check_source(
        "class A:\n    def ping(self) -> int:\n        return 9\n\n\nclass C(A):\n    def ping(self) -> int:\n        return super().ping()\n\n\nc = C()\nprint(c.ping())\n",
    )
    .expect("`A` carries the implicit constructor but still owns `ping`");

    let err = check_source(
        "class A:\n    pass\n\n\nclass C(A):\n    def ping(self) -> int:\n        return super().ping()\n",
    )
    .expect_err("`ping` exists nowhere above `C`");
    assert_eq!(
        err.code, "T0044",
        "an unknown-member error is still reported"
    );
}

/// D-189 rule 4 in `exception::reject_own_constructor`, and the behaviour
/// #966 flipped: `class MyError(Base, Exception)` where `Base` carries only
/// the implicit constructor is now raisable, because the walk ranks that
/// implicit constructor last and reaches `Exception.__init__`.
#[test]
fn reject_own_constructor_skips_an_implicit_constructor_on_an_ancestor() {
    check_source(
        "class Base:\n    pass\n\n\nclass MyError(Base, Exception):\n    pass\n\n\nraise MyError(\"boom\")\n",
    )
    .expect("the implicit constructor on `Base` no longer decides raisability");
}

/// The rejecting arm is unchanged: a *real* `__init__` anywhere along the
/// MRO ahead of `Exception`'s is still `C0001`, because only the implicit
/// constructor is re-ranked.
#[test]
fn reject_own_constructor_still_rejects_a_real_ancestor_constructor() {
    let err = check_source(
        "class Base:\n    def __init__(self) -> None:\n        self.z = 1\n\n\nclass MyError(Base, Exception):\n    pass\n\n\nraise MyError(\"boom\")\n",
    )
    .expect_err("a real ancestor constructor is still a capability gap");
    assert_eq!(err.code, "C0001");
    assert!(
        err.message
            .contains("declares or inherits an `__init__` other than `Exception`'s"),
        "unexpected message: {}",
        err.message
    );
}
