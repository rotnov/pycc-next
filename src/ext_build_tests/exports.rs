//! Export-set and capability-gap tests for the `--ext` build seam: which
//! public module-level functions `collect_exports` admits, and the `C0003`
//! diagnostic it raises for every signature the boundary cannot carry.
//!
//! Split out of `ext_build_tests.rs` by #1049 under `AGENTS.md`'s
//! decomposability rule; the tests themselves are unchanged except where
//! #1049's own widening required re-pointing a fixture.

use super::*;

#[test]
fn every_public_carriable_module_level_function_is_exported_in_source_order() {
    let hir = module(vec![
        func("first", &[("x", Ty::Int)], Ty::Int),
        func("second", &[], Ty::Int),
        func("third", &[("a", Ty::Int), ("b", Ty::Int)], Ty::Int),
        // #1048 widened the boundary: each of these reaches the export set
        // rather than the gap list, and the signature travels with it.
        func("scaled", &[("factor", Ty::Float)], Ty::Float),
        func("negated", &[("flag", Ty::Bool)], Ty::Bool),
        func("sink", &[("x", Ty::Int)], Ty::None),
    ]);
    assert_eq!(
        collect_exports(&hir).expect("a carriable program exports cleanly"),
        vec![
            ExtExport {
                defaults: Vec::new(),
                name: "first".to_string(),
                class: None,
                method: None,
                returns_buffer_slice: false,
                receiver: ExtReceiver::None,
                params: vec![Ty::Int],
                param_writable: vec![false; 1],
                return_ty: Ty::Int,
                keyword_names: None,
            },
            ExtExport {
                defaults: Vec::new(),
                name: "second".to_string(),
                class: None,
                method: None,
                returns_buffer_slice: false,
                receiver: ExtReceiver::None,
                params: Vec::new(),
                param_writable: Vec::new(),
                return_ty: Ty::Int,
                keyword_names: None,
            },
            ExtExport {
                defaults: Vec::new(),
                name: "third".to_string(),
                class: None,
                method: None,
                returns_buffer_slice: false,
                receiver: ExtReceiver::None,
                params: vec![Ty::Int, Ty::Int],
                param_writable: vec![false; 2],
                return_ty: Ty::Int,
                keyword_names: None,
            },
            ExtExport {
                defaults: Vec::new(),
                name: "scaled".to_string(),
                class: None,
                method: None,
                returns_buffer_slice: false,
                receiver: ExtReceiver::None,
                params: vec![Ty::Float],
                param_writable: vec![false; 1],
                return_ty: Ty::Float,
                keyword_names: None,
            },
            ExtExport {
                defaults: Vec::new(),
                name: "negated".to_string(),
                class: None,
                method: None,
                returns_buffer_slice: false,
                receiver: ExtReceiver::None,
                params: vec![Ty::Bool],
                param_writable: vec![false; 1],
                return_ty: Ty::Bool,
                keyword_names: None,
            },
            ExtExport {
                defaults: Vec::new(),
                name: "sink".to_string(),
                class: None,
                method: None,
                returns_buffer_slice: false,
                receiver: ExtReceiver::None,
                params: vec![Ty::Int],
                param_writable: vec![false; 1],
                return_ty: Ty::None,
                keyword_names: None,
            },
        ]
    );
}

#[test]
fn a_private_name_a_method_and_a_monomorphized_specialization_are_not_exports() {
    let hir = module(vec![
        // D-038: a leading underscore is private, and `--ext` uses the same
        // predicate as every other visibility decision in this compiler.
        func("_helper", &[("x", Ty::Str)], Ty::Str),
        // A *bare* dotted name is a regular method, a property getter or an
        // abstract method -- this spelling cannot tell them apart -- and
        // #1143 admits none of the three. It is refused as representation,
        // so it is neither exported nor a gap. Only the `.static` and
        // `.classmethod` spellings reach the export set.
        func("Point.norm", &[("self", Ty::Float)], Ty::Float),
        // A monomorphized specialization has no `fnptr_` global to call
        // through -- codegen dispatches it directly.
        func("0gen_identity_int", &[("x", Ty::Str)], Ty::Str),
        // A top-level statement is not a function at all.
        HirItem::TopLevelStmt(pycc_hir::HirStmt::ExprStmt(pycc_hir::HirExpr::IntLiteral(
            0,
        ))),
        func("kept", &[], Ty::Int),
    ]);
    assert_eq!(
        collect_exports(&hir).expect("no public gap remains"),
        vec![ExtExport {
            defaults: Vec::new(),
            name: "kept".to_string(),
            class: None,
            method: None,
            returns_buffer_slice: false,
            receiver: ExtReceiver::None,
            params: Vec::new(),
            param_writable: Vec::new(),
            return_ty: Ty::Int,
            keyword_names: None,
        }]
    );
}

#[test]
fn a_rebound_public_name_is_exported_once_with_the_last_definition_s_signature() {
    // Python rebinds rather than redeclares, and codegen follows it: one
    // `fnptr_two` global, bound to the second `def`. A second table entry
    // would emit `pycc_ext_wrap_two` twice and the C compiler would reject
    // the redefinition, so the export set collapses the rebind here.
    let hir = module(vec![
        func("one", &[], Ty::Int),
        func("two", &[("x", Ty::Int)], Ty::Int),
        func("three", &[], Ty::Int),
        func("two", &[("a", Ty::Int), ("b", Ty::Int)], Ty::Int),
    ]);
    assert_eq!(
        collect_exports(&hir).expect("a rebind is not a capability gap"),
        vec![
            ExtExport {
                defaults: Vec::new(),
                name: "one".to_string(),
                class: None,
                method: None,
                returns_buffer_slice: false,
                receiver: ExtReceiver::None,
                params: Vec::new(),
                param_writable: Vec::new(),
                return_ty: Ty::Int,
                keyword_names: None,
            },
            // Definition order, last definition's signature: the entry keeps
            // the position the name first claimed.
            ExtExport {
                defaults: Vec::new(),
                name: "two".to_string(),
                class: None,
                method: None,
                returns_buffer_slice: false,
                receiver: ExtReceiver::None,
                params: vec![Ty::Int, Ty::Int],
                param_writable: vec![false; 2],
                return_ty: Ty::Int,
                keyword_names: None,
            },
            ExtExport {
                defaults: Vec::new(),
                name: "three".to_string(),
                class: None,
                method: None,
                returns_buffer_slice: false,
                receiver: ExtReceiver::None,
                params: Vec::new(),
                param_writable: Vec::new(),
                return_ty: Ty::Int,
                keyword_names: None,
            },
        ]
    );
}

#[test]
fn a_program_with_no_public_function_exports_nothing_rather_than_failing() {
    let hir = module(vec![func("_only", &[], Ty::Int)]);
    assert_eq!(collect_exports(&hir).expect("not an error"), Vec::new());
}

#[test]
fn a_parameter_the_boundary_cannot_carry_is_a_capability_gap_naming_it() {
    // `list[int]` rather than `str`: #1049 made `str` carriable, so the
    // original fixture would no longer produce a gap at all. `list` is the
    // nearest still-unadmitted parameter type now that #1050 admits
    // `tuple` -- and it stays unadmitted because it is a mutable object
    // with no by-value crossing, not merely an unimplemented one.
    let hir = module(vec![func(
        "greet",
        &[("who", Ty::List(Box::new(Ty::Int)))],
        Ty::Int,
    )]);
    let gaps = collect_exports(&hir).expect_err("list is not bridged");
    assert_eq!(gaps.len(), 1);
    assert_eq!(gaps[0].code, EXT_CAPABILITY_CODE);
    assert_eq!(gaps[0].span, None);
    let message = &gaps[0].message;
    assert!(message.contains("`who: list`"), "{message}");
    // Both ways out stay in the message: D-038's `_`-prefix opt-out, and
    // dropping `--ext` altogether.
    assert!(message.contains("`_greet`"), "{message}");
    assert!(message.contains("without --ext"), "{message}");
}

#[test]
fn a_none_parameter_is_still_a_capability_gap_after_the_scalar_widening() {
    // `None` is admissible as a return type and not as a parameter, so the
    // two admissible sets are asked separately. This arm stays live until
    // #1047's call-argument ICE is fixed; widening it here would turn a
    // diagnostic into a codegen assertion failure.
    let hir = module(vec![func("sink", &[("x", Ty::None)], Ty::None)]);
    let gaps = collect_exports(&hir).expect_err("a None parameter is not carriable");
    assert_eq!(gaps.len(), 1);
    assert!(gaps[0].message.contains("`x: None`"), "{}", gaps[0].message);
}

#[test]
fn a_return_type_the_boundary_cannot_carry_is_a_capability_gap_naming_it() {
    // `list[int]` for the same reason as the parameter arm above: `str`
    // returns are carried after #1049.
    let hir = module(vec![func(
        "name_of",
        &[("x", Ty::Int)],
        Ty::List(Box::new(Ty::Int)),
    )]);
    let gaps = collect_exports(&hir).expect_err("list is not bridged");
    assert_eq!(gaps.len(), 1);
    assert!(gaps[0].message.contains("`-> list`"), "{}", gaps[0].message);
}

#[test]
fn a_bool_signature_is_carried_rather_than_gapped_and_keeps_its_own_slot() {
    // A declared `bool` parameter is not the D-141 encoded word an `int`
    // slot carries: the compiled function's own ABI slot is a plain `i8`
    // holding 0/1 -- `i8` at the parameter position too, since parameters
    // and returns share one `ty_to_basic_type`. So it is carried by its own
    // helper and its own one-byte C type, never through the `int` path.
    let hir = module(vec![func("flag", &[("b", Ty::Bool)], Ty::Bool)]);
    let exports = collect_exports(&hir).expect("bool is carried by #1048");
    assert_eq!(
        exports,
        vec![ExtExport {
            defaults: Vec::new(),
            name: "flag".to_string(),
            class: None,
            method: None,
            returns_buffer_slice: false,
            receiver: ExtReceiver::None,
            params: vec![Ty::Bool],
            param_writable: vec![false; 1],
            return_ty: Ty::Bool,
            keyword_names: None,
        }]
    );
}

#[test]
fn a_str_signature_is_carried_rather_than_gapped_in_either_position() {
    // Part 2 of #1037 (#1049): `str` is admissible at a parameter and as a
    // return type. It reaches the wrapper as an opaque `void *` in both
    // positions -- `ty_to_basic_type` gives `Ty::Str` a pointer -- so the
    // export set records it exactly like any other carriable type.
    let hir = module(vec![func("echo", &[("s", Ty::Str)], Ty::Str)]);
    let exports = collect_exports(&hir).expect("str is carried by #1049");
    assert_eq!(
        exports,
        vec![ExtExport {
            defaults: Vec::new(),
            name: "echo".to_string(),
            class: None,
            method: None,
            returns_buffer_slice: false,
            receiver: ExtReceiver::None,
            params: vec![Ty::Str],
            param_writable: vec![false; 1],
            return_ty: Ty::Str,
            keyword_names: None,
        }]
    );
}

#[test]
fn a_tuple_signature_is_carried_rather_than_gapped_in_either_position() {
    // Part 3 of #1037 (#1050): a `tuple` of carriable scalars is admissible
    // at a parameter and as a return type. The export entry records the
    // declared `Ty` unchanged -- the flattening into scalar slots is the
    // wrapper's and the thunk's business, and `collect_exports` stays a
    // statement about the Python signature.
    let hir = module(vec![func(
        "swap",
        &[("t", Ty::Tuple(Box::new(vec![Ty::Int, Ty::Float])))],
        Ty::Tuple(Box::new(vec![Ty::Float, Ty::Bool])),
    )]);
    let exports = collect_exports(&hir).expect("tuple is carried by #1050");
    assert_eq!(
        exports,
        vec![ExtExport {
            defaults: Vec::new(),
            name: "swap".to_string(),
            class: None,
            method: None,
            returns_buffer_slice: false,
            receiver: ExtReceiver::None,
            params: vec![Ty::Tuple(Box::new(vec![Ty::Int, Ty::Float]))],
            param_writable: vec![false; 1],
            return_ty: Ty::Tuple(Box::new(vec![Ty::Float, Ty::Bool])),
            keyword_names: None,
        }]
    );
}

#[test]
fn a_tuple_of_something_uncarriable_is_a_capability_gap_naming_the_tuple() {
    // The element type is what fails, but the message names the parameter's
    // own declared type: `render_ty` renders a `tuple` as the bare word, and
    // the reader's fix is to change the signature, not to reason about which
    // element the boundary refused. `tuple[list[int]]` is reachable today
    // only from hand-built HIR -- T0039 refuses it at the type checker --
    // so this is the defensive arm, kept because `boundary_carrier` recurses
    // and a recursion that cannot fail is a claim, not a fact.
    let hir = module(vec![func(
        "boxes",
        &[("t", Ty::Tuple(Box::new(vec![Ty::List(Box::new(Ty::Int))])))],
        Ty::Int,
    )]);
    let gaps = collect_exports(&hir).expect_err("a list element is not carriable");
    assert_eq!(gaps.len(), 1);
    assert_eq!(gaps[0].code, EXT_CAPABILITY_CODE);
    let message = &gaps[0].message;
    assert!(message.contains("`t: tuple`"), "{message}");
    // The remediation enumerates what the boundary *does* carry, and Part 3
    // of #1037 (#1050) put `tuple` into that list. A reader who reaches this
    // message through a `tuple` gap has to be told which tuples are carried,
    // not shown a scalar-only list that reads as "no tuple at all". #1129
    // put the buffer's second spelling into it for the same reason: a
    // reader who wrote `ndarray` and is shown a list naming only
    // `memoryview` reads it as "not that type at all". #1134 added the
    // third spelling `NDArray` on the same reasoning -- it is the one 18 of
    // the 19 array-parameter occurrences in the #1039 census use. Part 2b of
    // #1142 (#1164) removed the return position's "except the buffer"
    // carve-out: a buffer return is now carried, so a message that still
    // excluded it would send a reader who wrote a valid `-> memoryview`
    // signature looking for a gap that no longer exists.
    assert!(
        message.contains(
            "a parameter must be `int`, `float`, `bool`, `str`, `memoryview` (or its \
             other spellings `ndarray` and `NDArray`), a CPython object (`Any`, `object`, \
             a foreign class, or a container of objects), an instance of a class compiled \
             in this module (not an enum or an exception class), or a `tuple` of \
             `int`/`float`/`bool`, and a \
             return type must be one of those, or `None`"
        ),
        "{message}"
    );
    assert!(!message.contains("except the buffer"), "{message}");
}

#[test]
fn a_str_element_is_a_capability_gap_even_though_str_itself_is_carried() {
    // The one element type that is a scalar at a top-level position and
    // still not carriable inside a tuple. #1049 gave `str` its own C slot
    // and `pycc_ext_unpack_str`, but the element shims are the `_at`
    // variants, which exist only for D-116's three element types. Were
    // this admitted, the wrapper would render a call to an undeclared
    // `pycc_ext_unpack_str_at` and the artifact would die in clang rather
    // than as a `C0003` gap -- so the narrowing lives in
    // `boundary_carrier`'s tuple arm and not in `into_scalar`, which
    // passes every scalar through. Reachable today only from hand-built
    // HIR: `T0039` refuses `tuple[int, str]` at the type checker.
    let hir = module(vec![func(
        "label",
        &[("t", Ty::Tuple(Box::new(vec![Ty::Str, Ty::Int])))],
        Ty::Int,
    )]);
    let gaps = collect_exports(&hir).expect_err("a str element is not carriable");
    assert_eq!(gaps.len(), 1);
    assert_eq!(gaps[0].code, EXT_CAPABILITY_CODE);
    let message = &gaps[0].message;
    assert!(message.contains("`t: tuple`"), "{message}");
}

#[test]
fn a_nested_tuple_return_is_a_capability_gap_rather_than_a_flattening() {
    // The one arm that would be silently wrong if it were admitted: a
    // nested tuple has no single scalar slot, and flattening it would make
    // `tuple[tuple[int, int], int]` and `tuple[int, int, int]` indis-
    // tinguishable at the seam -- the wrapper would then re-pack three
    // elements into a two-element Python tuple. D-116 gives a tuple type a
    // fixed arity of scalar elements, and the boundary carries exactly that.
    let hir = module(vec![func(
        "nest",
        &[("x", Ty::Int)],
        Ty::Tuple(Box::new(vec![Ty::Tuple(Box::new(vec![Ty::Int])), Ty::Int])),
    )]);
    let gaps = collect_exports(&hir).expect_err("a nested tuple is not carriable");
    assert_eq!(gaps.len(), 1);
    let message = &gaps[0].message;
    assert!(message.contains("`-> tuple`"), "{message}");
}

#[test]
fn a_private_name_carrying_a_tuple_is_neither_exported_nor_gapped() {
    // The D-038 name filter runs before the type admissibility question, so
    // an underscore-prefixed helper is invisible to the boundary whatever it
    // carries. This is the same set `pycc_codegen`'s `is_ext_exportable_name`
    // decides thunk emission from: a name excluded here gets no wrapper, so
    // a thunk for it would advertise a symbol nothing calls.
    let hir = module(vec![func(
        "_scratch",
        &[("t", Ty::Tuple(Box::new(vec![Ty::List(Box::new(Ty::Int))])))],
        Ty::Tuple(Box::new(vec![Ty::Int])),
    )]);
    assert_eq!(collect_exports(&hir).expect("not an error"), Vec::new());
}

#[test]
fn every_gap_in_a_program_is_collected_before_the_build_gives_up() {
    let hir = module(vec![
        // `dict`, not `list`: `d` below is already the `list` arm, and two
        // gaps rendering the same spelling would stop distinguishing which
        // index belongs to which function. The key type is `int` rather than
        // `str` so nothing here depends on a type #1049 made carriable.
        func(
            "a",
            &[("x", Ty::Dict(Box::new((Ty::Int, Ty::Int))))],
            Ty::Int,
        ),
        func("b", &[("x", Ty::None)], Ty::Int),
        // Carriable after #1048/#1049, so none of these joins the gap list.
        func("c", &[("x", Ty::Float)], Ty::None),
        func("d", &[("xs", Ty::List(Box::new(Ty::Int)))], Ty::Int),
        func("e", &[("x", Ty::Bool)], Ty::Int),
        func("f", &[("s", Ty::Str)], Ty::Str),
    ]);
    let gaps = collect_exports(&hir).expect_err("three of the six are gaps");
    assert_eq!(gaps.len(), 3);
    assert!(gaps[0].message.contains("`x: dict`"), "{}", gaps[0].message);
    assert!(gaps[1].message.contains("`x: None`"), "{}", gaps[1].message);
    let third = &gaps[2].message;
    assert!(third.contains("`xs: list`"), "{third}");
}

/// D-258 rule 5 (#1397), replacing Part 1 of #1026 work item 12's refusal:
/// the opaque CPython object is carried at a parameter position as the
/// `PyObject *` itself. Reachable from Python source as an `Any`/`object`
/// annotation, a container of objects, and a class a foreign import binds
/// (unchecked against the class, D-244's #1386 amendment).
#[test]
fn an_object_typed_parameter_is_carried_at_the_export_boundary() {
    let hir = module(vec![func("wrap", &[("x", Ty::Object)], Ty::Int)]);
    let exports = collect_exports(&hir).expect("an opaque object is carriable");
    assert_eq!(exports.len(), 1);
    assert_eq!(exports[0].params, vec![Ty::Object]);
    assert_eq!(
        boundary_carrier(&Ty::Object),
        Some(BoundaryCarrier::Scalar("void *", "object"))
    );
}

/// The return half: `collect_exports` asks the two positions against
/// separate admissible sets, so the return is pinned on its own.
#[test]
fn an_object_return_type_is_carried_at_the_export_boundary() {
    let hir = module(vec![func("fetch", &[("x", Ty::Int)], Ty::Object)]);
    let exports = collect_exports(&hir).expect("an opaque object return is carriable");
    assert_eq!(exports[0].return_ty, Ty::Object);
    assert_eq!(return_c_type(&Ty::Object), Some("void *"));
}

/// A `tuple` element stays refused: there is no
/// `pycc_ext_unpack_object_at` element shim. Unreachable from an `ext`
/// source, where D-258 rule 4 collapses such a tuple to the object itself.
#[test]
fn an_object_tuple_element_is_still_a_capability_gap() {
    let hir = module(vec![func(
        "pair",
        &[("t", Ty::Tuple(Box::new(vec![Ty::Int, Ty::Object])))],
        Ty::Int,
    )]);
    let gaps = collect_exports(&hir).expect_err("no `_at` shim for an object element");
    let message = &gaps[0].message;
    assert!(message.contains("`t: tuple`"), "{message}");
}

#[test]
fn every_ty_the_gap_message_can_name_renders_a_python_spelling() {
    let cases = [
        (Ty::Float, "float"),
        (Ty::Bool, "bool"),
        (Ty::Str, "str"),
        (Ty::None, "None"),
        (Ty::List(Box::new(Ty::Int)), "list"),
        (Ty::Dict(Box::new((Ty::Str, Ty::Int))), "dict"),
        (Ty::Set(Box::new(Ty::Int)), "set"),
        (Ty::FrozenSet(Box::new(Ty::Int)), "frozenset"),
        (Ty::Tuple(Box::new(vec![Ty::Int])), "tuple"),
        (Ty::Object, "object"),
        (Ty::MemoryView, "memoryview"),
        (Ty::Param(Box::new("T".to_string())), "that type"),
    ];
    for (ty, spelling) in cases {
        assert_eq!(render_ty(&ty), spelling);
        assert_eq!(render_ty(&Ty::Int), "int");
    }
}

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

// --- #1145: instance methods, and the constructibility predicate ---------

/// The module every constructibility test below varies one field of: a
/// public class with a public instance method and a carriable `__init__`.
pub(super) fn constructible_module(class: &str) -> HirModule {
    let mut hir = module_with_classes(
        vec![
            init_func(class, &[("w", Ty::Int), ("h", Ty::Int)], Ty::None),
            func(
                &format!("{class}.area"),
                &[("self", Ty::Instance(Box::new(class.to_string())))],
                Ty::Int,
            ),
        ],
        vec![(class.to_string(), constructible_class_def(class))],
    );
    declare_member(&mut hir, class, Member::Method("area"));
    hir
}

/// Which of [`pycc_hir::HirClassDef`]'s member tables a fixture's binding
/// belongs in. `collect_class_publications` resolves a name against those
/// tables, so a fixture that pushes a `<Class>.<method>` item without the
/// matching table entry describes a class `pycc_hir::class` could never
/// have lowered -- and would exercise the namespace walk against a class
/// that binds nothing.
enum Member<'a> {
    Method(&'a str),
    Property(&'a str),
    Static(&'a str),
}

/// Record `member` in `class`'s own table, so the class table says what the
/// fixture's [`HirItem`]s already say.
fn declare_member(hir: &mut HirModule, class: &str, member: Member<'_>) {
    let (_, def) = hir
        .class_defs
        .iter_mut()
        .find(|(held, _)| held == class)
        .expect("the fixture defines the class it declares a member on");
    match member {
        Member::Method(name) => def
            .methods
            .push((name.to_string(), format!("{class}.{name}"))),
        Member::Property(name) => def.properties.push(pycc_hir::PropertyDef {
            name: name.to_string(),
            getter: format!("{class}.{name}"),
            setter: None,
        }),
        Member::Static(name) => def
            .static_methods
            .push((name.to_string(), format!("{class}.{name}.static"))),
    }
}

/// Drop both `class.method`'s compiled item and its method-table entry --
/// the inverse of [`declare_member`] for a regular method, used by the
/// fixtures that remove a class's only own export.
fn undeclare_method(hir: &mut HirModule, class: &str, method: &str) {
    let mangled = format!("{class}.{method}");
    hir.items
        .retain(|item| !matches!(item, HirItem::Function { name, .. } if *name == mangled));
    let (_, def) = hir
        .class_defs
        .iter_mut()
        .find(|(held, _)| held == class)
        .expect("the fixture defines the class it undeclares a method on");
    def.methods.retain(|(name, _)| name != method);
}

#[test]
fn an_instance_method_of_a_constructible_class_is_exported_with_a_real_receiver() {
    let exports = collect_exports(&constructible_module("Grid")).expect("a carriable signature");
    assert_eq!(
        exports
            .iter()
            .map(|e| (
                e.name.as_str(),
                e.class.as_deref(),
                e.method.as_deref(),
                e.receiver,
                e.params.len()
            ))
            .collect::<Vec<_>>(),
        // `__init__` is not public under `is_public_name`, so it is never an
        // export in its own right; only `area` is, and its `self` is split
        // off exactly as a `@classmethod`'s `cls` is.
        vec![(
            "Grid.area",
            Some("Grid"),
            Some("area"),
            ExtReceiver::SelfInstance,
            0
        )]
    );
}

#[test]
fn a_constructible_class_yields_one_constructor_descriptor_with_the_carried_tail_only() {
    let hir = constructible_module("Grid");
    let exports = collect_exports(&hir).expect("a carriable signature");
    assert_eq!(
        collect_constructors(&hir, &collect_class_publications(&hir, &exports)),
        vec![ExtCtor {
            defaults: Vec::new(),
            class: "Grid".to_string(),
            name: "Grid.__init__".to_string(),
            // `self` is gone; the two `int`s remain, and the slot layout is
            // `flat_attr_layout`'s, not the parameter list -- they agree in
            // count here only because the fixture declares two attributes.
            params: vec![Ty::Int, Ty::Int],
            param_writable: vec![false; 2],
            slot_names: vec!["w".to_string(), "h".to_string()],
            keyword_names: None,
        }]
    );
    // #1442: both `int` slots are carriable, so both are descriptors,
    // indexed in slot order (writable since Part 1 of #1443); since #1448
    // the table is keyed by carrier class rather than held by the ctor.
    assert_eq!(grid_getsets(&hir), grid_slot_getsets());
}

/// Each row removes exactly one constructibility condition from
/// [`constructible_module`] and asserts both consequences at once: the class
/// contributes no constructor descriptor, and its instance method is not an
/// export. The second half is the one that matters -- an exclusion that
/// dropped the constructor but still published the method would emit a
/// wrapper that dereferences a receiver no host can ever build.
#[test]
fn every_constructibility_condition_removes_the_class_and_its_instance_methods() {
    let mut rows: Vec<(&str, HirModule)> = Vec::new();

    let mut abstract_class = constructible_module("Grid");
    abstract_class.class_defs[0].1.is_abstract = true;
    rows.push(("is_abstract", abstract_class));

    let mut protocol = constructible_module("Grid");
    protocol.class_defs[0].1.is_protocol = true;
    rows.push(("is_protocol", protocol));

    let mut enum_class = constructible_module("Grid");
    enum_class.class_defs[0].1.is_enum = true;
    rows.push(("is_enum", enum_class));

    let mut tagged = constructible_module("Grid");
    tagged.class_defs[0].1.exception_type_tag = Some(FIRST_USER_EXCEPTION_TYPE_TAG);
    rows.push(("exception_type_tag", tagged));

    // A seeded builtin exception class carries no tag of its own, so this
    // row is the one `exception_type_tag` alone does not cover.
    let mut builtin_exception = constructible_module("ValueError");
    builtin_exception.class_defs[0].1.exception_type_tag = None;
    rows.push(("is_builtin_exception_class", builtin_exception));

    // An `__init__` that does not return `None` is not a constructor pycc
    // can call for its effect; nothing consumes the value at this boundary.
    let mut returning_init = constructible_module("Grid");
    returning_init.items[0] = init_func("Grid", &[("w", Ty::Int), ("h", Ty::Int)], Ty::Int);
    rows.push(("__init__ return type", returning_init));

    // Since Part 1 of #1447 (#1449) a `Ty::Instance` is carried exactly when
    // it names a regular class of this module, so the parameter that keeps a
    // class non-constructible is an instance of a class the module's table
    // does not hold -- the case a predicate on the type alone would miss.
    // The admitted counterpart is
    // `an_init_taking_a_same_module_instance_makes_the_class_constructible`.
    let mut instance_param = constructible_module("Grid");
    instance_param.items[0] = init_func(
        "Grid",
        &[("other", Ty::Instance(Box::new("Elsewhere".to_string())))],
        Ty::None,
    );
    rows.push(("uncarriable __init__ parameter", instance_param));

    // A `tuple` parameter is carriable everywhere else and refused here
    // alone: the constructor is called through its `fnptr_` slot with no
    // thunk, so no flattening exists for an aggregate (C8).
    let mut tuple_param = constructible_module("Grid");
    tuple_param.items[0] = init_func(
        "Grid",
        &[("wh", Ty::Tuple(Box::new(vec![Ty::Int, Ty::Int])))],
        Ty::None,
    );
    rows.push(("tuple __init__ parameter", tuple_param));

    // No `__init__` resolves at all: `methods` is empty, so `resolved_init`
    // finds nothing in the MRO.
    let mut no_init = constructible_module("Grid");
    no_init.class_defs[0].1.methods.clear();
    rows.push(("no resolved __init__", no_init));

    for (label, hir) in rows {
        let exports = collect_exports(&hir).expect("{label}: not a capability gap");
        assert!(
            exports.is_empty(),
            "{label}: an excluded class still exported {exports:?}"
        );
        assert!(
            collect_constructors(&hir, &collect_class_publications(&hir, &exports)).is_empty(),
            "{label}: an excluded class still yielded a constructor"
        );
    }
}

#[test]
fn a_class_whose_class_def_is_missing_entirely_is_not_constructible() {
    // The class table is what every constructibility condition is read
    // from, so a name with no entry has to fail closed rather than fall
    // through to "nothing refused it".
    let hir = module(vec![
        init_func("Grid", &[("w", Ty::Int)], Ty::None),
        func(
            "Grid.area",
            &[("self", Ty::Instance(Box::new("Grid".to_string())))],
            Ty::Int,
        ),
    ]);
    let exports = collect_exports(&hir).expect("not a capability gap");
    assert!(exports.is_empty(), "{exports:?}");
    assert!(collect_constructors(&hir, &collect_class_publications(&hir, &exports)).is_empty());
}

#[test]
fn a_property_getter_is_excluded_by_the_class_table_not_by_its_name() {
    // The getter's mangled name is the bare `<Class>.<method>` spelling, so
    // nothing lexical separates it from an ordinary method: only
    // `HirClassDef::properties` does. Deleting that driver filter publishes
    // `v` as a callable member.
    let mut hir = constructible_module("Grid");
    hir.items.push(func(
        "Grid.v",
        &[("self", Ty::Instance(Box::new("Grid".to_string())))],
        Ty::Int,
    ));
    hir.class_defs[0].1.properties = vec![pycc_hir::PropertyDef {
        name: "v".to_string(),
        getter: "Grid.v".to_string(),
        setter: None,
    }];
    let exports = collect_exports(&hir).expect("not a capability gap");
    assert_eq!(
        exports.iter().map(|e| e.name.as_str()).collect::<Vec<_>>(),
        vec!["Grid.area"]
    );
}

#[test]
fn the_constructor_is_found_past_an_unrelated_item_of_the_same_kind() {
    // `resolved_init` scans `HirModule::items` for the mangled name the MRO
    // walk produced. Every module of any size has functions before it that
    // are `HirItem::Function` too, so the name guard -- not the variant --
    // is what selects the constructor. With the constructor first in the
    // list, a guard that always matched would pass unnoticed.
    let mut hir = constructible_module("Grid");
    hir.items
        .insert(0, func("decoy", &[("n", Ty::Int)], Ty::Int));
    let (name, params, return_ty) =
        resolved_init(&hir, "Grid").expect("the constructor is still resolved");
    assert_eq!(name, "Grid.__init__");
    assert_eq!(params.len(), 3, "{params:?}");
    assert_eq!(*return_ty, Ty::None);
}

#[test]
fn an_implicit_object_init_ranks_below_a_real_one_in_the_same_mro() {
    // D-232/#966: a base whose `__init__` is the D-225 implicit zero-argument
    // constructor must never win over a real one further along the MRO, or a
    // subclass with a two-argument constructor would be published as taking
    // none. The two-pass ranking is what makes the first pass skip it.
    let mut hir = constructible_module("Grid");
    let mut base = constructible_class_def("Base");
    base.implicit_object_init = true;
    hir.class_defs[0].1.mro = vec!["Base".to_string(), "Grid".to_string()];
    hir.class_defs.push(("Base".to_string(), base));
    hir.items.push(init_func("Base", &[], Ty::None));
    let exports = collect_exports(&hir).expect("not a capability gap");
    let ctors = collect_constructors(&hir, &collect_class_publications(&hir, &exports));
    // `Base` is published too since #1450 (it is constructible through its
    // implicit constructor), so the ranking is read off `Grid`'s own row.
    assert_eq!(
        ctors
            .iter()
            .find(|c| c.class == "Grid")
            .map(|c| c.name.as_str()),
        Some("Grid.__init__"),
        "the implicit `object.__init__` outranked the real one: {ctors:?}"
    );
}

#[test]
fn an_implicit_object_init_is_still_resolved_when_it_is_the_only_one() {
    // The second pass of the same ranking: with no real `__init__` anywhere
    // in the MRO, the implicit one is the constructor, and a class with it
    // is constructible with zero carried arguments.
    let mut hir = constructible_module("Grid");
    hir.class_defs[0].1.implicit_object_init = true;
    hir.items[0] = init_func("Grid", &[], Ty::None);
    let exports = collect_exports(&hir).expect("not a capability gap");
    assert_eq!(
        collect_constructors(&hir, &collect_class_publications(&hir, &exports)),
        vec![ExtCtor {
            defaults: Vec::new(),
            class: "Grid".to_string(),
            name: "Grid.__init__".to_string(),
            params: Vec::new(),
            param_writable: Vec::new(),
            slot_names: vec!["w".to_string(), "h".to_string()],
            keyword_names: None,
        }]
    );
    // #1442: both `int` slots are carriable, so both are descriptors,
    // indexed in slot order (writable since Part 1 of #1443); since #1448
    // the table is keyed by carrier class rather than held by the ctor.
    assert_eq!(grid_getsets(&hir), grid_slot_getsets());
}

#[test]
fn an_instance_method_of_a_constructible_class_with_an_uncarriable_signature_is_a_gap() {
    // The ordering claim D-244 rule 1 rests on: representation exclusions
    // run *before* `unsupported_boundary_ty`, so only a method that survived
    // them can be a capability gap -- and once it has, it is held to the
    // boundary exactly like a module-level function.
    let mut hir = constructible_module("Grid");
    hir.items[1] = func(
        "Grid.area",
        &[
            ("self", Ty::Instance(Box::new("Grid".to_string()))),
            ("xs", Ty::List(Box::new(Ty::Int))),
        ],
        Ty::Int,
    );
    let gap = collect_exports(&hir).expect_err("an uncarriable parameter is a C0003");
    assert_eq!(gap.len(), 1, "{gap:?}");
    assert_eq!(gap[0].code, "C0003", "{gap:?}");
    assert!(gap[0].message.contains("Grid.area"), "{gap:?}");
}

#[test]
fn an_instance_method_of_a_non_constructible_class_is_never_a_gap() {
    // The other arm of the same ordering: the identical uncarriable
    // signature on a class pycc cannot construct is excluded as
    // representation and reported as nothing at all. This is what bounds
    // the new-failure set to constructible classes.
    let mut hir = constructible_module("Grid");
    hir.class_defs[0].1.is_abstract = true;
    hir.items[1] = func(
        "Grid.area",
        &[
            ("self", Ty::Instance(Box::new("Grid".to_string()))),
            ("xs", Ty::List(Box::new(Ty::Int))),
        ],
        Ty::Int,
    );
    assert!(
        collect_exports(&hir)
            .expect("excluded as representation, not reported as a gap")
            .is_empty()
    );
}

#[test]
fn a_constructor_descriptor_is_emitted_once_per_class_however_many_methods_it_exports() {
    // `collect_constructors` walks the export list, which carries one entry
    // per exported method; the `.inc` must declare one `tp_init` per class.
    let mut hir = constructible_module("Grid");
    hir.items.push(func(
        "Grid.perimeter",
        &[("self", Ty::Instance(Box::new("Grid".to_string())))],
        Ty::Int,
    ));
    declare_member(&mut hir, "Grid", Member::Method("perimeter"));
    let exports = collect_exports(&hir).expect("carriable");
    assert_eq!(exports.len(), 2);
    assert_eq!(
        collect_constructors(&hir, &collect_class_publications(&hir, &exports)).len(),
        1
    );
}

// --- #1145 finding 1: MRO-resolved publication ---------------------------

/// `Ty::Instance` for a receiver parameter, spelled once.
fn inst(class: &str) -> Ty {
    Ty::Instance(Box::new(class.to_string()))
}

/// `Base`, with one instance method, plus `Derived(Base)` with one of its
/// own -- both constructible, wired the way `pycc_hir::class` wires an
/// inheriting program: `Derived`'s `mro` is `[Derived, Base]` and `Base`'s
/// is `[Base]`, most derived first.
fn inheriting_module() -> HirModule {
    let mut hir = constructible_module("Base");
    hir.items[1] = func("Base.value", &[("self", inst("Base"))], Ty::Int);
    hir.class_defs[0]
        .1
        .methods
        .retain(|(name, _)| name != "area");
    declare_member(&mut hir, "Base", Member::Method("value"));
    let mut derived = constructible_class_def("Derived");
    derived.mro = vec!["Derived".to_string(), "Base".to_string()];
    hir.class_defs.push(("Derived".to_string(), derived));
    hir.items.push(init_func(
        "Derived",
        &[("w", Ty::Int), ("h", Ty::Int)],
        Ty::None,
    ));
    hir.items
        .push(func("Derived.twice", &[("self", inst("Derived"))], Ty::Int));
    declare_member(&mut hir, "Derived", Member::Method("twice"));
    hir
}

/// Each published class paired with the *compiled* names of the methods its
/// type object carries -- the compiled name rather than the host-visible one
/// because that is what says which definition won an override.
fn publication_rows(publications: &[ExtPublishedClass]) -> Vec<(&str, Vec<&str>)> {
    publications
        .iter()
        .map(|published| {
            (
                published.class.as_str(),
                published
                    .methods
                    .iter()
                    .map(|export| export.name.as_str())
                    .collect(),
            )
        })
        .collect()
}

fn publications_of(hir: &HirModule) -> Vec<ExtPublishedClass> {
    let exports = collect_exports(hir).expect("a carriable program");
    collect_class_publications(hir, &exports)
}

#[test]
fn an_inherited_instance_method_is_published_on_the_derived_class() {
    // The defect this fixes: `mod.Derived(21).twice()` worked while
    // `mod.Derived(21).value()` raised `AttributeError`, because the table
    // was built by declaring class. `Derived`'s own method comes first
    // because the walk is most-derived-first.
    assert_eq!(
        publication_rows(&publications_of(&inheriting_module())),
        vec![
            ("Base", vec!["Base.value"]),
            ("Derived", vec!["Derived.twice", "Base.value"]),
        ]
    );
}

#[test]
fn a_derived_override_shadows_its_base_at_the_first_mro_hit() {
    // First-MRO-hit-wins, which is the *opposite* direction from
    // `collect_exports`' own `(class, method)` dedup: that one keeps the
    // last binding, because a rebound name is what the class body means.
    // Here the derived definition is the one Python's attribute lookup
    // finds, so `Base.value` must not appear in `Derived`'s table at all.
    let mut hir = inheriting_module();
    hir.items
        .push(func("Derived.value", &[("self", inst("Derived"))], Ty::Int));
    declare_member(&mut hir, "Derived", Member::Method("value"));
    assert_eq!(
        publication_rows(&publications_of(&hir)),
        vec![
            ("Base", vec!["Base.value"]),
            ("Derived", vec!["Derived.twice", "Derived.value"]),
        ]
    );
}

#[test]
fn a_static_method_of_a_base_is_published_on_the_derived_class_too() {
    // One filter serves all three receiver kinds, so MRO-resolved
    // publication closes Part 1's identical gap for
    // `mod.Derived.static_from_base()`. Additive at the host surface: an
    // attribute appears, none disappears.
    let mut hir = inheriting_module();
    hir.items.push(func("Base.tag.static", &[], Ty::Int));
    declare_member(&mut hir, "Base", Member::Static("tag"));
    assert_eq!(
        publication_rows(&publications_of(&hir)),
        vec![
            ("Base", vec!["Base.value", "Base.tag.static"]),
            (
                "Derived",
                vec!["Derived.twice", "Base.value", "Base.tag.static"]
            ),
        ]
    );
}

#[test]
fn a_class_whose_only_exportable_members_are_inherited_is_published_and_constructible() {
    // The class list can no longer be read off the export set: `Derived`
    // declares no exportable member of its own, so the old loop gave it no
    // type object at all -- and therefore no `tp_init` either, by absence.
    let mut hir = inheriting_module();
    undeclare_method(&mut hir, "Derived", "twice");
    assert_eq!(
        publication_rows(&publications_of(&hir)),
        vec![
            ("Base", vec!["Base.value"]),
            ("Derived", vec!["Base.value"])
        ]
    );
    let ctors = collect_constructors(&hir, &publications_of(&hir));
    assert_eq!(
        ctors.iter().map(|c| c.class.as_str()).collect::<Vec<_>>(),
        vec!["Base", "Derived"]
    );
}

#[test]
fn an_unconstructible_base_still_exports_its_instance_method_for_a_constructible_subclass() {
    // Publication and constructibility are separate predicates. A `tuple`
    // parameter makes `Base` unconstructible (condition 4), but a `Derived`
    // instance still reaches `Base.value`'s compiled body, so the method is
    // exported and published -- while only `Derived` gets a `tp_init`.
    let mut hir = inheriting_module();
    hir.items[0] = init_func(
        "Base",
        &[("p", Ty::Tuple(Box::new(vec![Ty::Int, Ty::Int])))],
        Ty::None,
    );
    assert_eq!(
        publication_rows(&publications_of(&hir)),
        vec![
            ("Base", vec!["Base.value"]),
            ("Derived", vec!["Derived.twice", "Base.value"]),
        ]
    );
    let ctors = collect_constructors(&hir, &publications_of(&hir));
    assert_eq!(
        ctors.iter().map(|c| c.class.as_str()).collect::<Vec<_>>(),
        vec!["Derived"]
    );
}

/// [`inheriting_module`] with `Base` made unconstructible by a `tuple`
/// `__init__` and its only constructible subclass renamed to a **private**
/// spelling, so that subclass is never published.
///
/// `Derived.twice` goes with the rename: an inheriting class that exports
/// nothing of its own is the one shape that reaches publication without
/// having already passed `collect_exports`' own name filter, which is
/// exactly the shape whose witness has to be filtered here.
fn privately_derived_module() -> HirModule {
    let mut hir = inheriting_module();
    hir.items[0] = init_func(
        "Base",
        &[("p", Ty::Tuple(Box::new(vec![Ty::Int, Ty::Int])))],
        Ty::None,
    );
    hir.items[2] = init_func("_Priv", &[("w", Ty::Int), ("h", Ty::Int)], Ty::None);
    hir.items.truncate(3);
    hir.class_defs[1] = (
        "_Priv".to_string(),
        pycc_hir::HirClassDef {
            mro: vec!["_Priv".to_string(), "Base".to_string()],
            ..constructible_class_def("_Priv")
        },
    );
    hir
}

#[test]
fn a_privately_named_constructible_subclass_witnesses_nothing_for_its_base() {
    // The witness search has to hold the *publishability* condition
    // `collect_class_publications` applies, not constructibility alone.
    // `_Priv` is constructible but is published under no name, so the host
    // can never build an instance that reaches `Base.value`'s compiled body
    // -- exporting it would emit a `PyMethodDef` row with no obtainable
    // receiver. Drop `class_publishable` from the witness and `Base` starts
    // publishing `Base.value` again.
    let hir = privately_derived_module();
    assert!(
        collect_exports(&hir)
            .expect("a carriable program")
            .is_empty(),
        "an unpublished subclass makes its base's instance method unreachable"
    );
    assert!(publications_of(&hir).is_empty());
}

#[test]
fn an_unreachable_instance_method_with_an_uncarriable_signature_is_not_a_c0003() {
    // The same shape with a signature the boundary cannot carry. The
    // exclusion is representational and lands *before*
    // `unsupported_boundary_ty` is consulted, so a method nothing can ever
    // call must not fail the whole `--ext` build. Before the fix this exact
    // program exited 1 with `error[C0003]: ... parameter `xs: list`".
    let mut hir = privately_derived_module();
    hir.items[1] = func(
        "Base.value",
        &[("self", inst("Base")), ("xs", Ty::List(Box::new(Ty::Int)))],
        Ty::Int,
    );
    assert!(
        collect_exports(&hir)
            .expect("an unreachable method is excluded as representation, never a C0003")
            .is_empty()
    );
}

#[test]
fn a_published_abstract_class_is_refused_a_constructor_by_its_shape_alone() {
    // Publication is deliberately wider than constructibility: an abstract
    // class that exports a `@staticmethod` is published (Part 1's shape), so
    // it reaches `collect_constructors` even though nothing may ever
    // instantiate it. The refusal is `instance_shape_admissible`, not the
    // `__init__` conditions -- this fixture gives the class a perfectly
    // carriable `__init__` so only the shape half can answer.
    let mut hir = module_with_classes(
        vec![
            init_func("Grid", &[("w", Ty::Int)], Ty::None),
            func("Grid.scale.static", &[("n", Ty::Int)], Ty::Int),
        ],
        vec![("Grid".to_string(), constructible_class_def("Grid"))],
    );
    declare_member(&mut hir, "Grid", Member::Static("scale"));
    hir.class_defs[0].1.is_abstract = true;
    let publications = publications_of(&hir);
    assert_eq!(
        publications
            .iter()
            .map(|p| p.class.as_str())
            .collect::<Vec<_>>(),
        vec!["Grid"],
        "the staticmethod still publishes the type object"
    );
    assert!(
        collect_constructors(&hir, &publications).is_empty(),
        "an abstract class is never constructible, however carriable its `__init__` is"
    );
}

#[test]
fn an_abstract_base_exports_no_instance_method_however_constructible_its_subclass_is() {
    // The half of the old constructibility filter a constructible subclass
    // must *not* rescue: an `@abstractmethod`'s stub body returns nothing
    // while its `return_ty` says otherwise, so a wrapper over it would
    // return indeterminate storage. Only `Derived`'s own method is exported.
    let mut hir = inheriting_module();
    hir.class_defs[0].1.is_abstract = true;
    let exports = collect_exports(&hir).expect("excluded as representation, never a gap");
    assert_eq!(
        exports.iter().map(|e| e.name.as_str()).collect::<Vec<_>>(),
        vec!["Derived.twice"]
    );
}

#[test]
fn a_private_or_exception_inheriting_class_is_not_published() {
    // Both filters bite only here: every class in the export set already
    // passed them, so an inheriting class that exports nothing of its own
    // is the one shape that can reach publication without them.
    let mut private = inheriting_module();
    undeclare_method(&mut private, "Derived", "twice");
    private.class_defs[1].0 = "_Derived".to_string();
    private.class_defs[1].1.name = "_Derived".to_string();
    private.class_defs[1].1.mro = vec!["_Derived".to_string(), "Base".to_string()];
    assert_eq!(
        publication_rows(&publications_of(&private)),
        vec![("Base", vec!["Base.value"])]
    );

    let mut raising = inheriting_module();
    undeclare_method(&mut raising, "Derived", "twice");
    raising.class_defs[1].1.exception_type_tag = Some(FIRST_USER_EXCEPTION_TYPE_TAG);
    assert_eq!(
        publication_rows(&publications_of(&raising)),
        vec![("Base", vec!["Base.value"])]
    );
}

#[test]
fn a_class_absent_from_the_class_table_publishes_nothing() {
    // Neither predicate can answer without the class definition -- there is
    // no MRO to resolve and no tag to read -- so such a spelling exports no
    // instance method and publishes no type object. Unreachable from a
    // program `pycc check` accepts: `pycc_hir::class` records a definition
    // for every class it lowers a method of.
    let hir = module(vec![
        func("Grid.tag.static", &[], Ty::Int),
        func("Grid.area", &[("self", inst("Grid"))], Ty::Int),
    ]);
    let exports = collect_exports(&hir).expect("a carriable program");
    assert_eq!(
        exports.iter().map(|e| e.name.as_str()).collect::<Vec<_>>(),
        vec!["Grid.tag.static"]
    );
    assert!(collect_class_publications(&hir, &exports).is_empty());
}

#[test]
#[should_panic(expected = "lists `Ghost` in its method resolution order")]
fn a_ghost_class_in_the_mro_fails_fast_instead_of_under_allocating_an_instance() {
    // #1145 finding 2: the MIR counterpart (`pycc_mir::class`' `mro_attrs`)
    // panics on the same input, so skipping the entry here would allocate an
    // instance too small for the slots an inherited compiled method indexes.
    let mut hir = constructible_module("Grid");
    hir.class_defs[0].1.mro = vec!["Grid".to_string(), "Ghost".to_string()];
    let _ = class_constructible(&hir, "Grid");
}

// --- #1146: the walk resolves the namespace, not the export set ----------

/// [`inheriting_module`] with `Derived` shadowing `Base.value` with a
/// `@property` of the same name: the getter is an ordinary compiled
/// `Derived.value` item plus the `properties` entry that tells it apart.
fn property_shadowing_module() -> HirModule {
    let mut hir = inheriting_module();
    hir.items
        .push(func("Derived.value", &[("self", inst("Derived"))], Ty::Int));
    declare_member(&mut hir, "Derived", Member::Property("value"));
    hir
}

#[test]
fn a_derived_property_getter_shadows_its_base_method_and_publishes_nothing() {
    // The #1146 defect, at the unit seam. `collect_exports` has already
    // removed the getter from the export set, so a walk that stops at the
    // first *exportable* hit fell through to `Base.value` and published a
    // `method_descriptor` returning Base's answer where Python's own
    // attribute lookup gives the derived property. Resolving the namespace
    // first stops at `Derived`, which owns the name.
    assert_eq!(
        publication_rows(&publications_of(&property_shadowing_module())),
        vec![
            ("Base", vec!["Base.value"]),
            ("Derived", vec!["Derived.twice"]),
        ]
    );
}

#[test]
fn a_derived_instance_attribute_shadows_its_base_method_and_publishes_nothing() {
    // `__init__`'s `self.value = ...` binds `value` on the instance, and
    // Python resolves `obj.value` against the instance before the type, so
    // the derived slot wins over `Base.value` exactly as an override would.
    // `pycc_hir` accepts the collision -- verified end to end: the same
    // source built with `--ext` published a callable `Derived.value`
    // answering Base's 21 where CPython reads the slot's own value -- so
    // this walk is the only place it can be seen.
    let mut hir = inheriting_module();
    hir.class_defs[1]
        .1
        .attrs
        .push(("value".to_string(), Ty::Int));
    assert_eq!(
        publication_rows(&publications_of(&hir)),
        vec![
            ("Base", vec!["Base.value"]),
            ("Derived", vec!["Derived.twice"]),
        ]
    );
}

#[test]
fn a_sibling_base_s_class_attribute_shadows_a_further_base_s_method() {
    // A class attribute is an ordinary entry in the class object's
    // namespace, so an MRO entry binding `value` as a `ClassVar` answers
    // the name and the walk must stop there rather than reach `Base`'s
    // method behind it. `pycc_hir` accepts this shape: its class-attribute
    // collision check runs over a class's *own* newly declared attributes,
    // so `Derived`, which declares none, combines two independent bases
    // without complaint. Verified end to end: before the fix, the same
    // source built with `--ext` published a callable `mod.Derived(3).f()`
    // answering `3` where CPython raises `TypeError: 'int' object is not
    // callable`, because `d.f` is the sibling base's `2`.
    let mut hir = inheriting_module();
    let mut shadowing = class_def("Shadowing", None);
    shadowing.class_attrs = vec![(
        "value".to_string(),
        Ty::Int,
        pycc_hir::ClassAttrValue::Int(2),
    )];
    hir.class_defs.push(("Shadowing".to_string(), shadowing));
    hir.class_defs[1].1.mro = vec![
        "Derived".to_string(),
        "Shadowing".to_string(),
        "Base".to_string(),
    ];
    assert_eq!(
        publication_rows(&publications_of(&hir)),
        vec![
            ("Base", vec!["Base.value"]),
            ("Derived", vec!["Derived.twice"]),
        ]
    );
}

#[test]
fn a_class_binding_one_name_both_ways_publishes_the_slot_s_neighbours_only() {
    // The same class assigns `self.value` in `__init__` *and* declares
    // `def value`. `pycc_hir` retains both bindings, and a walk that
    // modelled the slot as one more namespace kind would find `Base` owning
    // the name either way and publish the method. Python does not: the
    // instance `__dict__` answers `obj.value` ahead of a non-data
    // descriptor, so the compiled method is unreachable. Verified end to
    // end: before the fix, that source built with `--ext` answered
    // `mod.C(1).value()` with `5` where CPython raises `TypeError: 'int'
    // object is not callable`. `Derived.twice` is the positive direction --
    // an unrelated name on a class whose MRO carries the slot still
    // publishes. `Base` keeps a type object with no method row: it is
    // constructible, and #1450 publishes a constructible class whatever its
    // resolved set holds.
    let mut hir = inheriting_module();
    hir.class_defs[0]
        .1
        .attrs
        .push(("value".to_string(), Ty::Int));
    assert_eq!(
        publication_rows(&publications_of(&hir)),
        vec![("Derived", vec!["Derived.twice"]), ("Base", vec![])]
    );
}

#[test]
fn a_base_s_instance_slot_shadows_a_derived_class_s_method() {
    // The shape a position-sensitive walk cannot answer: the slot is
    // declared by the *least* derived class and the method by the most
    // derived one, so the first MRO entry binding `twice` is `Derived`
    // itself. CPython still reads the instance slot, because its precedence
    // over a non-data descriptor does not depend on where in the MRO the
    // slot was assigned. Verified end to end: before the fix, a `Base`
    // assigning `self.value` with a `Derived(Base)` declaring `def value`
    // built with `--ext` and answered `mod.Derived(7).value()` with `5`
    // where CPython raises `TypeError`. `Base.value` is the positive
    // direction: unshadowed, it is still published on both classes.
    let mut hir = inheriting_module();
    hir.class_defs[0]
        .1
        .attrs
        .push(("twice".to_string(), Ty::Int));
    assert_eq!(
        publication_rows(&publications_of(&hir)),
        vec![
            ("Base", vec!["Base.value"]),
            ("Derived", vec!["Base.value"])
        ]
    );
}

#[test]
fn a_protocol_base_s_declared_method_shadows_a_further_base_s_export() {
    // A `Protocol`'s declaration-style `def f(self) -> int: ...` is a real
    // function object in the protocol's namespace, so CPython answers it and
    // not a later base's binding of the same name. Verified end to end:
    // `class Q(P, A)` with `P` declaring `f` and `A` exporting a
    // `@staticmethod f` built with `--ext` and answered `mod.Q.f()` with `7`,
    // where CPython raises `TypeError: P.f() missing 1 required positional
    // argument: 'self'`. The protocol binding is not itself exportable, so
    // the name is now published by no one, while `Base.value` -- the name no
    // protocol member binds -- is inherited unchanged.
    let mut hir = inheriting_module();
    hir.class_defs[1]
        .1
        .protocol_members
        .push(pycc_hir::ProtocolMember::Method {
            name: "value".to_string(),
            param_tys: Vec::new(),
            return_ty: Ty::Int,
        });
    assert_eq!(
        publication_rows(&publications_of(&hir)),
        vec![
            ("Base", vec!["Base.value"]),
            ("Derived", vec!["Derived.twice"])
        ]
    );
}

#[test]
fn a_protocol_base_s_annotation_only_attribute_shadows_nothing() {
    // The other half of `protocol_members`: `x: int` in a class body
    // declares a type and binds no namespace entry, in a `Protocol` exactly
    // as anywhere else, so it must not suppress a name a real base exports.
    let mut hir = inheriting_module();
    hir.class_defs[1]
        .1
        .protocol_members
        .push(pycc_hir::ProtocolMember::Attribute {
            name: "value".to_string(),
            ty: Ty::Int,
        });
    assert_eq!(
        publication_rows(&publications_of(&hir)),
        vec![
            ("Base", vec!["Base.value"]),
            ("Derived", vec!["Derived.twice", "Base.value"])
        ]
    );
}

#[test]
fn a_constructible_class_whose_whole_resolved_set_is_shadowed_away_publishes_no_method() {
    // The collapse case the per-name matrix above never reaches: every one
    // of `Derived`'s resolved names is shadowed by a non-exporting binding,
    // so its row carries no method at all. Before #1450 such a class got no
    // type object; it is constructible, so it is now published with no
    // exported method, as CPython's own module still answers
    // `mod.Derived(...)` -- with `value` read from the instance, not called.
    let mut hir = inheriting_module();
    hir.class_defs[1]
        .1
        .methods
        .retain(|(name, _)| name != "twice");
    hir.items
        .retain(|item| !matches!(item, HirItem::Function { name, .. } if name == "Derived.twice"));
    hir.class_defs[1]
        .1
        .attrs
        .push(("value".to_string(), Ty::Int));
    assert_eq!(
        publication_rows(&publications_of(&hir)),
        vec![("Base", vec!["Base.value"]), ("Derived", vec![])]
    );
}

#[test]
fn a_derived_method_shadowing_a_base_property_is_published_unchanged() {
    // The mirror direction, which must not regress: `Base` binds `value` as
    // a `@property` and publishes nothing for it, while `Derived`'s ordinary
    // method owns the name on `Derived` and is published there. A
    // non-regression guard rather than a discriminator: the base contributes
    // no export under the name, so nothing exists for a first-exportable-hit
    // walk to fall through to and this shape answered the same before #1146.
    // `Base` exports nothing but is constructible, so since #1450 it is
    // published with no exported method.
    let mut hir = inheriting_module();
    hir.class_defs[0]
        .1
        .methods
        .retain(|(name, _)| name != "value");
    declare_member(&mut hir, "Base", Member::Property("value"));
    hir.items
        .push(func("Derived.value", &[("self", inst("Derived"))], Ty::Int));
    declare_member(&mut hir, "Derived", Member::Method("value"));
    assert_eq!(
        publication_rows(&publications_of(&hir)),
        vec![
            ("Derived", vec!["Derived.twice", "Derived.value"]),
            ("Base", vec![])
        ]
    );
}

#[test]
fn a_derived_abstract_method_shadows_its_base_s_concrete_one() {
    // An `@abstractmethod` binds its name in `methods` exactly as an
    // ordinary method does, and exports nothing of its own
    // (`instance_shape_admissible` refuses an abstract class). `Derived`
    // therefore publishes no `value` at all -- publishing `Base.value` there
    // would answer a name whose derived binding is a stub.
    let mut hir = inheriting_module();
    hir.class_defs[1].1.is_abstract = true;
    hir.class_defs[1]
        .1
        .abstract_methods
        .push("value".to_string());
    declare_member(&mut hir, "Derived", Member::Method("value"));
    assert_eq!(
        publication_rows(&publications_of(&hir)),
        vec![("Base", vec!["Base.value"])]
    );
}

#[test]
fn a_derived_static_or_class_method_shadows_its_base_instance_method() {
    // A shadowing binding of a *different* receiver kind is still the
    // binding Python resolves, and it is published under its own kind: the
    // derived `@staticmethod` wins on `Derived` while `Base` keeps its
    // instance method. The first two arms are non-regression guards -- both
    // bindings are exports, so a first-exportable-hit walk reached the same
    // rows -- and the third discriminates: the shadowing binding of a
    // different kind is not itself an export.
    let mut statics = inheriting_module();
    statics
        .items
        .push(func("Derived.value.static", &[], Ty::Int));
    declare_member(&mut statics, "Derived", Member::Static("value"));
    assert_eq!(
        publication_rows(&publications_of(&statics)),
        vec![
            ("Base", vec!["Base.value"]),
            ("Derived", vec!["Derived.twice", "Derived.value.static"]),
        ]
    );

    // The same for a `@classmethod`, whose table is its own as well.
    let mut classmethods = inheriting_module();
    classmethods.items.push(func(
        "Derived.value.classmethod",
        &[("cls", inst("Derived"))],
        Ty::Int,
    ));
    classmethods.class_defs[1]
        .1
        .class_methods
        .push(("value".to_string(), "Derived.value.classmethod".to_string()));
    assert_eq!(
        publication_rows(&publications_of(&classmethods)),
        vec![
            ("Base", vec!["Base.value"]),
            (
                "Derived",
                vec!["Derived.twice", "Derived.value.classmethod"]
            ),
        ]
    );

    // The kind pair the export set alone cannot answer: `Base` exports `tag`
    // as a `@staticmethod` and `Derived` rebinds the name as a `@property`,
    // which exports nothing. Stopping at the first exportable hit publishes
    // `mod.Derived.tag()` as a callable the Python program does not have.
    let mut over_static = inheriting_module();
    over_static
        .items
        .push(func("Base.tag.static", &[], Ty::Int));
    declare_member(&mut over_static, "Base", Member::Static("tag"));
    over_static
        .items
        .push(func("Derived.tag", &[("self", inst("Derived"))], Ty::Int));
    declare_member(&mut over_static, "Derived", Member::Property("tag"));
    assert_eq!(
        publication_rows(&publications_of(&over_static)),
        vec![
            ("Base", vec!["Base.value", "Base.tag.static"]),
            ("Derived", vec!["Derived.twice", "Base.value"]),
        ]
    );
}

#[test]
fn a_derived_instance_method_shadows_its_base_static_method() {
    // And the reverse direction: an ordinary method shadowing an inherited
    // `@staticmethod`. `Base.tag.static` stays published on `Base` itself.
    // Another non-regression guard: both bindings are exports, so this shape
    // answered the same under the first-exportable-hit walk.
    let mut hir = inheriting_module();
    hir.items.push(func("Base.tag.static", &[], Ty::Int));
    declare_member(&mut hir, "Base", Member::Static("tag"));
    hir.items
        .push(func("Derived.tag", &[("self", inst("Derived"))], Ty::Int));
    declare_member(&mut hir, "Derived", Member::Method("tag"));
    assert_eq!(
        publication_rows(&publications_of(&hir)),
        vec![
            ("Base", vec!["Base.value", "Base.tag.static"]),
            // `value` is not shadowed, so `Derived` still inherits it.
            (
                "Derived",
                vec!["Derived.twice", "Derived.tag", "Base.value"]
            ),
        ]
    );
}

#[test]
fn a_base_method_every_witness_shadows_is_not_exported_and_never_a_c0003() {
    // The export side of the same rule. `Base` is unconstructible (a `tuple`
    // `__init__`), so its only witness is `Derived` -- which shadows `value`
    // with a `@property`. No host instance can ever reach `Base.value`'s
    // compiled body, so it is excluded as representation: a dead
    // `PyMethodDef` row on `Base`'s own type object with the carriable
    // signature, and a whole failed `--ext` build with an uncarriable one.
    let mut hir = property_shadowing_module();
    hir.items[0] = init_func(
        "Base",
        &[("p", Ty::Tuple(Box::new(vec![Ty::Int, Ty::Int])))],
        Ty::None,
    );
    let exports = collect_exports(&hir).expect("a carriable program");
    assert_eq!(
        exports.iter().map(|e| e.name.as_str()).collect::<Vec<_>>(),
        vec!["Derived.twice"]
    );

    hir.items[1] = func(
        "Base.value",
        &[("self", inst("Base")), ("xs", Ty::List(Box::new(Ty::Int)))],
        Ty::Int,
    );
    assert_eq!(
        collect_exports(&hir)
            .expect("a shadowed method is excluded as representation, never a C0003")
            .iter()
            .map(|e| e.name.as_str())
            .collect::<Vec<_>>(),
        vec!["Derived.twice"]
    );
}

/// Part 1 of #1142: `collect_exports` records which buffer parameters the
/// body stores into, and only those.
///
/// The three-parameter shape is the point. The flag is per parameter, so a
/// buffer the body only reads stays read-only -- acquiring it writable
/// would refuse the read-only exporters D-244's boundary has always
/// accepted -- and the `int` beside them is inert, since `PyBUF_WRITABLE`
/// has no meaning for a slot that acquires no buffer.
#[test]
fn an_exports_buffer_parameter_is_writable_exactly_when_its_body_stores_into_it() {
    let hir = module(vec![func_with_body(
        "mix",
        &[
            ("read", Ty::MemoryView),
            ("written", Ty::MemoryView),
            ("n", Ty::Int),
        ],
        Ty::None,
        vec![pycc_hir::HirStmt::While {
            test: pycc_hir::HirExpr::BoolLiteral(true),
            body: vec![element_store("written")],
        }],
    )]);
    let exports = collect_exports(&hir).expect("a carriable signature");
    assert_eq!(
        exports
            .iter()
            .map(|e| &e.param_writable)
            .collect::<Vec<_>>(),
        vec![&vec![false, true, false]]
    );
}

/// The flags are indexed against the *post-split* parameter list, so a
/// method whose receiver was dropped does not shift them by one.
///
/// An off-by-one here would acquire the wrong slot writable: `self` never
/// crosses the boundary, so a flag list still counting it would mark the
/// buffer after the written one, or run past the end.
#[test]
fn an_instance_methods_writability_flags_skip_the_dropped_receiver() {
    let class = "Grid";
    let mut hir = module_with_classes(
        vec![
            init_func(class, &[("w", Ty::Int)], Ty::None),
            func_with_body(
                &format!("{class}.fill"),
                &[
                    ("self", Ty::Instance(Box::new(class.to_string()))),
                    ("b", Ty::MemoryView),
                ],
                Ty::None,
                vec![element_store("b")],
            ),
        ],
        vec![(class.to_string(), constructible_class_def(class))],
    );
    declare_member(&mut hir, class, Member::Method("fill"));
    let exports = collect_exports(&hir).expect("a carriable signature");
    let fill = exports
        .iter()
        .find(|e| e.name == "Grid.fill")
        .expect("the method is exported");
    assert_eq!(fill.params, vec![Ty::MemoryView]);
    assert_eq!(fill.param_writable, vec![true]);
}

/// The same walk on a constructor, which `collect_exports` never sees:
/// `__init__` is refused as an export outright, so `ctor_descriptor` has to
/// look its body up for itself. Left unplumbed, `Py_tp_init` would acquire
/// read-only storage this body writes through.
#[test]
fn a_constructors_buffer_parameter_carries_the_same_writability_flag() {
    let class = "Grid";
    let mut hir = module_with_classes(
        vec![
            func_with_body(
                &format!("{class}.__init__"),
                &[
                    ("self", Ty::Instance(Box::new(class.to_string()))),
                    ("b", Ty::MemoryView),
                    ("read", Ty::MemoryView),
                ],
                Ty::None,
                vec![element_store("b")],
            ),
            func(
                &format!("{class}.area"),
                &[("self", Ty::Instance(Box::new(class.to_string())))],
                Ty::Int,
            ),
        ],
        vec![(class.to_string(), constructible_class_def(class))],
    );
    declare_member(&mut hir, class, Member::Method("area"));
    let exports = collect_exports(&hir).expect("a carriable signature");
    let ctors = collect_constructors(&hir, &collect_class_publications(&hir, &exports));
    assert_eq!(
        ctors.iter().map(|c| &c.param_writable).collect::<Vec<_>>(),
        vec![&vec![true, false]]
    );
}

/// `Grid`'s descriptor table in `hir`, as the `--ext` build collects it
/// (#1448).
fn grid_getsets(hir: &HirModule) -> Vec<crate::ext_build::ExtGetset> {
    let tables = collect_carrier_getsets(hir, &collect_carrier_classes(hir));
    assert_eq!(tables.len(), 1, "only `Grid` has a table");
    assert_eq!(tables[0].class, "Grid");
    tables
        .into_iter()
        .next()
        .map(|table| table.getsets)
        .unwrap_or_default()
}

/// The two `int` slot descriptors [`constructible_module`]'s `Grid` exposes
/// (#1442).
fn grid_slot_getsets() -> Vec<crate::ext_build::ExtGetset> {
    ["w", "h"]
        .iter()
        .enumerate()
        .map(|(index, name)| crate::ext_build::ExtGetset::Slot {
            name: (*name).to_string(),
            index,
            ty: Ty::Int,
        })
        .collect()
}
