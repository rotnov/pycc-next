//! Instance carriers, buffer-returning exports, slice egress and object signatures.

use super::*;

#[test]
fn the_shim_defines_the_carrier_and_the_shared_dealloc_above_the_generated_include() {
    // The generated `.inc` names `PyccExtInstance` and
    // `pycc_ext_instance_dealloc` without declaring either, so both must be
    // defined earlier in the translation unit. C has no forward reference
    // for a struct size or a function address used as an initializer.
    let shim = shim_c();
    let carrier = shim
        .find("typedef struct {\n    PyObject_HEAD\n    void *inst;\n} PyccExtInstance;\n")
        .expect("the carrier typedef");
    let dealloc = shim
        .find("static void pycc_ext_instance_dealloc(PyObject *self)\n")
        .expect("the shared dealloc");
    let include = shim
        .find("#include \"pycc_ext_exports.inc\"")
        .expect("the generated include");
    assert!(
        carrier < include && dealloc < include,
        "{carrier} {dealloc} {include}"
    );
    // The heap-type reference every instance holds is discharged here.
    // Dropping the `Py_DECREF(tp)` leaks the type object in silence.
    assert!(
        shim.contains(
            "    freefunc tp_free = (freefunc)PyType_GetSlot(tp, Py_tp_free);\n    \
             tp_free(self);\n    Py_DECREF(tp);\n"
        ),
        "{shim}"
    );
    assert!(
        shim.contains(
            "extern void *pycc_rt_instance_new(long long slot_count, const char *layout, \
             size_t layout_len);"
        ),
        "{shim}"
    );
}

#[test]
fn a_derived_class_table_carries_its_base_s_rows_and_its_own_tp_init() {
    // The rendered half of #1145's inheritance fix, end to end from a
    // `HirModule`: `Derived`'s table names `Base.value`'s wrapper -- the one
    // `generate_exports_inc` emitted once, from the export list -- and
    // `Derived` gets its own type object and `tp_init` although it declares
    // no exportable member of its own.
    let mut derived = constructible_class_def("Derived");
    derived.mro = vec!["Derived".to_string(), "Base".to_string()];
    // `Base` binds `value` in its own method table, exactly as
    // `pycc_hir::class` lowers it: `collect_class_publications` resolves the
    // name against that table before it consults the export set.
    let mut base = constructible_class_def("Base");
    base.methods
        .push(("value".to_string(), "Base.value".to_string()));
    let hir = module_with_classes(
        vec![
            init_func("Base", &[("w", Ty::Int)], Ty::None),
            func(
                "Base.value",
                &[("self", Ty::Instance(Box::new("Base".to_string())))],
                Ty::Int,
            ),
            init_func("Derived", &[("w", Ty::Int)], Ty::None),
        ],
        vec![("Base".to_string(), base), ("Derived".to_string(), derived)],
    );
    let exports = collect_exports(&hir).expect("a carriable program");
    let publications = collect_class_publications(&hir, &exports);
    let ctors = collect_constructors(&hir, &publications);
    let inc = generate_exports_inc("m", &exports, &[], &publications, &ctors, &[]);
    // One wrapper, two rows: the inherited row points at the base's own
    // compiled symbol, so nothing is generated twice.
    assert_eq!(
        inc.matches("static PyObject *pycc_ext_wrap_0m4_Base5_value")
            .count(),
        1,
        "{inc}"
    );
    let row = "    {\"value\", (PyCFunction)(void (*)(void))pycc_ext_wrap_0m4_Base5_value, \
               METH_FASTCALL, NULL},\n";
    assert!(
        inc.contains(&format!(
            "static PyMethodDef pycc_ext_type_methods_Derived[] = {{\n{row}"
        )),
        "{inc}"
    );
    assert!(
        inc.contains(&format!(
            "static PyMethodDef pycc_ext_type_methods_Base[] = {{\n{row}"
        )),
        "{inc}"
    );
    assert!(inc.contains("pycc_ext_tp_init_Derived"), "{inc}");
    // Both classes reach the publication table, in publication order.
    let base_at = inc
        .find("if (strcmp(name, \"Base\") == 0)")
        .expect("Base publishable");
    let derived_at = inc
        .find("if (strcmp(name, \"Derived\") == 0)")
        .expect("Derived publishable");
    assert!(base_at < derived_at, "{inc}");
    assert!(
        !inc.contains("PyModule_AddObjectRef(module, \"Base\""),
        "{inc}"
    );
}

// --- Part 2b of #1142 (#1164): the buffer return position -------------------

#[test]
fn a_buffer_returning_export_declares_the_carrier_pointer_and_packs_it() {
    // The result slot is declared with `BUFFER_VIEW_RETURN_C_TYPE` rather
    // than routed through `BoundaryCarrier::into_scalar`, whose own
    // admissible set stays scalar-only so that `tuple[memoryview]` keeps
    // being refused. The declaration and the packer call are asserted
    // together: a declaration without the packer would not compile, and a
    // packer call over a wrongly declared slot would compile and be wrong.
    let inc = memoryview_inc("make", 0, Ty::MemoryView);
    assert!(inc.contains("    PyccExtBufferView * result;\n"), "{inc}");
    assert!(
        inc.contains("    return pycc_ext_pack_memoryview(result);\n}\n\n"),
        "{inc}"
    );
    // The compiled callee's own C signature returns the carrier pointer, so
    // the cast the wrapper calls through has to say so too.
    assert!(
        inc.contains("((PyccExtBufferView * (*)(void))fnptr_make)()"),
        "{inc}"
    );
    // ...and nothing reaches the scalar packers by accident.
    assert!(!inc.contains("pycc_ext_pack_int(result)"), "{inc}");
    assert!(!inc.contains("Py_NewRef(Py_None)"), "{inc}");
}

#[test]
fn a_buffer_returning_export_releases_its_parameters_before_packing() {
    // A `memoryview` **parameter** is the host's, borrowed for exactly one
    // call, so it is released on the way out -- and the release has to
    // precede the pack, which is a `return`. Asserted as adjacency rather
    // than as presence, because a release emitted after the pack is dead
    // code that a presence-only assertion accepts.
    //
    // Part 1 of #1175 put one thing between them, and only one: the
    // caller-owned hand-back, which is itself a `return` and so owes the
    // same precedence. What it did *not* move is the acquire -- see
    // `a_caller_owned_buffer_return_acquires_before_it_releases`, which pins
    // the half of this ordering that a `contains` on the tail cannot see.
    let inc = memoryview_inc("make", 1, Ty::MemoryView);
    assert!(
        inc.contains(
            "    PyBuffer_Release(&b0);\n    if (caller_owned) {\n        return borrowed;\n    \
             }\n    return pycc_ext_pack_memoryview(result);\n}\n\n"
        ),
        "{inc}"
    );
    // Exactly two releases: the two exits reachable with the buffer held.
    assert_eq!(inc.matches("PyBuffer_Release(&b0);").count(), 2, "{inc}");
}

/// Part 1 of #1175's substantive ordering property, and the renderer-level
/// half of the PEP 688 host-side test in
/// `tests/issue_1175_caller_owned_buffer_return.rs`.
///
/// The acquire must be emitted **before** the release, not merely somewhere
/// before the pack. `pycc_ext_unpack_memoryview` admits any
/// `PyObject_CheckBuffer` object, so the argument can be a PEP 688
/// Python-level exporter whose `__release_buffer__` runs arbitrary code;
/// with the release first, the host object is at zero outstanding exports at
/// that instant and may legally reallocate, and the view the wrapper then
/// acquires can describe storage the compiled body never touched. Asserted
/// as one contiguous block, because two `contains` on the two halves pass
/// under either order.
#[test]
fn a_caller_owned_buffer_return_acquires_before_it_releases() {
    let inc = memoryview_inc("make", 1, Ty::MemoryView);
    assert!(
        inc.contains(
            "    if (result == &a0) {\n        caller_owned = 1;\n        borrowed = \
             pycc_ext_pack_memoryview_borrowed(args[0], &b0, \"make\", 0, 0);\n    }\n    \
             PyBuffer_Release(&b0);\n"
        ),
        "{inc}"
    );
    // The two locals, and the flag rather than a `borrowed != NULL` test:
    // `PyMemoryView_FromObject` answers NULL with the exception set, so
    // `borrowed == NULL` cannot discriminate "no parameter matched" from
    // "the match failed", and only the second of those must return NULL.
    assert!(
        inc.contains("    int caller_owned = 0;\n    PyObject *borrowed = NULL;\n"),
        "{inc}"
    );
    assert!(!inc.contains("borrowed != NULL"), "{inc}");
    // The acquire is emitted once, on the success path only. The pending-
    // exception bail above it returns NULL without packing anything.
    assert_eq!(
        inc.matches("pycc_ext_pack_memoryview_borrowed").count(),
        1,
        "{inc}"
    );
}

/// The identity chain names each parameter exactly once, so a body that
/// returns the *second* of two buffer parameters -- or either of them on
/// different paths -- hands back the right one.
///
/// `&a0` and `&a1` are distinct locals in the wrapper's own live stack
/// frame even when the same host object was passed twice, which is what
/// makes the test exact rather than heuristic; artifact-owned storage is a
/// heap block from `pycc_rt_buffer_f64_alloc` and cannot alias either.
#[test]
fn a_caller_owned_buffer_return_tests_every_buffer_parameter_in_turn() {
    let inc = memoryview_inc("pick", 2, Ty::MemoryView);
    assert!(
        inc.contains(concat!(
            "    if (result == &a0) {\n        caller_owned = 1;\n",
            "        borrowed = pycc_ext_pack_memoryview_borrowed(args[0], &b0, \"pick\", 0, 0);\n",
            "    } else if (result == &a1) {\n        caller_owned = 1;\n",
            "        borrowed = pycc_ext_pack_memoryview_borrowed(args[1], &b1, \"pick\", 1, 0);\n    }\n",
            "    PyBuffer_Release(&b0);\n    PyBuffer_Release(&b1);\n",
        )),
        "{inc}"
    );
}

/// A `memoryview` parameter on an export that does **not** return a buffer
/// stays byte-identical to what it was: no flag, no acquire, no hand-back.
///
/// The conjunction is "returns a buffer **and** takes one". Guarding on the
/// parameter alone would emit dead code into every `-> float` reduction over
/// a buffer, which is the common shape.
#[test]
fn a_buffer_parameter_on_a_scalar_returning_export_emits_no_caller_owned_test() {
    let inc = memoryview_inc("total", 1, Ty::Float);
    assert!(!inc.contains("caller_owned"), "{inc}");
    assert!(!inc.contains("borrowed"), "{inc}");
    assert!(
        inc.contains("    PyBuffer_Release(&b0);\n    return pycc_ext_pack_float(result);\n"),
        "{inc}"
    );
}

/// The other half of the byte-identity guard: a buffer-returning export with
/// **no** buffer parameter has no `&a{i}` for `result` to equal, so the
/// artifact-owned pack is reached with nothing in front of it, exactly as
/// before Part 1 of #1175.
#[test]
fn a_buffer_returning_export_with_no_buffer_parameter_packs_as_it_always_did() {
    let inc = memoryview_inc("make", 0, Ty::MemoryView);
    assert!(!inc.contains("caller_owned"), "{inc}");
    assert!(!inc.contains("pycc_ext_pack_memoryview_borrowed"), "{inc}");
    assert!(
        inc.contains(concat!(
            "    if (pycc_rt_ext_pending_type() >= 0) {\n",
            "        pycc_ext_raise_pending();\n",
            "        return NULL;\n    }\n",
            "    return pycc_ext_pack_memoryview(result);\n",
        )),
        "{inc}"
    );
}

/// A public **method** reaches the same emission, because `wrapper_for` has
/// exactly one call site (`src/ext_build.rs`) and handles all three
/// receivers; `method_types.rs` only tables the resulting wrapper names.
///
/// The index worth pinning is `args[0]`: an instance receiver is pushed into
/// the call arguments separately and never occupies a slot, so the first
/// *declared* parameter is `args[0]` here exactly as it is for a
/// module-level function. An off-by-one would hand the host a view over
/// whichever object happened to follow.
#[test]
fn a_caller_owned_buffer_returning_method_uses_the_declared_parameter_index() {
    let inc = inc_no_classes(
        "m",
        &[ExtExport {
            defaults: Vec::new(),
            name: "Grid.row".to_string(),
            class: Some("Grid".to_string()),
            method: Some("row".to_string()),
            returns_buffer_slice: false,
            receiver: ExtReceiver::SelfInstance,
            params: vec![Ty::MemoryView],
            param_writable: vec![false],
            return_ty: Ty::MemoryView,
            keyword_names: None,
        }],
    );
    assert!(
        inc.contains(
            "    if (result == &a0) {\n        caller_owned = 1;\n        borrowed = \
             pycc_ext_pack_memoryview_borrowed(args[0], &b0, \"Grid.row\", 0, 0);\n    \
             }\n    PyBuffer_Release(&b0);\n"
        ),
        "{inc}"
    );
}

#[test]
fn a_buffer_returning_method_packs_through_the_same_arm() {
    // The method boundary #1131 added dispatches on the same `return_ty`,
    // so the egress arm has to answer there too rather than only for a
    // module-level function. This was a speculative pin when it was written
    // -- `src/memoryview_mode.rs` still refused a buffer-returning *method*
    // at the source level, and the arm existed against the day that
    // narrowing was lifted. #1174 lifted it, so this now covers a live
    // path: `tests/issue_1174_method_buffer_return.rs` builds exactly this
    // shape end to end, and this unit test is the renderer-level half of
    // it -- the alternative is a `BoundaryCarrier::into_scalar` panic.
    let inc = inc_no_classes(
        "m",
        &[ExtExport {
            defaults: Vec::new(),
            name: "Grid.make".to_string(),
            class: Some("Grid".to_string()),
            method: Some("make".to_string()),
            returns_buffer_slice: false,
            receiver: ExtReceiver::None,
            params: vec![],
            param_writable: vec![],
            return_ty: Ty::MemoryView,
            keyword_names: None,
        }],
    );
    assert!(
        inc.contains("    return pycc_ext_pack_memoryview(result);\n"),
        "{inc}"
    );
}

// ---------------------------------------------------------------------------
// Part 2 of #1175 (#1179): the buffer sub-range egress's generated C.
// ---------------------------------------------------------------------------

/// [`memoryview_inc`]'s slice-capable twin: the same export, with the
/// out-slot fact set.
fn memoryview_slice_inc(name: &str, count: usize) -> String {
    inc_no_classes(
        "m",
        &[ExtExport {
            defaults: Vec::new(),
            name: name.to_string(),
            class: None,
            method: None,
            returns_buffer_slice: true,
            receiver: ExtReceiver::None,
            params: vec![Ty::MemoryView; count],
            param_writable: vec![false; count],
            return_ty: Ty::MemoryView,
            keyword_names: None,
        }],
    )
}

/// D4's byte-identity promise, asserted rather than argued: an export that
/// does not carry a sub-range egress generates exactly the C it generated
/// before #1179.
///
/// This is why the three bound locals sit on their own declaration path
/// beside Part 1's `caller_owned`/`borrowed` instead of joining `out_slots`,
/// and why the host-side derivation is a second packer entry point instead
/// of three more parameters on the existing one. Every other test in this
/// file that pins a `memoryview` wrapper's exact text is the rest of the
/// guard; this one states the property directly.
#[test]
fn an_export_without_a_slice_egress_generates_unchanged_c() {
    for (name, count, return_ty) in [
        ("make", 1, Ty::MemoryView),
        ("pick", 2, Ty::MemoryView),
        ("make", 0, Ty::MemoryView),
        ("total", 1, Ty::Float),
    ] {
        let inc = memoryview_inc(name, count, return_ty);
        assert!(!inc.contains("slice_present"), "{inc}");
        assert!(!inc.contains("slice_start"), "{inc}");
        assert!(!inc.contains("slice_stop"), "{inc}");
        assert!(
            !inc.contains("pycc_ext_pack_memoryview_borrowed_slice"),
            "{inc}"
        );
        assert!(!inc.contains("long long *"), "{inc}");
        assert!(!inc.contains(pycc_codegen::EXT_THUNK_PREFIX), "{inc}");
    }
}

/// The three out-slot locals, all initialized. `slice_present` must start at
/// `0` because no pair of `long long` bounds can serve as an in-band
/// whole-view sentinel -- `b[0:-1]` is a legal slice and `LLONG_MIN`/
/// `LLONG_MAX` are legal bounds that clamp correctly -- and the two bound
/// locals must start initialized because a call that raises leaves them
/// untouched and an indeterminate read is undefined behaviour even where the
/// value is discarded.
#[test]
fn a_slice_egress_declares_three_initialized_out_slot_locals() {
    let inc = memoryview_slice_inc("head", 1);
    assert!(
        inc.contains(
            "    long long slice_present = 0;\n    long long slice_start = 0;\n    \
             long long slice_stop = 0;\n"
        ),
        "{inc}"
    );
}

/// The declaration, the cast and the call all carry the three trailing
/// pointers. Omitting them from any one of the three would be an ABI
/// mismatch neither compiler can see across the object boundary.
#[test]
fn a_slice_egress_extends_the_prototype_the_cast_and_the_call() {
    let inc = memoryview_slice_inc("head", 1);
    assert!(
        inc.contains(&format!(
            "{BUFFER_VIEW_C_TYPE} *, long long *, long long *, long long *"
        )),
        "{inc}"
    );
    assert!(
        inc.contains("&slice_present, &slice_start, &slice_stop"),
        "{inc}"
    );
    // The compiled body still returns the whole view's pointer, so `result`
    // is still bound and the identity chain still discriminates on it.
    assert!(inc.contains("result = "), "{inc}");
    assert!(inc.contains("if (result == &a0)"), "{inc}");
    // ...and it goes through the export thunk, because pycc's own calling
    // convention is not the platform C struct ABI.
    assert!(inc.contains(pycc_codegen::EXT_THUNK_PREFIX), "{inc}");
}

/// The host-side derivation: Part 1's whole-window PEP 688 check still runs
/// first, and the sub-range packer is selected at run time by the flag the
/// compiled body wrote -- so an export that also contains a bare `return b`
/// takes Part 1's path on that branch unchanged.
#[test]
fn a_slice_egress_selects_the_sub_range_packer_on_the_flag() {
    let inc = memoryview_slice_inc("head", 1);
    assert!(
        inc.contains(concat!(
            "    if (result == &a0) {\n        caller_owned = 1;\n",
            "        borrowed = slice_present\n",
            "            ? pycc_ext_pack_memoryview_borrowed_slice(args[0], &b0, \"head\", 0, 0, ",
            "slice_start, slice_stop)\n",
            "            : pycc_ext_pack_memoryview_borrowed(args[0], &b0, \"head\", 0, 0);\n",
            "    }\n    PyBuffer_Release(&b0);\n",
        )),
        "{inc}"
    );
}

/// Two buffer parameters: each arm of the identity chain gets its own
/// branched packer call with its own argument index and its own `&b{i}`.
#[test]
fn a_slice_egress_branches_every_parameter_of_the_identity_chain() {
    let inc = memoryview_slice_inc("pick", 2);
    for index in 0..2 {
        assert!(
            inc.contains(&format!(
                "pycc_ext_pack_memoryview_borrowed_slice(args[{index}], &b{index}, \
                 \"pick\", {index}, 0, slice_start, slice_stop)"
            )),
            "{inc}"
        );
    }
    assert_eq!(
        inc.matches("pycc_ext_pack_memoryview_borrowed_slice")
            .count(),
        2,
        "{inc}"
    );
}

#[test]
fn a_source_level_default_does_not_reach_the_generated_wrapper() {
    // Part 2 of #884 (#1189) fills a default while the *calling module* is
    // lowered, so HIR carries no trace of it and the host boundary is
    // unchanged: the wrapper still demands every declared argument. D-244
    // rule 7 is therefore untouched, and #1194 tracks widening it.
    let dir = pycc_scratch::ScratchDir::new("ext_default_params").expect("scratch");
    let src = dir.join("m.py");
    std::fs::write(
        &src,
        "def add(a: int, b: int = 2) -> int:\n    return a + b\n",
    )
    .expect("write source");
    let module = crate::frontend::resolve_frontend(&src, Some("m"))
        .unwrap_or_else(|_| panic!("the fixture must type-check"));
    let exports = collect_exports(&module).expect("a carriable program");
    assert_eq!(exports.len(), 1, "{exports:?}");
    assert_eq!(exports[0].params, vec![Ty::Int, Ty::Int], "{exports:?}");
    let inc = generate_exports_inc("m", &exports, &[], &flat_publications(&exports), &[], &[]);
    assert!(
        inc.contains("add() takes exactly 2 arguments (%zd given)"),
        "{inc}"
    );
    assert!(
        inc.contains("pycc_ext_unpack_int(args[1], \"add\", 1, &a1)"),
        "{inc}"
    );
}

/// D-258 rule 5 (#1397): an `Any`/`object`/container-of-objects signature
/// lowered from real source by the `--ext` frontend crosses as the
/// `PyObject *` itself -- one `void *` slot, unpacked by
/// `pycc_ext_unpack_object` and packed by `pycc_ext_pack_object`: the
/// unpacked argument is a new reference the parameter slot owns, and the
/// packer hands the host the reference the compiled return owns. Not
/// `#[ignore]`d, so the lines reached only through the hosted tests in
/// `tests/issue_1397_ext_any_object.rs` are also executed here.
#[test]
fn an_object_signature_crosses_the_boundary_as_the_pyobject_itself() {
    let dir = pycc_scratch::ScratchDir::new("ext_any_object").expect("scratch");
    let src = dir.join("m.py");
    std::fs::write(
        &src,
        "from typing import Any, Dict\n\
         def ident(x: Any, d: Dict[str, Any]) -> Any:\n    return x\n\
         def keep(xs: list) -> object:\n    return xs\n",
    )
    .expect("write source");
    let module = crate::frontend::resolve_frontend_with(
        &src,
        Some("m"),
        crate::modules::RelativeImports::Project,
    )
    .unwrap_or_else(|_| panic!("the fixture must type-check in an ext build"));
    let exports = collect_exports(&module).expect("an object signature is carriable");
    assert_eq!(exports.len(), 2, "{exports:?}");
    assert_eq!(exports[0].params, vec![Ty::Object, Ty::Object]);
    assert_eq!(exports[0].return_ty, Ty::Object);
    let inc = generate_exports_inc("m", &exports, &[], &flat_publications(&exports), &[], &[]);
    for needle in [
        "pycc_ext_unpack_object(args[0], \"ident\", 0, &a0)",
        "pycc_ext_unpack_object(args[1], \"ident\", 1, &a1)",
        "pycc_ext_unpack_object(args[0], \"keep\", 0, &a0)",
        "Py_DECREF((PyObject *)a0);",
        "return pycc_ext_pack_object(result);",
    ] {
        assert!(inc.contains(needle), "{needle}\n{inc}");
    }
    let shim = shim_c();
    for (signature, body_needle) in [
        (
            "static int pycc_ext_unpack_object(PyObject *obj",
            "*out = Py_NewRef(obj);",
        ),
        (
            "static PyObject *pycc_ext_pack_object(void *result)",
            "return (PyObject *)result;",
        ),
    ] {
        let body = &shim[shim.find(signature).expect(signature)..];
        let body = &body[..body.find("\n}\n").expect("the helper's end")];
        assert!(body.contains(body_needle), "{body}");
    }
}

/// The same source through the `native` test frontend keeps `T0002`: the
/// ext flag is set only by `resolve_frontend_with`.
#[test]
fn the_native_frontend_keeps_t0002_for_the_same_source() {
    let dir = pycc_scratch::ScratchDir::new("ext_any_native").expect("scratch");
    let src = dir.join("m.py");
    std::fs::write(
        &src,
        "from typing import Any\ndef ident(x: Any) -> Any:\n    return x\n",
    )
    .expect("write source");
    // No panicking `else` arm: every line runs when the test passes, so the
    // diff-coverage gate sees no failure-only line.
    let mut first_code = None;
    if let Err(crate::frontend::FrontendFailure::Compile { files }) =
        crate::frontend::resolve_frontend(&src, Some("m"))
    {
        first_code = Some(files[0].diagnostics[0].code.to_string());
    }
    assert_eq!(first_code.as_deref(), Some("T0002"));
}
