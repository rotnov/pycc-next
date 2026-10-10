//! Static, class and instance methods of exported classes.

use super::*;

// --- #1143: methods in the export set ----------------------------------

#[test]
fn a_public_static_and_class_method_of_a_public_class_are_exported() {
    // The mangled spellings `pycc_hir` actually emits: `.static` for a
    // `@staticmethod`, `.classmethod` for a `@classmethod`. The classmethod's
    // own parameter list leads with `cls`, which is the receiver and never
    // crosses the boundary, so `params` holds the receiver-free tail.
    let hir = module_with_classes(
        vec![
            func("Grid.scale.static", &[("n", Ty::Int)], Ty::Int),
            func(
                "Grid.make.classmethod",
                &[
                    ("cls", Ty::Instance(Box::new("Grid".to_string()))),
                    ("n", Ty::Int),
                ],
                Ty::Int,
            ),
        ],
        vec![("Grid".to_string(), class_def("Grid", None))],
    );
    assert_eq!(
        collect_exports(&hir).expect("both methods are carriable"),
        vec![
            ExtExport {
                defaults: Vec::new(),
                name: "Grid.scale.static".to_string(),
                class: Some("Grid".to_string()),
                method: Some("scale".to_string()),
                returns_buffer_slice: false,
                receiver: ExtReceiver::None,
                params: vec![Ty::Int],
                param_writable: vec![false; 1],
                return_ty: Ty::Int,
                keyword_names: None,
            },
            ExtExport {
                defaults: Vec::new(),
                name: "Grid.make.classmethod".to_string(),
                class: Some("Grid".to_string()),
                method: Some("make".to_string()),
                returns_buffer_slice: false,
                receiver: ExtReceiver::NullCls,
                params: vec![Ty::Int],
                param_writable: vec![false; 1],
                return_ty: Ty::Int,
                keyword_names: None,
            },
        ]
    );
}

#[test]
fn a_method_of_a_user_exception_class_is_excluded_by_its_exception_type_tag() {
    // The exclusion selector is `HirClassDef::exception_type_tag.is_some()`,
    // deliberately and not `collect_user_exception_classes`' own selector:
    // the tag is set for exactly the classes the runtime treats as
    // exceptions, and the two selectors drifting apart must not be able to
    // widen the export set behind this rule. This test pins the tag.
    let hir = module_with_classes(
        vec![
            func("Failure.of.static", &[("n", Ty::Int)], Ty::Int),
            func("Grid.scale.static", &[("n", Ty::Int)], Ty::Int),
        ],
        vec![
            ("Failure".to_string(), class_def("Failure", Some(7))),
            ("Grid".to_string(), class_def("Grid", None)),
        ],
    );
    let exports = collect_exports(&hir).expect("no gap remains");
    assert_eq!(
        exports.iter().map(|e| e.name.as_str()).collect::<Vec<_>>(),
        vec!["Grid.scale.static"]
    );
}

#[test]
fn a_private_class_or_a_private_method_leaves_the_export_set() {
    // D-038's `is_public_name` is applied to the class name and the method
    // name alike, never forked: either leading underscore removes the member.
    let hir = module_with_classes(
        vec![
            func("_Grid.scale.static", &[("n", Ty::Int)], Ty::Int),
            func("Grid._scale.static", &[("n", Ty::Int)], Ty::Int),
            func("Grid.scale.static", &[("n", Ty::Int)], Ty::Int),
        ],
        vec![
            ("_Grid".to_string(), class_def("_Grid", None)),
            ("Grid".to_string(), class_def("Grid", None)),
        ],
    );
    let exports = collect_exports(&hir).expect("no gap remains");
    assert_eq!(
        exports.iter().map(|e| e.name.as_str()).collect::<Vec<_>>(),
        vec!["Grid.scale.static"]
    );
}

#[test]
fn a_bare_method_and_a_property_setter_are_refused_as_representation_not_as_a_gap() {
    // An instance method (`Grid.area`), a property getter (also spelled
    // `Grid.width`) and a property setter (`Grid.width.setter`) are excluded
    // *as representation*: they are never routed to the `C0003` gap
    // collector, so an unexportable signature on one of them is silence and
    // not a diagnostic. The signatures here are deliberately uncarriable --
    // a `list` parameter would be a `C0003` on any member that were in the
    // export set -- so the assertion distinguishes "excluded" from
    // "admitted and carriable".
    let hir = module_with_classes(
        vec![
            func(
                "Grid.area",
                &[("self", Ty::Instance(Box::new("Grid".to_string())))],
                Ty::List(Box::new(Ty::Int)),
            ),
            func(
                "Grid.width.setter",
                &[
                    ("self", Ty::Instance(Box::new("Grid".to_string()))),
                    ("value", Ty::List(Box::new(Ty::Int))),
                ],
                Ty::None,
            ),
        ],
        vec![("Grid".to_string(), class_def("Grid", None))],
    );
    assert_eq!(
        collect_exports(&hir).expect("neither member is a capability gap"),
        Vec::new()
    );
}

#[test]
fn an_unexportable_method_signature_is_a_capability_gap_naming_the_source_spelling() {
    // The subject renders `Grid.scale`, the source-level spelling -- never
    // `Grid.scale.static`, which is a compiler-internal mangling the user
    // never wrote. The remedy renames the *method*, so it renders
    // `Grid._scale` and not `_Grid.scale`.
    let hir = module_with_classes(
        vec![func(
            "Grid.scale.static",
            &[("who", Ty::List(Box::new(Ty::Int)))],
            Ty::Int,
        )],
        vec![("Grid".to_string(), class_def("Grid", None))],
    );
    let gaps = collect_exports(&hir).expect_err("list is not bridged");
    assert_eq!(gaps.len(), 1);
    assert_eq!(gaps[0].code, EXT_CAPABILITY_CODE);
    let message = &gaps[0].message;
    assert!(message.contains("method `Grid.scale`"), "{message}");
    assert!(!message.contains("Grid.scale.static"), "{message}");
    assert!(message.contains("`Grid._scale`"), "{message}");
    assert!(!message.contains("`_Grid.scale`"), "{message}");
}

#[test]
fn the_underscore_remedy_a_method_gap_prints_actually_removes_it_from_the_export_set() {
    // A remedy that does not work is worse than none: rename the method the
    // way the message above spells it and the member leaves the export set,
    // taking its gap with it.
    let hir = module_with_classes(
        vec![func(
            "Grid._scale.static",
            &[("who", Ty::List(Box::new(Ty::Int)))],
            Ty::Int,
        )],
        vec![("Grid".to_string(), class_def("Grid", None))],
    );
    assert_eq!(
        collect_exports(&hir).expect("the renamed method is no longer exported"),
        Vec::new()
    );
}

#[test]
fn two_static_methods_of_one_class_with_the_same_name_export_once() {
    // Python rebinds rather than redeclares inside a class body too, and the
    // dedup key is `(class, method)`: a bare name key would collapse two
    // different classes' `scale`. A mangled-name key is the opposite defect
    // and is pinned by the test below.
    let hir = module_with_classes(
        vec![
            func("Grid.scale.static", &[("n", Ty::Int)], Ty::Int),
            func(
                "Grid.scale.static",
                &[("a", Ty::Int), ("b", Ty::Int)],
                Ty::Int,
            ),
            func("Other.scale.static", &[("n", Ty::Int)], Ty::Int),
        ],
        vec![
            ("Grid".to_string(), class_def("Grid", None)),
            ("Other".to_string(), class_def("Other", None)),
        ],
    );
    let exports = collect_exports(&hir).expect("a rebind is not a capability gap");
    assert_eq!(
        exports
            .iter()
            .map(|e| (e.name.as_str(), e.params.len()))
            .collect::<Vec<_>>(),
        vec![("Grid.scale.static", 2), ("Other.scale.static", 1)]
    );
}

#[test]
#[should_panic(expected = "is spelled as a method with a receiver but has no parameters")]
fn a_class_method_without_a_receiver_parameter_is_an_internal_error() {
    // `classify_export_name` reads the `.classmethod` suffix and nothing
    // else, so the guarantee that such a function leads with `cls` belongs
    // to `pycc_hir::class`, a crate away. Hand-built HIR can violate it;
    // real HIR cannot. Pin the failure mode as a named internal error rather
    // than as an index-out-of-bounds slice panic. #1145 widened the same
    // guard to the instance-method receiver, so the message names "a method
    // with a receiver" rather than a `@classmethod` alone.
    let hir = module_with_classes(
        vec![func("Grid.make.classmethod", &[], Ty::Int)],
        vec![("Grid".to_string(), class_def("Grid", None))],
    );
    let _ = collect_exports(&hir);
}

#[test]
fn a_static_and_a_class_method_of_one_class_with_the_same_name_export_once() {
    // The dedup key is `(class, method)` and not the mangled name, so the
    // two kind suffixes of one `(class, method)` pair collapse to the last
    // binding exactly as Python's own class body does. A mangled-name key
    // would admit both, and nothing downstream would reject them: the two
    // wrappers carry distinct C symbols, so the C compiler stays silent and
    // the duplicate surfaces only as two `PyMethodDef` entries with the same
    // `ml_name` in one type's method table -- a runtime shadowing, not a
    // build failure.
    let hir = module_with_classes(
        vec![
            func("Grid.scale.static", &[("n", Ty::Int)], Ty::Int),
            func(
                "Grid.scale.classmethod",
                &[("cls", Ty::Int), ("a", Ty::Int), ("b", Ty::Int)],
                Ty::Int,
            ),
        ],
        vec![("Grid".to_string(), class_def("Grid", None))],
    );
    let exports = collect_exports(&hir).expect("a rebind is not a capability gap");
    assert_eq!(
        exports
            .iter()
            .map(|e| (e.name.as_str(), e.receiver, e.params.len()))
            .collect::<Vec<_>>(),
        vec![("Grid.scale.classmethod", ExtReceiver::NullCls, 2)]
    );
}

#[test]
fn the_driver_and_codegen_export_predicates_agree_on_every_shape() {
    // `src/ext_build.rs`'s `classify_export_name` and
    // `pycc_codegen::is_ext_exportable_name` duplicate one predicate across a
    // crate boundary, because `pycc_codegen` does not depend on `pycc_hir`
    // and so cannot call the one that owns `is_public_name`. Duplication
    // across a crate boundary is only safe while something proves the two
    // copies equal, which is this test. The codegen mirror is allowed to be
    // a *superset* on the class-table question alone -- it cannot see
    // `exception_type_tag`, `HirClassDef::properties` or #1145's
    // constructibility predicate -- so the corpus here carries no
    // class-table dependence.
    for name in [
        "f",
        "_f",
        "",
        "Grid.scale.static",
        "Grid.make.classmethod",
        "_Grid.scale.static",
        "Grid._scale.static",
        "Grid.scale",
        // #1145: the bare spelling is now admitted on both sides. It is
        // `MethodKind::Regular`, `PropertyGetter` or `AbstractMethod` and
        // nothing lexical tells them apart, so the two non-`Regular` kinds
        // are removed by `collect_exports`' driver filters instead -- which
        // is exactly why the mirror is allowed to stay a superset.
        "Grid.__init__",
        "_Grid.area",
        "Grid._area",
        "Grid.width.setter",
        "Grid.scale.other",
        "Grid.scale.static.extra",
        // D-254: `super()`-target copies, with and without a kind suffix.
        "Grid.scale.0super_Base",
        "Grid.scale.0super_Base.classmethod",
        "0gen_identity_int",
        "0gen_f.scale.static",
        "Grid..static",
        ".static",
        // #1467: the PEP 562 hooks, which the driver admits through
        // `collect_exports_with_hooks` rather than `classify_export_name`.
        "__getattr__",
        "__dir__",
        "__getattr__x",
        "__getattribute__",
        "_getattr",
        "Grid.__getattr__",
        // #1427: the comparison and hash slot dunders, on a public or a
        // private class, which the driver admits through `richcompare`.
        "Grid.__eq__",
        "_Grid.__lt__",
        "Grid.__hash__",
        "Grid.__eq__.static",
        "Grid.__bool__",
        "__eq__",
        ".__eq__",
        "0gen_Grid.__eq__",
    ] {
        assert_eq!(
            classify_export_name(name).is_some()
                || is_module_hook(name)
                || is_slot_dunder_method(name),
            pycc_codegen::is_ext_exportable_name(name),
            "predicates disagree on {name:?}"
        );
    }
    // #1427: every slot dunder the driver installs, on a public and a
    // private class, so the two crates' `SLOT_DUNDERS` lists cannot drift.
    for dunder in SLOT_DUNDERS {
        for name in [format!("Grid.{dunder}"), format!("_Grid.{dunder}")] {
            assert!(is_slot_dunder_method(&name), "{name:?}");
            assert!(
                pycc_codegen::is_ext_exportable_name(&name),
                "codegen does not export {name:?}"
            );
        }
    }
    assert!(classify_export_name("Grid.scale.0super_Base.classmethod").is_none());
}
