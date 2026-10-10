//! Function redefinition compatibility and cross-MRO attribute conflicts.

use super::*;

// ---- Issue #22 review fixes: incompatible redefinition rejection ----

#[test]
fn incompatible_redefinition_with_different_param_count_is_rejected() {
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            HirItem::Function {
                name: "foo".to_string(),
                params: vec![("x".to_string(), Ty::Int)],
                return_ty: Ty::Int,
                body: vec![HirStmt::Return(Some(HirExpr::Name("x".to_string())))],
            },
            HirItem::Function {
                name: "foo".to_string(),
                params: vec![("x".to_string(), Ty::Int), ("y".to_string(), Ty::Int)],
                return_ty: Ty::Int,
                body: vec![HirStmt::Return(Some(HirExpr::Name("x".to_string())))],
            },
        ],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    let err = check(&hir).unwrap_err();
    assert_eq!(err.code, "T0021");
    assert!(
        err.message
            .contains("cannot redefine function `foo` with a different signature"),
        "got: {}",
        err.message
    );
}

#[test]
fn checked_function_signatures_rejects_incompatible_redefinition() {
    // Exercises the fast path in checked_function_signatures_all that calls
    // check_incompatible_redefinitions before trying concrete or solver.
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            HirItem::Function {
                name: "foo".to_string(),
                params: vec![("x".to_string(), Ty::Int)],
                return_ty: Ty::Int,
                body: vec![HirStmt::Return(Some(HirExpr::Name("x".to_string())))],
            },
            HirItem::Function {
                name: "foo".to_string(),
                params: vec![("x".to_string(), Ty::Int), ("y".to_string(), Ty::Int)],
                return_ty: Ty::Int,
                body: vec![HirStmt::Return(Some(HirExpr::Name("x".to_string())))],
            },
        ],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    let local_names = module_function_local_names(&hir);
    let err = checked_function_signatures(&hir, &local_names).unwrap_err();
    assert_eq!(err.code, "T0021");
    assert!(
        err.message
            .contains("cannot redefine function `foo` with a different signature"),
        "got: {}",
        err.message
    );
}

#[test]
fn incompatible_redefinition_with_different_return_type_is_rejected() {
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            HirItem::Function {
                name: "foo".to_string(),
                params: vec![("x".to_string(), Ty::Int)],
                return_ty: Ty::Int,
                body: vec![HirStmt::Return(Some(HirExpr::Name("x".to_string())))],
            },
            HirItem::Function {
                name: "foo".to_string(),
                params: vec![("x".to_string(), Ty::Int)],
                return_ty: Ty::None,
                body: vec![],
            },
        ],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    let err = check(&hir).unwrap_err();
    assert_eq!(err.code, "T0021");
    assert!(
        err.message
            .contains("cannot redefine function `foo` with a different signature"),
        "got: {}",
        err.message
    );
}

#[test]
fn incompatible_redefinition_with_different_param_type_is_rejected() {
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            HirItem::Function {
                name: "foo".to_string(),
                params: vec![("x".to_string(), Ty::Int)],
                return_ty: Ty::None,
                body: vec![],
            },
            HirItem::Function {
                name: "foo".to_string(),
                params: vec![("x".to_string(), Ty::Str)],
                return_ty: Ty::None,
                body: vec![],
            },
        ],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    let err = check(&hir).unwrap_err();
    assert_eq!(err.code, "T0021");
    assert!(
        err.message
            .contains("cannot redefine function `foo` with a different signature"),
        "got: {}",
        err.message
    );
}

#[test]
fn compatible_redefinition_with_same_signature_is_accepted() {
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            HirItem::Function {
                name: "foo".to_string(),
                params: vec![],
                return_ty: Ty::None,
                body: vec![HirStmt::ExprStmt(HirExpr::Call {
                    callee: "print".to_string(),
                    args: vec![HirExpr::IntLiteral(1)],
                })],
            },
            HirItem::Function {
                name: "foo".to_string(),
                params: vec![],
                return_ty: Ty::None,
                body: vec![HirStmt::ExprStmt(HirExpr::Call {
                    callee: "print".to_string(),
                    args: vec![HirExpr::IntLiteral(2)],
                })],
            },
        ],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    assert!(check(&hir).is_ok());
}

#[test]
fn check_incompatible_redefinitions_rejects_infer_signature_mismatch() {
    // Issue #402: a same-arity redefinition where one signature still
    // carries Ty::Infer (unresolved by the solver) and the other is
    // concrete must be rejected, not silently accepted. Ty::Infer is an
    // ordinary unit variant under Ty's derived PartialEq, so it only
    // ever equals another Ty::Infer at the same position -- comparing
    // it unconditionally against check_incompatible_redefinitions's
    // `seen` map correctly flags this as a mismatch.
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            HirItem::Function {
                name: "foo".to_string(),
                params: vec![("x".to_string(), Ty::Infer)],
                return_ty: Ty::Infer,
                body: vec![],
            },
            HirItem::Function {
                name: "foo".to_string(),
                params: vec![("x".to_string(), Ty::Int)],
                return_ty: Ty::None,
                body: vec![],
            },
        ],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    let err = check(&hir).unwrap_err();
    assert_eq!(err.code, "T0021");
    assert!(
        err.message
            .contains("cannot redefine function `foo` with a different signature"),
        "got: {}",
        err.message
    );
}

#[test]
fn check_and_resolve_rejects_the_issue_402_reproduction_fixture() {
    // Issue #402's own reproduction fixture, exercised through
    // check_and_resolve (the `pycc build` path): first `foo` is fully
    // unannotated (Ty::Infer for both the parameter and the return
    // type), second `foo` is fully concrete and a different shape.
    // This is now rejected by the pre-resolution
    // check_incompatible_redefinitions call inside
    // checked_function_signatures_all, before check_and_resolve ever
    // builds a resolved HIR.
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            HirItem::Function {
                name: "foo".to_string(),
                params: vec![("x".to_string(), Ty::Infer)],
                return_ty: Ty::Infer,
                body: vec![],
            },
            HirItem::Function {
                name: "foo".to_string(),
                params: vec![("x".to_string(), Ty::Int)],
                return_ty: Ty::None,
                body: vec![],
            },
        ],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    let err = check_and_resolve(&hir).unwrap_err();
    assert_eq!(err.code, "T0021");
    assert!(
        err.message.contains("cannot redefine function `foo`"),
        "got: {}",
        err.message
    );
    assert!(
        err.message.contains("<inferred>"),
        "message should render the unresolved first signature's Ty::Infer \
             positions as `<inferred>`, got: {}",
        err.message
    );
}

#[test]
fn checked_function_signatures_rejects_the_issue_402_reproduction_fixture() {
    // Same fixture as check_and_resolve_rejects_the_issue_402_reproduction_fixture
    // and check_incompatible_redefinitions_rejects_infer_signature_mismatch,
    // but through `checked_function_signatures`'s own separate call site
    // (mirrors checked_function_signatures_rejects_incompatible_redefinition
    // above, which exercises that same call site with a fully-concrete,
    // different-arity fixture instead of an Infer-involving one).
    // `check_incompatible_redefinitions` is called directly from both
    // `check_all` and `checked_function_signatures_all`; each entry point is
    // exercised independently rather than relying on one to cover the
    // other.
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            HirItem::Function {
                name: "foo".to_string(),
                params: vec![("x".to_string(), Ty::Infer)],
                return_ty: Ty::Infer,
                body: vec![],
            },
            HirItem::Function {
                name: "foo".to_string(),
                params: vec![("x".to_string(), Ty::Int)],
                return_ty: Ty::None,
                body: vec![],
            },
        ],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    let local_names = module_function_local_names(&hir);
    let err = checked_function_signatures(&hir, &local_names).unwrap_err();
    assert_eq!(err.code, "T0021");
    assert!(
        err.message.contains("cannot redefine function `foo`"),
        "got: {}",
        err.message
    );
}

// -- #676 (D-210): cross-MRO attribute redeclaration with a differing
// declared type ------------------------------------------------------

/// Helper for the `check_incompatible_attribute_redeclarations` tests
/// below: builds a minimal `HirClassDef` with the given name, bases, MRO,
/// and own declared attributes, and every other field at its "no extra
/// feature" default -- mirroring the boilerplate already established by
/// `generic_class_module_with_call`'s own `HirClassDef` literal above.
fn minimal_class_def(
    name: &str,
    bases: &[&str],
    mro: &[&str],
    attrs: &[(&str, Ty)],
) -> HirClassDef {
    HirClassDef {
        class_attrs: Vec::new(),
        exception_type_tag: None,
        name: name.to_string(),
        bases: bases.iter().map(|s| s.to_string()).collect(),
        mro: mro.iter().map(|s| s.to_string()).collect(),
        attrs: attrs
            .iter()
            .map(|(attr_name, ty)| (attr_name.to_string(), ty.clone()))
            .collect(),
        methods: Vec::new(),
        properties: Vec::new(),
        static_methods: Vec::new(),
        class_methods: Vec::new(),
        type_param: None,
        is_enum: false,
        implicit_object_init: false,
        method_defaults: Vec::new(),
        enum_members: Vec::new(),
        is_dataclass: false,
        dataclass_fields: Vec::new(),
        is_protocol: false,
        runtime_checkable: false,
        protocol_members: Vec::new(),
        abstract_methods: Vec::new(),
        is_abstract: false,
    }
}

/// Wraps a set of `HirClassDef`s (already carrying their own name, in
/// `(name, def)` pairs) into a minimal `HirModule` with no functions or
/// top-level statements -- `check_incompatible_attribute_redeclarations`
/// only ever reads `hir.class_defs`.
fn module_with_class_defs(class_defs: Vec<(String, HirClassDef)>) -> HirModule {
    HirModule {
        seeded_builtin_exception_classes: false,
        items: Vec::new(),
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs,
    }
}

#[test]
fn cross_mro_attribute_redeclaration_of_int_as_bool_is_rejected() {
    // The issue's own primary repro direction: Base declares `int`,
    // Derived redeclares `bool` for the same attribute name.
    let base = minimal_class_def("Base", &[], &["Base"], &[("v", Ty::Int)]);
    let derived = minimal_class_def(
        "Derived",
        &["Base"],
        &["Derived", "Base"],
        &[("v", Ty::Bool)],
    );
    let hir = module_with_class_defs(vec![
        ("Base".to_string(), base),
        ("Derived".to_string(), derived),
    ]);
    let err = check(&hir).unwrap_err();
    assert_eq!(err.code, "T0052");
    assert!(
        err.message.contains('v')
            && err.message.contains("Base")
            && err.message.contains("Derived"),
        "got: {}",
        err.message
    );
}

#[test]
fn cross_mro_attribute_redeclaration_of_bool_as_int_is_rejected_symmetrically() {
    // The mirror direction (criterion 3): Base declares `bool`, Derived
    // redeclares `int`. Confirms the rule is symmetric/direction-
    // independent, not special-cased to "narrower into wider".
    let base = minimal_class_def("Base", &[], &["Base"], &[("v", Ty::Bool)]);
    let derived = minimal_class_def(
        "Derived",
        &["Base"],
        &["Derived", "Base"],
        &[("v", Ty::Int)],
    );
    let hir = module_with_class_defs(vec![
        ("Base".to_string(), base),
        ("Derived".to_string(), derived),
    ]);
    let err = check(&hir).unwrap_err();
    assert_eq!(err.code, "T0052");
}

#[test]
fn cross_mro_attribute_redeclaration_of_float_as_int_is_rejected() {
    // The independently-discovered generalization: the defect (and its
    // fix) is not bool/int-specific. A float/int redeclaration must be
    // rejected the same way.
    let base = minimal_class_def("Base", &[], &["Base"], &[("v", Ty::Float)]);
    let derived = minimal_class_def(
        "Derived",
        &["Base"],
        &["Derived", "Base"],
        &[("v", Ty::Int)],
    );
    let hir = module_with_class_defs(vec![
        ("Base".to_string(), base),
        ("Derived".to_string(), derived),
    ]);
    let err = check(&hir).unwrap_err();
    assert_eq!(err.code, "T0052");
}

#[test]
fn same_class_bool_value_into_an_int_declared_attribute_is_not_rejected() {
    // D-187 regression: this check only ever compares two distinct
    // classes' own *declared* attribute types -- never a single class's
    // declared type against an assigned *value*. A same-class `int`-
    // declared attribute assigned a `bool` value elsewhere in the same
    // class's own methods involves no cross-class redeclaration at all,
    // and `attrs` records only the one declared type for `n` on `C`, so
    // this must not fire T0052.
    let c = minimal_class_def("C", &[], &["C"], &[("n", Ty::Int)]);
    let hir = module_with_class_defs(vec![("C".to_string(), c)]);
    assert!(check(&hir).is_ok());
}

#[test]
fn identical_type_redeclaration_across_the_mro_is_not_rejected() {
    // The existing `mro_attrs` dedup case: both Base and Derived declare
    // `v` as `int` -- an identical redeclaration, not a differing one.
    let base = minimal_class_def("Base", &[], &["Base"], &[("v", Ty::Int)]);
    let derived = minimal_class_def(
        "Derived",
        &["Base"],
        &["Derived", "Base"],
        &[("v", Ty::Int)],
    );
    let hir = module_with_class_defs(vec![
        ("Base".to_string(), base),
        ("Derived".to_string(), derived),
    ]);
    assert!(check(&hir).is_ok());
}

#[test]
fn a_base_class_only_attribute_with_no_redeclaration_is_not_rejected() {
    // Baseline "no MRO conflict at all" case: Derived inherits `v` from
    // Base without redeclaring it anywhere.
    let base = minimal_class_def("Base", &[], &["Base"], &[("v", Ty::Int)]);
    let derived = minimal_class_def("Derived", &["Base"], &["Derived", "Base"], &[]);
    let hir = module_with_class_defs(vec![
        ("Base".to_string(), base),
        ("Derived".to_string(), derived),
    ]);
    assert!(check(&hir).is_ok());
}

#[test]
fn diamond_sibling_base_classes_with_differing_types_are_rejected() {
    // Section 3/7's corrected comparison rule: two sibling base classes
    // `B1`/`B2` (neither an ancestor of the other) each declare the same
    // attribute name with different types, and `D(B1, B2)` inherits both
    // without redeclaring it itself. A rule limited to "class vs. its own
    // ancestor" would miss this entirely -- the check must walk `D`'s own
    // linearized MRO and compare `B1` against `B2` directly as a same-MRO
    // sibling pair.
    let b1 = minimal_class_def("B1", &[], &["B1"], &[("v", Ty::Int)]);
    let b2 = minimal_class_def("B2", &[], &["B2"], &[("v", Ty::Float)]);
    let d = minimal_class_def("D", &["B1", "B2"], &["D", "B1", "B2"], &[]);
    let hir = module_with_class_defs(vec![
        ("B1".to_string(), b1),
        ("B2".to_string(), b2),
        ("D".to_string(), d),
    ]);
    let err = check(&hir).unwrap_err();
    assert_eq!(err.code, "T0052");
}

#[test]
fn checked_function_signatures_also_rejects_a_cross_mro_attribute_redeclaration() {
    // `check_incompatible_attribute_redeclarations` is called from both
    // `check_all` and `checked_function_signatures_all` (the latter reachable
    // from `pycc build`'s `check_and_resolve` without an earlier `pycc
    // check`/`check` call) -- each entry point is exercised
    // independently, mirroring how
    // `checked_function_signatures_rejects_the_issue_402_reproduction_fixture`
    // exercises `check_incompatible_redefinitions`'s own second call site.
    let base = minimal_class_def("Base", &[], &["Base"], &[("v", Ty::Int)]);
    let derived = minimal_class_def(
        "Derived",
        &["Base"],
        &["Derived", "Base"],
        &[("v", Ty::Bool)],
    );
    let hir = module_with_class_defs(vec![
        ("Base".to_string(), base),
        ("Derived".to_string(), derived),
    ]);
    let local_names = module_function_local_names(&hir);
    let err = checked_function_signatures(&hir, &local_names).unwrap_err();
    assert_eq!(err.code, "T0052");
}

#[test]
fn check_and_resolve_rejects_the_issue_676_bool_reproduction_fixture() {
    // The issue's own exact end-to-end repro, through the real
    // parser/lowerer/`check_and_resolve` pipeline (the `pycc build`
    // path): Base's own `__init__` declares `v` as `bool`, a base-class
    // method assigns `True` into it (matching Base's own declared type,
    // so this alone is fine), and Derived's own `__init__` redeclares
    // the same attribute name as `int`. This must now be rejected with
    // `T0052` at compile time, not silently mis-decoded or aborted at
    // run time.
    let err = check_source(
        "class Base:\n    def __init__(self) -> None:\n        self.v = False\n    def set_true(self) -> None:\n        self.v = True\nclass Derived(Base):\n    def __init__(self) -> None:\n        self.v = 0\nd = Derived()\nd.set_true()\nprint(d.v)\n",
    )
    .unwrap_err();
    assert_eq!(err.code, "T0052");
}

#[test]
fn check_and_resolve_rejects_the_issue_676_float_int_generalization_fixture() {
    // The independently-discovered generalization, through the real
    // pipeline: Base declares a `float`-typed attribute and assigns into
    // it from another method; Derived's own `__init__` redeclares the
    // same attribute name as `int`. Before this change this compiled
    // with no diagnostic and segfaulted at run time (exit 139); it must
    // now be rejected with `T0052` at compile time instead.
    let err = check_source(
        "class Base:\n    def __init__(self) -> None:\n        self.v = 1.0\n    def set_it(self) -> None:\n        self.v = 2.5\nclass Derived(Base):\n    def __init__(self) -> None:\n        self.v = 1\nd = Derived()\nd.set_it()\nprint(d.v)\n",
    )
    .unwrap_err();
    assert_eq!(err.code, "T0052");
}

#[test]
fn check_rejects_the_issue_676_bool_reproduction_fixture_via_the_pycc_check_path() {
    // The two `check_and_resolve_rejects_the_issue_676_*` tests above
    // exercise `pycc build`'s own entry point end-to-end; this mirrors
    // the issue's own bool reproduction through `check` directly (the
    // `pycc check` CLI path, which validates without resolving/
    // monomorphizing), confirming both public entry points reject the
    // same real-source module rather than only one.
    let module = pycc_parser::parse(
        "class Base:\n    def __init__(self) -> None:\n        self.v = False\n    def set_true(self) -> None:\n        self.v = True\nclass Derived(Base):\n    def __init__(self) -> None:\n        self.v = 0\nd = Derived()\nd.set_true()\nprint(d.v)\n",
    )
    .expect("test fixture must parse");
    let hir = pycc_hir::lower_checked(&module).expect("test fixture must lower");
    let err = check(&hir).unwrap_err();
    assert_eq!(err.code, "T0052");
}

#[test]
fn a_bare_base_instance_with_no_derived_redeclaration_still_widens_bool_into_int() {
    // The decisive counter-example against "coerce" (D-210's own
    // rationale), pinned as a regression: with no conflicting
    // redeclaration anywhere in the module, a bare `Base()` instance
    // calling a method that assigns a `bool` into its own `int`-declared
    // attribute must still type-check and keep D-187's widening
    // behavior -- this module must NOT be rejected by T0052 merely
    // because a `Base` class exists.
    let hir = check_source(
        "class Base:\n    def __init__(self) -> None:\n        self.v = False\n    def set_true(self) -> None:\n        self.v = True\nb = Base()\nb.set_true()\nprint(b.v)\n",
    )
    .expect("no cross-MRO conflict exists here -- must type-check");
    assert!(
        hir.class_defs.iter().any(|(name, _)| name == "Base"),
        "Base should still be a registered class"
    );
}

#[test]
fn subclassing_a_monomorphized_generic_specialization_is_already_rejected_pre_existing() {
    // D-210's own false-positive sweep (section 3) flagged this as an
    // unverified residual: whether a non-generic class can subclass an
    // already-monomorphized generic specialization (`class
    // Derived(C[int]):`). Verified here: it is already rejected with
    // `C0001` ("a base class must be a bare name") before the new
    // `check_incompatible_attribute_redeclarations` check would ever run
    // -- a subscript expression is not a bare base-class name, so this
    // path can never reach `check_incompatible_attribute_redeclarations`'s
    // MRO walk at all. Pinned here so a future change to base-class
    // parsing can't silently open this path without this test noticing.
    let module = pycc_parser::parse(
        "class C[T]:\n    def __init__(self, v: T) -> None:\n        self.v = v\nclass Derived(C[int]):\n    pass\n",
    )
    .expect("test fixture must parse");
    let err = pycc_hir::lower_checked(&module).unwrap_err();
    assert_eq!(err.code, "C0001");
}

#[test]
fn dataclass_field_type_conflict_across_mro_is_rejected_during_hir_lowering() {
    // Round-2 deep review pinned the pre-fix behavior (a dataclass
    // field-name conflict was silently discarded by `pycc_hir::class`'s
    // merge/dedup before `check_incompatible_attribute_redeclarations`
    // ever ran) as "expected". A later external bot review (round 4,
    // chatgpt-codex-connector) correctly flagged that as the same class of
    // silent cross-type bug issue #676 itself was filed about, just
    // reached through dataclass field merging instead of a base-class
    // method's attribute assignment: Base and Derived each declare a
    // field `v` with a *different* type, and merging must not silently
    // keep the least-derived type -- it must reject the conflict, exactly
    // as a non-dataclass cross-MRO redeclaration is rejected.
    // The conflict is now caught during HIR lowering itself (not by
    // `check_and_resolve`), since dataclass field merging happens in
    // `pycc_hir::class::lower_class` -- so this goes through
    // `pycc_hir::lower_checked` directly rather than `check_source`,
    // which `.expect()`s lowering to succeed.
    let module = pycc_parser::parse(
        "from dataclasses import dataclass\n@dataclass\nclass Base:\n    v: int\n@dataclass\nclass Derived(Base):\n    v: str\nd = Derived(1)\nprint(d.v)\n",
    )
    .expect("test fixture must parse");
    let err = pycc_hir::lower_checked(&module).expect_err(
        "a dataclass field redeclared with a differing type across the MRO must be rejected",
    );
    assert_eq!(err.code, "T0052");
    assert!(
        err.message.contains('v') && err.message.contains("int") && err.message.contains("str"),
        "got: {}",
        err.message
    );
}

#[test]
fn dataclass_field_type_conflict_between_two_bases_is_rejected_during_hir_lowering() {
    // The diamond case: two independent dataclass bases each declare the
    // same field name with a different type, and a derived dataclass
    // inherits from both. The merge must reject this exactly as the
    // base-vs-own-declaration case above, not only when the conflict
    // involves the derived class's own redeclaration.
    let module = pycc_parser::parse(
        "from dataclasses import dataclass\n@dataclass\nclass A:\n    v: int\n@dataclass\nclass B:\n    v: str\n@dataclass\nclass Derived(A, B):\n    pass\n",
    )
    .expect("test fixture must parse");
    let err = pycc_hir::lower_checked(&module).expect_err(
        "two dataclass bases redeclaring the same field with different types must be rejected",
    );
    assert_eq!(err.code, "T0052");
    assert!(
        err.message.contains('v') && err.message.contains("int") && err.message.contains("str"),
        "got: {}",
        err.message
    );
}

#[test]
fn infer_signature_redefinition_is_rejected_regardless_of_body_evidence() {
    // Issue #402: the rejection is structural, based on each
    // definition's raw pre-resolution shape -- not on what the solver
    // would eventually infer from the body. The first `foo`'s body
    // would resolve `x`/the return type to `int` if it ran, but the
    // second `foo` is concretely `(str) -> str`; the raw shapes
    // (Infer, Infer) vs (Str, Str) already disagree, so this is
    // rejected before the solver ever sees the first body.
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            HirItem::Function {
                name: "foo".to_string(),
                params: vec![("x".to_string(), Ty::Infer)],
                return_ty: Ty::Infer,
                body: vec![HirStmt::Return(Some(HirExpr::BinOp {
                    op: BinOpKind::Add,
                    left: Box::new(HirExpr::Name("x".to_string())),
                    right: Box::new(HirExpr::IntLiteral(1)),
                }))],
            },
            HirItem::Function {
                name: "foo".to_string(),
                params: vec![("x".to_string(), Ty::Str)],
                return_ty: Ty::Str,
                body: vec![],
            },
        ],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    let err = check_and_resolve(&hir).unwrap_err();
    assert_eq!(err.code, "T0021");
    assert!(
        err.message.contains("cannot redefine function `foo`"),
        "got: {}",
        err.message
    );
}

#[test]
fn compatible_infer_signature_redefinition_with_call_site_evidence_is_accepted() {
    // Issue #402 must-not-regress: two structurally identical
    // Ty::Infer signatures (an unannotated helper redefined verbatim)
    // must still be accepted -- the fix only rejects a *mismatch*
    // between raw shapes, not an unresolved shape on its own. A call
    // site is required so the solver has evidence to resolve `x`/the
    // return type; without one, this fixture would fail for an
    // unrelated reason ("cannot infer type of parameter").
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            HirItem::Function {
                name: "foo".to_string(),
                params: vec![("x".to_string(), Ty::Infer)],
                return_ty: Ty::Infer,
                body: vec![HirStmt::Return(Some(HirExpr::Name("x".to_string())))],
            },
            HirItem::Function {
                name: "foo".to_string(),
                params: vec![("x".to_string(), Ty::Infer)],
                return_ty: Ty::Infer,
                body: vec![HirStmt::Return(Some(HirExpr::Name("x".to_string())))],
            },
            HirItem::TopLevelStmt(HirStmt::ExprStmt(HirExpr::Call {
                callee: "foo".to_string(),
                args: vec![HirExpr::IntLiteral(1)],
            })),
        ],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    assert!(check_and_resolve(&hir).is_ok());
}

#[test]
fn check_and_resolve_rejects_incompatible_redefinition_with_infer_signature() {
    // Issue #402: two functions share the name `foo`; the first has
    // Ty::Infer, the second has a concrete but incompatible (different
    // arity) signature. Before this fix, only a resolved-arity mismatch
    // like this one was caught, and only by a post-resolution recheck
    // inside `check_and_resolve`. That recheck is gone now: this raw
    // shape mismatch (arity, independent of Infer) is caught by the
    // pre-resolution `check_incompatible_redefinitions` call inside
    // `checked_function_signatures_all`, reached before `check_and_resolve`
    // ever builds a resolved HIR.
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            HirItem::Function {
                name: "foo".to_string(),
                params: vec![("x".to_string(), Ty::Infer)],
                return_ty: Ty::Infer,
                body: vec![HirStmt::Return(Some(HirExpr::Name("x".to_string())))],
            },
            HirItem::Function {
                name: "foo".to_string(),
                params: vec![("x".to_string(), Ty::Int), ("y".to_string(), Ty::Int)],
                return_ty: Ty::Int,
                body: vec![HirStmt::Return(Some(HirExpr::Name("x".to_string())))],
            },
        ],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    let err = check_and_resolve(&hir).unwrap_err();
    assert_eq!(err.code, "T0021");
    assert!(
        err.message
            .contains("cannot redefine function `foo` with a different signature"),
        "got: {}",
        err.message
    );
}

#[test]
fn incompatible_redefinition_with_bare_return_type_is_rejected() {
    // Issue #402's most plausible real-world trip (see the plan's own
    // §3.5): a first definition that annotates its parameter but
    // leaves the return type bare (Ty::Infer), redefined with a fully
    // concrete signature that adds a return annotation. Params agree;
    // only the return position differs by Infer-vs-concrete -- this
    // isolates that the fix triggers on the return position alone.
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            HirItem::Function {
                name: "foo".to_string(),
                params: vec![("x".to_string(), Ty::Int)],
                return_ty: Ty::Infer,
                body: vec![],
            },
            HirItem::Function {
                name: "foo".to_string(),
                params: vec![("x".to_string(), Ty::Int)],
                return_ty: Ty::None,
                body: vec![],
            },
        ],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    let err = check_and_resolve(&hir).unwrap_err();
    assert_eq!(err.code, "T0021");
    assert!(
        err.message
            .contains("cannot redefine function `foo` with a different signature"),
        "got: {}",
        err.message
    );
}

#[test]
fn mangled_method_name_does_not_collide_with_same_named_top_level_function() {
    // Issue #402 §3.6 regression: `check_incompatible_redefinitions`
    // iterates the same flat `hir.items` list a class's own methods are
    // merged into, name-keyed with no class metadata. A method always
    // lowers into a mangled `<ClassName>.<method_name>` name (never
    // containing a bare `.`-free `NAME` token), so it can never
    // conflate with a bare top-level function of the unmangled name --
    // confirmed here with deliberately *different* signatures: if the
    // name-keyed map ever conflated `"Foo.bar"` and `"bar"`, this
    // fixture would trip T0021 for a differing param type "under the
    // same name". Accepting it is a stronger check than accepting two
    // identical signatures would be.
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            HirItem::Function {
                name: "Foo.bar".to_string(),
                params: vec![("x".to_string(), Ty::Int)],
                return_ty: Ty::None,
                body: vec![],
            },
            HirItem::Function {
                name: "bar".to_string(),
                params: vec![("x".to_string(), Ty::Str)],
                return_ty: Ty::None,
                body: vec![],
            },
        ],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    assert!(check_and_resolve(&hir).is_ok());
}
