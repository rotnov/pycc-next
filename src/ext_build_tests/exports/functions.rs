//! Which module-level functions export, and the capability gaps of the rest.

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
             in this module (not an enum or an exception class), a `tuple` of \
             `int`/`float`/`bool`, or a `tuple[str, ...]`, and a return type must be one \
             of those, an immutable tuple of them, or `None`"
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
fn a_nested_tuple_return_crosses_as_one_packed_object_rather_than_a_flattening() {
    // The arm that would be silently wrong if it were flattened: a nested
    // tuple has no single scalar slot, and flattening it would make
    // `tuple[tuple[int, int], int]` and `tuple[int, int, int]` indis-
    // tinguishable at the seam. Since v0.5 such a tuple is a boxed tuple
    // object, returned as one pointer and packed recursively into a CPython
    // tuple of the same shape (`pycc_ext_pack_tuple_object`), never through
    // the scalar thunk's flattened slots.
    let hir = module(vec![func(
        "nest",
        &[("x", Ty::Int)],
        Ty::Tuple(Box::new(vec![Ty::Tuple(Box::new(vec![Ty::Int])), Ty::Int])),
    )]);
    let exports = collect_exports(&hir).expect("an immutable nested tuple is carriable");
    assert_eq!(exports.len(), 1);
    assert_eq!(
        crate::ext_build::carrier::return_c_type(&exports[0].return_ty),
        Some("void *")
    );
}

#[test]
fn a_tuple_holding_a_variadic_tuple_returns_one_packed_object() {
    // `-> tuple[int, tuple[str, ...]]` (tomli's `parse_key`): the result is
    // a boxed tuple object. Treating the variadic element as a flattened
    // out-slot (it has a scalar carrier of its own as a parameter) made the
    // wrapper read the tuple object's words as the out-slots and crash.
    let ty = Ty::Tuple(Box::new(vec![Ty::Int, Ty::VarTuple(Box::new(Ty::Str))]));
    assert!(!matches!(
        crate::ext_build::carrier::boundary_carrier(&ty),
        Some(crate::ext_build::carrier::BoundaryCarrier::Tuple(_))
    ));
    assert_eq!(crate::ext_build::carrier::return_c_type(&ty), Some("void *"));
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
