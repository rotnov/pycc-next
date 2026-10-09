//! `typing.cast(T, value)` unit tests for the type-checking crate root.
//!
//! Extracted verbatim from `tests.rs` under AGENTS.md's decomposability rule
//! (part of #695, which tracks decomposing that oversized file). These are the
//! tests that exercise the #767 special-cased `typing.cast` builtin call, so
//! the split keeps one cohesive feature in one place. As a child module this
//! still sees the parent's private items directly through `use super::*`, so
//! nothing needed widened visibility; only the tests' location changed.

use super::*;

// -- #767: `typing.cast(T, value)` as a special-cased builtin call ------

#[test]
fn cast_to_a_builtin_scalar_type_checks_and_infers_that_type() {
    // #767: the core case -- `from typing import cast` resolves (the
    // registry entry) and `cast(int, x)` type-checks with type `int`, so
    // it satisfies an `-> int` return annotation.
    check_source(
        "from typing import cast\ndef f(x: int) -> int:\n    return cast(int, x)\nprint(f(1))\n",
    )
    .unwrap();
}

#[test]
fn cast_to_each_builtin_scalar_type_infers_that_scalar() {
    // #767: `cast_target_ty`'s four builtin-scalar arms, each exercised
    // through a return annotation that only the correct `Ty` satisfies.
    for (target, annotation, value) in [
        ("int", "int", "1"),
        ("float", "float", "1.5"),
        ("bool", "bool", "True"),
        ("str", "str", "\"s\""),
    ] {
        check_source(&format!(
            "from typing import cast\ndef f() -> {annotation}:\n    return cast({target}, {value})\nprint(f())\n"
        ))
        .unwrap_or_else(|e| panic!("cast({target}, ...) should infer `{annotation}`, got: {e:?}"));
    }
}

#[test]
fn cast_to_a_user_defined_class_type_checks() {
    // #767: `cast_target_ty`'s fallthrough arm -- a non-builtin bare name
    // that `validate_class_name` accepts maps to `Ty::Instance`, so the
    // cast result supports the class's own attributes.
    check_source(
        "from typing import cast\nclass C:\n    def __init__(self, v: int) -> None:\n        self.v = v\ndef f(c: C) -> int:\n    d = cast(C, c)\n    return d.v\nprint(f(C(7)))\n",
    )
    .unwrap();
}

#[test]
fn cast_up_to_a_base_class_checks() {
    // #767 review fix (second pass, D-198): a cast from a class to one of
    // its own MRO ancestors changes no representation and narrows no
    // attribute layout -- `Derived`'s slots are a superset of `Base`'s, so
    // whatever MIR keeps tracking after erasure already supports every
    // attribute the checker lets code reach through the cast result.
    check_source(
        "from typing import cast\nclass Base:\n    def __init__(self, a: int) -> None:\n        self.a = a\nclass Derived(Base):\n    def __init__(self, a: int, b: int) -> None:\n        self.a = a\n        self.b = b\ndef f(d: Derived) -> Base:\n    return cast(Base, d)\nprint(f(Derived(1, 2)).a)\n",
    )
    .unwrap();
}

#[test]
fn cast_down_to_a_derived_class_is_c0001() {
    // #767 review fix (second pass, D-198): a genuine down-cast (the
    // rejected direction the first review pass's `cast_shares_representation`
    // wrongly admitted) is unsound under erasure -- `pycc_mir` never learns
    // the checker-verified target `Derived`, so it keeps resolving attribute
    // access against the value's real MIR type `Base`, which lacks `Derived`'s
    // extra slot. Accepting this at the type level, as the first pass did,
    // reaches either a `pycc_mir` panic (unannotated binding/inline access)
    // or an out-of-bounds `pycc_rt` instance-slot abort at runtime
    // (`AnnAssign`, which re-anchors the MIR type to `Derived` without the
    // object ever having been allocated with `Derived`'s slots). Rejecting it
    // here, at `check_cast`, closes the hole before either path is reached.
    let err = check_source(
        "from typing import cast\nclass Base:\n    def __init__(self, a: int) -> None:\n        self.a = a\nclass Derived(Base):\n    def __init__(self, a: int, b: int) -> None:\n        self.a = a\n        self.b = b\ndef f(base: Base) -> int:\n    return cast(Derived, base).b\nprint(f(Base(1)))\n",
    )
    .unwrap_err();
    assert_eq!(err.code, "C0001");
    assert!(
        err.message.contains("narrow the value's attribute layout"),
        "expected the layout message, got: {}",
        err.message
    );
}

#[test]
fn cast_down_to_a_derived_class_via_a_plain_assignment_binding_is_c0001() {
    // #771: the exact filed repro. Binding a rejected `cast(...)` result to
    // a plain (non-annotated) local before reading it used to report a
    // misleading `T0021` ("`d` is not bound before this use") instead of
    // the real `C0001` down-cast rejection that the equivalent inline form
    // (`cast_down_to_a_derived_class_is_c0001` above) already reported
    // correctly. Root cause (D-199): `check()` discards the concrete
    // path's correct `C0001` on `Err` and unconditionally falls back to
    // `infer_function_signatures_with_solver_all`, whose `Assign` arm left `d`
    // wholly untracked because the `Cast` arm returns `Ok(None)` for a
    // non-scalar target -- so `return d.b` misfired as an unbound local in
    // that second pass before the first pass's real diagnostic could
    // surface. `opaque_bindings` tracks `d` as definitely-but-opaquely
    // bound instead, so the solver pass no longer errors and the real
    // `C0001` (computed by the first, concrete pass) is the one returned.
    let err = check_source(
        "from typing import cast\nclass Base:\n    def __init__(self, a: int) -> None:\n        self.a = a\nclass Derived(Base):\n    def __init__(self, a: int, b: int) -> None:\n        self.a = a\n        self.b = b\ndef f(base: Base) -> int:\n    d = cast(Derived, base)\n    return d.b\nprint(f(Base(1)))\n",
    )
    .unwrap_err();
    assert_eq!(err.code, "C0001");
    assert!(
        err.message.contains("narrow the value's attribute layout"),
        "expected the layout message, got: {}",
        err.message
    );
}

#[test]
fn cast_down_to_a_derived_class_via_a_walrus_binding_is_c0001() {
    // #771/D-199 deep-review follow-up: this branch's `opaque_bindings`
    // mechanism was developed before PEP 572 (#774) walrus support reached
    // this crate. When the two features were rebased together,
    // `bind_named_expr_targets`'s own `HirExpr::NamedExpr` arm (the walrus
    // equivalent of `collect_block_constraints`'s `Assign` arm) was found
    // not to mirror the `opaque_bindings` tracking `Assign` gained --
    // `(d := cast(Derived, base))` left `d` untracked in exactly the same
    // way a plain `d = cast(Derived, base)` did before this decision, so a
    // later read misreported `T0021` instead of the real `C0001` down-cast
    // rejection. This is the walrus counterpart of
    // `cast_down_to_a_derived_class_via_a_plain_assignment_binding_is_c0001`
    // above, pinning that the fix now covers both binding forms.
    let err = check_source(
        "from typing import cast\nclass Base:\n    def __init__(self, a: int) -> None:\n        self.a = a\nclass Derived(Base):\n    def __init__(self, a: int, b: int) -> None:\n        self.a = a\n        self.b = b\ndef f(base: Base) -> int:\n    (d := cast(Derived, base))\n    return d.b\nprint(f(Base(1)))\n",
    )
    .unwrap_err();
    assert_eq!(err.code, "C0001");
    assert!(
        err.message.contains("narrow the value's attribute layout"),
        "expected the layout message, got: {}",
        err.message
    );
}

#[test]
fn an_opaque_assignment_whose_value_is_never_read_still_compiles() {
    // Issue #771 positive-path coverage: adding `opaque_bindings` tracking
    // for an unconditional assignment whose RHS the solver can't represent
    // as a term must not introduce a new false positive when that binding
    // is simply never read afterward. `y` is assigned from `d.get("a", 0)`
    // (a `DictGetOrDefault`, one of the affected-site inventory's
    // constructs) inside an unannotated private helper (the return type is
    // `Ty::Infer`, forcing the whole module through the solver path per
    // `concrete_function_environment`), so the solver runs, but `y` is
    // discarded rather than returned -- only the literal `1` determines
    // the inferred return type.
    check_source(
        "def _h():\n    d = {\"a\": 1}\n    y = d.get(\"a\", 0)\n    return 1\nprint(_h())\n",
    )
    .unwrap_or_else(|e| panic!("expected a clean compile, got: {e:?}"));
}

#[test]
fn cast_between_two_unrelated_class_types_is_c0001() {
    // #767 review fix (second pass, D-198): two classes with no MRO
    // relationship are neither an up-cast nor an identity cast, so this is
    // rejected on the same layout-soundness grounds as a genuine down-cast --
    // neither class's slot layout is guaranteed to be a superset of the
    // other's.
    let err = check_source("from typing import cast\nclass A:\n    def __init__(self, v: int) -> None:\n        self.v = v\nclass B:\n    def __init__(self, v: int) -> None:\n        self.v = v\ndef f(a: A) -> B:\n    return cast(B, a)\nprint(f(A(1)).v)\n")
        .unwrap_err();
    assert_eq!(err.code, "C0001");
    assert!(
        err.message.contains("narrow the value's attribute layout"),
        "expected the layout message, got: {}",
        err.message
    );
}

#[test]
fn cast_up_across_an_overridden_method_is_c0001() {
    // #767 review fix (third pass, D-198): the deep-reviewer's remaining
    // blocker on the up-cast subset -- `pycc_mir` resolves method calls
    // statically from the cast result's declared type
    // (`crates/pycc_mir/src/expr.rs`, no vtable), so `cast(Base, d)` followed
    // by a call to a method `Derived` overrides would silently run `Base`'s
    // implementation instead of CPython's dynamically-dispatched override.
    // `check_cast` rejects the up-cast itself rather than let that dispatch
    // divergence reach codegen undiagnosed.
    let err = check_source(
        "from typing import cast\nclass Base:\n    def __init__(self, a: int) -> None:\n        self.a = a\n    def describe(self) -> int:\n        return self.a\nclass Derived(Base):\n    def __init__(self, a: int, b: int) -> None:\n        self.a = a\n        self.b = b\n    def describe(self) -> int:\n        return self.a + self.b\ndef f(d: Derived) -> int:\n    return cast(Base, d).describe()\nprint(f(Derived(1, 2)))\n",
    )
    .unwrap_err();
    assert_eq!(err.code, "C0001");
    assert!(
        err.message.contains("`describe`") && err.message.contains("statically resolve"),
        "expected the method-dispatch message naming `describe`, got: {}",
        err.message
    );
}

#[test]
fn cast_up_to_a_base_class_with_no_overridden_methods_still_checks() {
    // #767 review fix (third pass, D-198): the method-override check must
    // not reject an up-cast merely because the subclass defines its own
    // `__init__` (the ordinary case for any real class hierarchy) or its own
    // non-overriding methods -- only an up-cast that crosses an actual
    // override boundary is unsound.
    check_source(
        "from typing import cast\nclass Base:\n    def __init__(self, a: int) -> None:\n        self.a = a\n    def describe(self) -> int:\n        return self.a\nclass Derived(Base):\n    def __init__(self, a: int, b: int) -> None:\n        self.a = a\n        self.b = b\n    def extra(self) -> int:\n        return self.b\ndef f(d: Derived) -> int:\n    b = cast(Base, d)\n    return b.describe()\nprint(f(Derived(1, 2)))\n",
    )
    .unwrap();
}

#[test]
fn cast_up_across_an_override_in_an_intermediate_ancestor_is_c0001() {
    // #767 review fix (4th pass, D-198): every existing `OverriddenMethod`
    // test puts the override on the value's own class (position 0 of
    // `from_def.mro[..to_pos]`). This pins the 3-level case where the
    // override instead lives in an ancestor strictly between the value's
    // class and the cast target -- `A` declares `describe`, `B(A)` overrides
    // it, `C(B)` does not -- so `cast(A, c)` must still be rejected even
    // though `C` itself, the value's own class, overrides nothing.
    let err = check_source(
        "from typing import cast\nclass A:\n    def __init__(self, a: int) -> None:\n        self.a = a\n    def describe(self) -> int:\n        return self.a\nclass B(A):\n    def __init__(self, a: int, b: int) -> None:\n        self.a = a\n        self.b = b\n    def describe(self) -> int:\n        return self.a + self.b\nclass C(B):\n    def __init__(self, a: int, b: int, c: int) -> None:\n        self.a = a\n        self.b = b\n        self.c = c\ndef f(v: C) -> int:\n    return cast(A, v).describe()\nprint(f(C(1, 2, 3)))\n",
    )
    .unwrap_err();
    assert_eq!(err.code, "C0001");
    assert!(
        err.message.contains("`describe`") && err.message.contains("statically resolve"),
        "expected the method-dispatch message naming `describe`, got: {}",
        err.message
    );
}

#[test]
fn cast_changing_representation_is_c0001() {
    // #767 review fix (blocker, D-198): a `cast` whose target type's runtime
    // representation differs from the value's own inferred type used to
    // type-check unconditionally, then reach either a codegen-internal-error
    // panic (debug build, via the local-type-drift `debug_assert_eq!`) or
    // silently misinterpreted bits (release build) once `pycc_mir` elided
    // the whole call to the value alone. `check_cast` now rejects it with
    // `C0001`, the versioned capability code -- the rejection is pycc's own
    // erasure limit, not a claim that the program is ill-typed Python.
    let err = check_source("from typing import cast\ny = cast(str, 5)\nprint(y)\n").unwrap_err();
    assert_eq!(err.code, "C0001");
    assert!(
        err.message
            .contains("would change the value's runtime representation"),
        "expected the representation message, got: {}",
        err.message
    );
}

#[test]
fn cast_changing_representation_in_an_annassign_is_c0001() {
    // #767 review fix (blocker): the exact repro the review found -- an
    // `AnnAssign` masks the representation mismatch from a casual read
    // (`x: str = cast(str, 5)` looks consistent), but the value's real
    // inferred type is still `int`.
    let err =
        check_source("from typing import cast\nx: str = cast(str, 5)\nprint(x)\n").unwrap_err();
    assert_eq!(err.code, "C0001");
}

#[test]
fn cast_widening_bool_to_int_is_c0001() {
    // #767 review fix (D-198): `bool` is a *static* subtype of `int`, but
    // `TYPE_SYSTEM.md`'s representation table gives it a standalone `i8`
    // against `int`'s `i64`-compatible word. Because pycc emits no
    // conversion for an erased `cast`, this widening is a representation
    // change like any other and is rejected -- the one case where the
    // representation rule is deliberately stricter than assignability.
    let err = check_source(
        "from typing import cast\ndef f(b: bool) -> int:\n    return cast(int, b)\nprint(f(True))\n",
    )
    .unwrap_err();
    assert_eq!(err.code, "C0001");
}

#[test]
fn cast_narrowing_int_to_bool_is_c0001() {
    // #767 review fix: the reverse direction of the case above, rejected
    // for the same reason.
    let err = check_source(
        "from typing import cast\ndef f(x: int) -> bool:\n    return cast(bool, x)\nprint(f(1))\n",
    )
    .unwrap_err();
    assert_eq!(err.code, "C0001");
}

#[test]
fn cast_with_the_wrong_argument_count_is_t0021() {
    // #767: `cast` takes exactly two arguments; every other arity is an
    // ordinary call-shape error, the same `T0021` `isinstance` uses.
    let err = check_source("from typing import cast\ndef f(x: int) -> int:\n    return cast(x)\n")
        .unwrap_err();
    assert_eq!(err.code, "T0021");
    assert!(
        err.message
            .contains("`cast` expects exactly 2 arguments, got 1"),
        "expected an arity message, got: {}",
        err.message
    );
}

#[test]
fn cast_with_a_subscripted_first_argument_is_c0001() {
    // #767: the target type must be a bare name in this subset.
    // `cast(list[int], x)` is valid Python that pycc does not implement
    // yet, so it is a versioned capability gap (`C0001`), not a
    // by-design rejection -- the same classification `check_isinstance`
    // uses for its own in-scope-but-unimplemented call shapes.
    let err = check_source(
        "from typing import cast\ndef f(x: int) -> int:\n    return cast(list[int], x)\n",
    )
    .unwrap_err();
    assert_eq!(err.code, "C0001");
    assert!(
        err.message.contains("must be a bare type name"),
        "expected a bare-type-name message, got: {}",
        err.message
    );
}

#[test]
fn cast_to_an_unknown_class_name_is_t0001() {
    // #767: an unknown target name is rejected by `validate_class_name`,
    // shared with `isinstance`/`issubclass`. This also pins the solver
    // mirror's deliberate `Ok(None)` for non-builtin targets: producing an
    // unverified `Ty::Instance("Nope")` there instead made the solver
    // report `T0022` ("conflicting inferred types `int` and `Nope`")
    // before this accurate diagnostic could run.
    let err =
        check_source("from typing import cast\ndef f(x: int) -> int:\n    return cast(Nope, x)\n")
            .unwrap_err();
    assert_eq!(err.code, "T0001");
    assert!(
        err.message
            .contains("`Nope` is not a known class or builtin type"),
        "expected an unknown-class message, got: {}",
        err.message
    );
}

#[test]
fn cast_reports_errors_in_its_value_argument() {
    // #767: `check_cast` infers the value operand so its own errors are
    // still reported, even though the inferred type is discarded.
    let err = check_source(
        "from typing import cast\ndef f() -> int:\n    return cast(int, undefined_name)\n",
    )
    .unwrap_err();
    assert_eq!(err.code, "T0021");
}

#[test]
fn a_user_defined_cast_function_takes_priority_over_the_builtin() {
    // #767: a program defining its own `def cast(...)` calls that function
    // -- the same user-definition-takes-priority rule `float`,
    // `isinstance`, and `issubclass` follow, in both the validation pass
    // (`env.lookup_function`) and the solver (`signatures.contains_key`).
    // A two-`int` signature would be a type error under the builtin's own
    // rules (`1` is not a bare type name), so this only checks if the
    // user's function really won.
    check_source("def cast(a: int, b: int) -> int:\n    return a + b\nprint(cast(1, 2))\n")
        .unwrap();
}

#[test]
fn a_user_defined_cast_function_is_resolved_by_the_solver_in_a_private_helper() {
    // #767: the solver-path half of the priority rule -- a private helper
    // with an inferred return type calls the user's own `cast`, so
    // `collect_expr_constraints` must resolve it through `signatures`
    // rather than short-circuiting to the builtin's target-type mapping.
    check_source(
        "def cast(a: int, b: int) -> int:\n    return a + b\ndef _helper():\n    return cast(1, 2)\nprint(_helper())\n",
    )
    .unwrap();
}

#[test]
fn cast_in_a_private_helper_resolves_through_the_solver() {
    // #767: the solver mirror's builtin-scalar arm -- a private helper
    // with an inferred return type returning `cast(str, ...)` must resolve
    // to `Ty::Str` in `collect_expr_constraints`. Without the mirror the
    // callee misses `signatures`, the call stays unresolved, and the
    // solver dead-ends in "cannot infer return type".
    check_source(
        "from typing import cast\ndef _helper():\n    return cast(str, \"v\")\nprint(_helper())\n",
    )
    .unwrap();
}

#[test]
fn cast_without_its_import_is_currently_accepted() {
    // #768 pins present behavior, deliberately: `pycc_types` never reads
    // `HirModule::imports`, so the bare-`cast` interception cannot tell an
    // imported `cast` from an unimported one and accepts a program CPython
    // rejects with `NameError`. Every other bare-name stdlib symbol this
    // compiler special-cases (`Final`, `Annotated`, the `Enum`/`Protocol`/
    // `ABC` markers) has the same gap, so gating `cast` alone would make the
    // compiler inconsistent rather than correct. Closing it uniformly needs
    // import visibility threaded through both `Environment` and the solver's
    // `ConstraintEnvironment`; #768 owns that work and must invert this test.
    check_source("def f(x: int) -> int:\n    return cast(int, x)\nprint(f(1))\n").unwrap();
}

#[test]
fn cast_without_its_import_is_currently_accepted_through_the_solver_mirror() {
    // #768, solver-mirror half of the pin above: `cast_without_its_import_is_currently_accepted`
    // only exercises `expr.rs`'s validation-pass interception (an annotated
    // function's body). `constraints.rs`'s solver mirror is a separate,
    // hand-written arm with its own `signatures.contains_key(callee)` guard
    // and no import check of its own, reached only for a return-type-inferred
    // private helper -- so a #768 fix that inverts only the validation-pass
    // gate would leave this route accepting unimported `cast` unnoticed.
    // Pinning both routes now means #768 must invert both.
    check_source("def _helper():\n    return cast(int, 1)\nprint(_helper())\n").unwrap();
}

#[test]
fn a_malformed_cast_in_a_private_helper_still_reports_the_cast_diagnostic() {
    // #767: the solver mirror reports a malformed call shape itself,
    // through the same `cast_target_name` helper `check_cast` uses. Left
    // to produce "no type term" instead, the solver would surface its
    // generic "cannot infer return type of private helper `_helper`" here
    // and hide the accurate message.
    let err = check_source(
        "from typing import cast\ndef _helper():\n    return cast(1, 2)\nprint(_helper())\n",
    )
    .unwrap_err();
    assert_eq!(err.code, "C0001");
    assert!(
        err.message.contains("must be a bare type name"),
        "expected a bare-type-name message, got: {}",
        err.message
    );
}

#[test]
fn qualified_cast_marker_used_as_value_is_t0021() {
    // #767: exercises the dedicated `CastMarker` arm folded into the
    // marker arm of infer_expr_in's Name handler (expr.rs). `cast` is
    // recognized by bare callee name, so the qualified `typing.cast` form
    // never reaches the special case.
    let err = check_source("import typing\nx = typing.cast\n").unwrap_err();
    assert_eq!(err.code, "T0021");
    assert!(
        err.message.contains("compile-time cast marker")
            && err.message.contains("from typing import cast"),
        "expected a cast-specific message, got: {}",
        err.message
    );
}

#[test]
fn qualified_cast_marker_as_value_in_private_helper_is_t0021() {
    // #767: the same `CastMarker` arm in collect_expr_constraints' Name
    // handler (constraints.rs, the solver path).
    let err = check_source(
        "import typing\ndef _helper() -> int:\n    x = typing.cast\n    return 1\n_helper()\n",
    )
    .unwrap_err();
    assert_eq!(err.code, "T0021");
    assert!(
        err.message.contains("compile-time cast marker"),
        "expected a cast-specific message, got: {}",
        err.message
    );
}

#[test]
fn qualified_cast_marker_called_is_t0021() {
    // #767: exercises the `CastMarker` branch of the call-site marker
    // guard in expr.rs's infer_expr_in (the `Function` let-else
    // fallthrough) -- `typing.cast(int, 1)` is rejected with guidance to
    // import and call the bare name instead.
    let err = check_source("import typing\nx = typing.cast(int, 1)\n").unwrap_err();
    assert_eq!(err.code, "T0021");
    assert!(
        err.message.contains("compile-time cast marker"),
        "expected a cast-specific message, got: {}",
        err.message
    );
}

#[test]
fn qualified_cast_marker_called_inside_an_annotated_function_is_t0021() {
    // #767: the module-level and private-helper forms above both reach
    // `check` through the solver, so they exercise `constraints.rs`'s own
    // `CastMarker` arm rather than `expr.rs`'s. A call inside a fully
    // annotated public function takes the validation pass instead, which is
    // the only route to `infer_expr_in`'s own `CastMarker` branch. The
    // arguments must both be ordinary values: `infer_expr_in` infers every
    // argument *before* it inspects the callee, so a type name in argument
    // position (`typing.cast(int, 1)`) fails as an undefined value first and
    // never reaches the marker guard.
    let err = check_source(
        "import typing\ndef f() -> int:\n    x = typing.cast(1, 2)\n    return x\nprint(f())\n",
    )
    .unwrap_err();
    assert_eq!(err.code, "T0021");
    assert!(
        err.message.contains("compile-time cast marker"),
        "expected a cast-specific message, got: {}",
        err.message
    );
}

#[test]
fn qualified_cast_marker_called_in_private_helper_is_t0021() {
    // #767: the same call-site `CastMarker` branch in constraints.rs's
    // collect_expr_constraints (the solver path).
    let err = check_source(
        "import typing\ndef _helper() -> int:\n    x = typing.cast(int, 1)\n    return 1\n_helper()\n",
    )
    .unwrap_err();
    assert_eq!(err.code, "T0021");
    assert!(
        err.message.contains("compile-time cast marker"),
        "expected a cast-specific message, got: {}",
        err.message
    );
}
