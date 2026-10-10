//! memoryview parameters: buffer acquisition and release.

use super::*;

/// Part 1 of #1142's writability bit reaches the generated request, and
/// reaches only the parameter it belongs to.
///
/// The two-parameter case is the one worth pinning: a flag applied to the
/// slot vector as a whole rather than per slot would either acquire
/// argument 0 writable -- refusing read-only exporters the body never
/// writes -- or acquire argument 1 read-only, which is the memory-safety
/// direction. The `0`/`1` here is the fourth argument of
/// `pycc_ext_unpack_memoryview`, the only consumer of the bit.
#[test]
fn only_a_buffer_parameter_its_body_stores_into_is_acquired_writable() {
    let inc = inc_no_classes(
        "m",
        &[ExtExport {
            defaults: Vec::new(),
            name: "mix".to_string(),
            class: None,
            method: None,
            returns_buffer_slice: false,
            receiver: ExtReceiver::None,
            params: vec![Ty::MemoryView, Ty::MemoryView],
            param_writable: vec![false, true],
            return_ty: Ty::None,
            keyword_names: None,
        }],
    );
    assert!(
        inc.contains("pycc_ext_unpack_memoryview(args[0], \"mix\", 0, 0, &b0)"),
        "{inc}"
    );
    assert!(
        inc.contains("pycc_ext_unpack_memoryview(args[1], \"mix\", 1, 1, &b1)"),
        "{inc}"
    );
}

/// The same bit on a constructor's slot vector, which `tp_init_c` builds
/// separately: D-244's admissibility matrix is one matrix, so a
/// `memoryview` `__init__` parameter its body stores into must be acquired
/// exactly as an export's would be. Left unplumbed, this path would write
/// through storage acquired read-only.
#[test]
fn a_constructor_buffer_parameter_its_body_stores_into_is_acquired_writable() {
    let inc = generate_exports_inc(
        "m",
        &[instance_export("Grid", "area", vec![], Ty::Int)],
        &[],
        &flat_publications(&[instance_export("Grid", "area", vec![], Ty::Int)]),
        &[ExtCtor {
            defaults: Vec::new(),
            class: "Grid".to_string(),
            name: "Grid.__init__".to_string(),
            params: vec![Ty::MemoryView],
            param_writable: vec![true],
            slot_names: vec!["w".to_string()],
            slot_kinds: "i".to_string(),
            keyword_names: None,
        }],
        &[],
    );
    assert!(
        inc.contains(
            "pycc_ext_unpack_memoryview(PyTuple_GetItem(args, 0), \"Grid.__init__\", 0, 1, &b0)"
        ),
        "{inc}"
    );
}

#[test]
fn a_memoryview_parameter_acquires_a_buffer_and_carries_only_a_copied_pair() {
    let inc = memoryview_inc("total", 1, Ty::Float);
    // The wrapper owns both locals for the whole call.
    assert!(inc.contains("    Py_buffer b0;\n"), "{inc}");
    assert!(inc.contains("    PyccExtBufferView a0;\n"), "{inc}");
    // One refusal arm, ahead of the call, exactly like every other slot.
    assert!(
        inc.contains(
            "    if (pycc_ext_unpack_memoryview(args[0], \"total\", 0, 0, &b0) != 0) {\n        \
             return NULL;\n    }\n"
        ),
        "{inc}"
    );
    // The pair is `buf.buf` plus a *copy* of `shape[0]`. Carrying
    // `b0.shape` itself would be a use-after-free the moment the wrapper
    // released the buffer, and nothing else in the tree would notice.
    assert!(inc.contains("    a0.ptr = b0.buf;\n"), "{inc}");
    assert!(
        inc.contains("    a0.len = (long long)b0.shape[0];\n"),
        "{inc}"
    );
    assert!(!inc.contains("a0.shape"), "{inc}");
    // One C slot, spelled as the pycc-owned POD and never as `Py_buffer`,
    // and the export still rides the `fnptr_` cast path -- no thunk.
    assert!(
        inc.contains("    result = ((double (*)(PyccExtBufferView *))fnptr_total)(&a0);\n"),
        "{inc}"
    );
    assert!(!inc.contains("pycc_ext_thunk_total"), "{inc}");
    assert!(!inc.contains("Py_buffer *"), "{inc}");
}

#[test]
fn a_memoryview_parameter_is_released_on_the_success_path_and_on_the_pending_bail() {
    let inc = memoryview_inc("total", 1, Ty::Float);
    // The half nothing else in the tree would notice was missing: the
    // buffer is still held when the compiled call returns normally.
    assert!(
        inc.contains(
            "    pycc_ext_bridge_release_to(bridge_mark);\n    PyBuffer_Release(&b0);\n    \
             return pycc_ext_pack_float(result);\n}\n\n"
        ),
        "{inc}"
    );
    // And on the other exit past the acquisition -- a pycc exception the
    // compiled body raised. Released before the CPython exception is set,
    // so an exporter's own `releasebuffer` cannot run with one pending.
    assert!(
        inc.contains(
            "    if (pycc_rt_ext_pending_type() >= 0) {\n        PyBuffer_Release(&b0);\n        \
             pycc_ext_raise_pending();\n        pycc_ext_bridge_release_to(bridge_mark);\n        \
             return NULL;\n    }\n"
        ),
        "{inc}"
    );
    // Exactly two releases: the two exits that can be reached with the
    // buffer held. A third would be a double release.
    assert_eq!(inc.matches("PyBuffer_Release(&b0);").count(), 2, "{inc}");
}

#[test]
fn a_none_returning_memoryview_export_releases_before_returning_none() {
    // Returning `None` is a `return` with no pack in front of it, so the
    // release has to precede it rather than sit anywhere after the pack.
    let inc = memoryview_inc("consume", 1, Ty::None);
    assert!(
        inc.contains("    PyBuffer_Release(&b0);\n    return Py_NewRef(Py_None);\n}\n\n"),
        "{inc}"
    );
}

#[test]
fn a_second_memoryview_argument_that_refuses_releases_the_first() {
    let inc = memoryview_inc("dot", 2, Ty::Float);
    // The bail path for argument 2 owes argument 1's buffer. Without it,
    // every `TypeError` on the second argument leaks an exporter lock --
    // invisible to the artifact and to every assertion that only checks
    // that a refusal happened.
    assert!(
        inc.contains(
            "    if (pycc_ext_unpack_memoryview(args[1], \"dot\", 1, 0, &b1) != 0) {\n        \
             PyBuffer_Release(&b0);\n        return NULL;\n    }\n"
        ),
        "{inc}"
    );
    // Argument 1's own bail owes nothing: nothing was acquired yet.
    assert!(
        inc.contains(
            "    if (pycc_ext_unpack_memoryview(args[0], \"dot\", 0, 0, &b0) != 0) {\n        \
             return NULL;\n    }\n"
        ),
        "{inc}"
    );
    // And both are released on the way out.
    assert!(
        inc.contains("    PyBuffer_Release(&b0);\n    PyBuffer_Release(&b1);\n    return "),
        "{inc}"
    );
    // One Python argument per declared parameter, not one per C slot.
    assert!(inc.contains("if (nargs != 2)"), "{inc}");
}

#[test]
fn a_mixed_str_and_memoryview_signature_owes_each_slot_its_own_cleanup() {
    // The whole point of replacing #1049's `"str"` literal test with a
    // carrier-level property: two cleanup classes in one wrapper, each
    // emitted for the slots that actually owe it.
    let inc = inc_no_classes(
        "m",
        &[ExtExport {
            defaults: Vec::new(),
            name: "label".to_string(),
            class: None,
            method: None,
            returns_buffer_slice: false,
            receiver: ExtReceiver::None,
            params: vec![Ty::Str, Ty::MemoryView, Ty::Int],
            param_writable: vec![false; 3],
            return_ty: Ty::Int,
            keyword_names: None,
        }],
    );
    assert!(
        inc.contains(
            "    if (pycc_ext_unpack_memoryview(args[1], \"label\", 1, 0, &b1) != 0) {\n        \
             pycc_rt_str_decref(a0);\n        return NULL;\n    }\n"
        ),
        "{inc}"
    );
    assert!(
        inc.contains(
            "    if (pycc_ext_unpack_int(args[2], \"label\", 2, &a2) != 0) {\n        \
             pycc_rt_str_decref(a0);\n        PyBuffer_Release(&b1);\n        return NULL;\n    }\n"
        ),
        "{inc}"
    );
    // A `str` owes nothing on the success path -- the compiled function's
    // own parameter slot consumed it -- so only the buffer is released
    // there.
    assert!(
        inc.contains("    PyBuffer_Release(&b1);\n    return pycc_ext_pack_int("),
        "{inc}"
    );
    assert_eq!(inc.matches("pycc_rt_str_decref(a0);").count(), 2, "{inc}");
}

#[test]
fn an_export_with_no_memoryview_parameter_emits_no_release_at_all() {
    // The byte-identity guard for every wrapper generated before Part 1 of
    // #1027: the success-path cleanup block is empty when nothing owes it.
    let inc = inc_no_classes(
        "m",
        &[ExtExport {
            defaults: Vec::new(),
            name: "greet".to_string(),
            class: None,
            method: None,
            returns_buffer_slice: false,
            receiver: ExtReceiver::None,
            params: vec![Ty::Str],
            param_writable: vec![false; 1],
            return_ty: Ty::Str,
            keyword_names: None,
        }],
    );
    assert!(!inc.contains("PyBuffer_Release"), "{inc}");
    assert!(
        inc.contains(
            "    if (pycc_rt_ext_pending_type() >= 0) {\n        pycc_ext_raise_pending();\n        \
             pycc_ext_bridge_release_to(bridge_mark);\n        return NULL;\n    }\n    \
             pycc_ext_bridge_release_to(bridge_mark);\n    return pycc_ext_pack_str(result);\n"
        ),
        "{inc}"
    );
}
