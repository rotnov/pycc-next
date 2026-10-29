//! Generated per-export wrappers: arity, scalar/str carriers and exception checks.

use super::*;

#[test]
fn the_generated_companion_defines_the_module_name_in_both_spellings() {
    let inc = inc_no_classes("fastmath", &[]);
    assert!(
        inc.contains("#define PYCC_EXT_MODULE_NAME fastmath\n"),
        "{inc}"
    );
    assert!(
        inc.contains("#define PYCC_EXT_MODULE_NAME_STR \"fastmath\"\n"),
        "{inc}"
    );
    // An export-free module still emits a terminated method table, which is
    // what `PyModuleDef` requires.
    assert!(
        inc.contains("static PyMethodDef pycc_ext_methods[] = {"),
        "{inc}"
    );
    assert!(inc.contains("    {NULL, NULL, 0, NULL},\n};\n"), "{inc}");
}

#[test]
fn a_nullary_export_declares_a_void_parameter_list_and_checks_its_arity() {
    let inc = inc_no_classes(
        "m",
        &[ExtExport {
            defaults: Vec::new(),
            name: "answer".to_string(),
            class: None,
            method: None,
            returns_buffer_slice: false,
            receiver: ExtReceiver::None,
            param_names: crate::ext_build::test_param_names(&[]),
            params: Vec::new(),
            param_writable: Vec::new(),
            return_ty: Ty::Int,
            keyword_names: None,
            keyword_receiver: None,
        }],
    );
    assert!(inc.contains("extern void *fnptr_answer;"), "{inc}");
    assert!(
        inc.contains("result = ((long long (*)(void))fnptr_answer)();"),
        "{inc}"
    );
    assert!(
        inc.contains("if (nargs < 0 || nargs > 0) {"),
        "{inc}"
    );
    assert!(
        inc.contains("(PyCFunction)(void (*)(void))pycc_ext_wrap_answer, METH_FASTCALL"),
        "{inc}"
    );
}

#[test]
fn a_unary_export_uses_the_singular_arity_message_and_unpacks_one_argument() {
    let inc = inc_no_classes(
        "m",
        &[ExtExport {
            defaults: Vec::new(),
            name: "square".to_string(),
            class: None,
            method: None,
            returns_buffer_slice: false,
            receiver: ExtReceiver::None,
            param_names: crate::ext_build::test_param_names(&[Ty::Int]),
            params: vec![Ty::Int],
            param_writable: vec![false; 1],
            return_ty: Ty::Int,
            keyword_names: None,
            keyword_receiver: None,
        }],
    );
    assert!(
        inc.contains("if (nargs < 1 || nargs > 1) {"),
        "{inc}"
    );
    assert!(
        inc.contains("pycc_ext_unpack_int(args[0], \"square\", 0, &a0)"),
        "{inc}"
    );
    assert!(
        inc.contains("result = ((long long (*)(long long))fnptr_square)(a0);"),
        "{inc}"
    );
    assert!(
        inc.contains("return pycc_ext_pack_int(\"square\", result);"),
        "{inc}"
    );
}

#[test]
fn a_binary_export_unpacks_each_argument_at_its_own_index() {
    let inc = inc_no_classes(
        "m",
        &[ExtExport {
            defaults: Vec::new(),
            name: "add".to_string(),
            class: None,
            method: None,
            returns_buffer_slice: false,
            receiver: ExtReceiver::None,
            param_names: crate::ext_build::test_param_names(&[Ty::Int, Ty::Int]),
            params: vec![Ty::Int, Ty::Int],
            param_writable: vec![false; 2],
            return_ty: Ty::Int,
            keyword_names: None,
            keyword_receiver: None,
        }],
    );
    assert!(
        inc.contains("if (nargs < 2 || nargs > 2) {"),
        "{inc}"
    );
    assert!(
        inc.contains("pycc_ext_unpack_int(args[0], \"add\", 0, &a0)"),
        "{inc}"
    );
    assert!(
        inc.contains("pycc_ext_unpack_int(args[1], \"add\", 1, &a1)"),
        "{inc}"
    );
    assert!(
        inc.contains("result = ((long long (*)(long long, long long))fnptr_add)(a0, a1);"),
        "{inc}"
    );
}

#[test]
fn every_wrapper_checks_the_runtime_exception_flag_before_packing_a_result() {
    // A compiled function that raised returns a neutral carrier and leaves
    // the thread-local flag set, so packing it first would hand Python a
    // fabricated value instead of the exception.
    let inc = inc_no_classes(
        "m",
        &[ExtExport {
            defaults: Vec::new(),
            name: "risky".to_string(),
            class: None,
            method: None,
            returns_buffer_slice: false,
            receiver: ExtReceiver::None,
            param_names: crate::ext_build::test_param_names(&[Ty::Int]),
            params: vec![Ty::Int],
            param_writable: vec![false; 1],
            return_ty: Ty::Int,
            keyword_names: None,
            keyword_receiver: None,
        }],
    );
    let check = inc
        .find("pycc_rt_ext_pending_type() >= 0")
        .expect("the wrapper checks the pending flag");
    let pack = inc
        .find("pycc_ext_pack_int(\"risky\"")
        .expect("the wrapper packs a result");
    assert!(
        check < pack,
        "the pending check must precede the pack:\n{inc}"
    );
}

#[test]
fn a_float_export_carries_a_double_through_every_slot_of_the_wrapper() {
    let inc = inc_no_classes(
        "m",
        &[ExtExport {
            defaults: Vec::new(),
            name: "scale".to_string(),
            class: None,
            method: None,
            returns_buffer_slice: false,
            receiver: ExtReceiver::None,
            param_names: crate::ext_build::test_param_names(&[Ty::Float]),
            params: vec![Ty::Float],
            param_writable: vec![false; 1],
            return_ty: Ty::Float,
            keyword_names: None,
            keyword_receiver: None,
        }],
    );
    assert!(inc.contains("    double result;"), "{inc}");
    // The half-widened wrapper is the sharpest failure mode here: a
    // `long long result` left behind while the cast and the pack move to
    // `double` compiles clean, warns nothing, and returns 2 for 2.5.
    assert!(!inc.contains("long long result"), "{inc}");
    assert!(inc.contains("    double a0;"), "{inc}");
    assert!(
        inc.contains("pycc_ext_unpack_float(args[0], \"scale\", 0, &a0)"),
        "{inc}"
    );
    assert!(
        inc.contains("result = ((double (*)(double))fnptr_scale)(a0);"),
        "{inc}"
    );
    // The float packer cannot fail on a value, so it takes no function name.
    assert!(inc.contains("return pycc_ext_pack_float(result);"), "{inc}");
}

#[test]
fn a_bool_export_uses_a_one_byte_c_type_to_match_the_compiled_i8_slot() {
    let inc = inc_no_classes(
        "m",
        &[ExtExport {
            defaults: Vec::new(),
            name: "negate".to_string(),
            class: None,
            method: None,
            returns_buffer_slice: false,
            receiver: ExtReceiver::None,
            param_names: crate::ext_build::test_param_names(&[Ty::Bool]),
            params: vec![Ty::Bool],
            param_writable: vec![false; 1],
            return_ty: Ty::Bool,
            keyword_names: None,
            keyword_receiver: None,
        }],
    );
    assert!(inc.contains("    char result;"), "{inc}");
    assert!(inc.contains("    char a0;"), "{inc}");
    assert!(
        inc.contains("pycc_ext_unpack_bool(args[0], \"negate\", 0, &a0)"),
        "{inc}"
    );
    assert!(
        inc.contains("result = ((char (*)(char))fnptr_negate)(a0);"),
        "{inc}"
    );
    assert!(inc.contains("return pycc_ext_pack_bool(result);"), "{inc}");
    // `_Bool` is not `i1`-shaped here and `int` is four bytes: either would
    // disagree with the callee across an unchecked `void *` cast.
    assert!(!inc.contains("_Bool"), "{inc}");
    assert!(!inc.contains("(int (*)"), "{inc}");
}

#[test]
fn a_none_returning_export_casts_to_void_and_declares_no_result_at_all() {
    let inc = inc_no_classes(
        "m",
        &[ExtExport {
            defaults: Vec::new(),
            name: "sink".to_string(),
            class: None,
            method: None,
            returns_buffer_slice: false,
            receiver: ExtReceiver::None,
            param_names: crate::ext_build::test_param_names(&[Ty::Int]),
            params: vec![Ty::Int],
            param_writable: vec![false; 1],
            return_ty: Ty::None,
            keyword_names: None,
            keyword_receiver: None,
        }],
    );
    // LLVM emits a `None` return as `void`, so there is nothing to hold and
    // nothing to assign -- both would be C type errors.
    assert!(!inc.contains("result"), "{inc}");
    assert!(
        inc.contains("((void (*)(long long))fnptr_sink)(a0);"),
        "{inc}"
    );
    assert!(inc.contains("    return Py_NewRef(Py_None);"), "{inc}");
    assert!(!inc.contains("pycc_ext_pack"), "{inc}");
}

#[test]
fn a_mixed_signature_gives_each_slot_its_own_c_type_and_unpack_helper() {
    let inc = inc_no_classes(
        "m",
        &[ExtExport {
            defaults: Vec::new(),
            name: "mix".to_string(),
            class: None,
            method: None,
            returns_buffer_slice: false,
            receiver: ExtReceiver::None,
            param_names: crate::ext_build::test_param_names(&[Ty::Int, Ty::Float, Ty::Bool]),
            params: vec![Ty::Int, Ty::Float, Ty::Bool],
            param_writable: vec![false; 3],
            return_ty: Ty::Float,
            keyword_names: None,
            keyword_receiver: None,
        }],
    );
    assert!(
        inc.contains("    long long a0;\n    double a1;\n    char a2;\n"),
        "{inc}"
    );
    assert!(
        inc.contains("pycc_ext_unpack_int(args[0], \"mix\", 0, &a0)"),
        "{inc}"
    );
    assert!(
        inc.contains("pycc_ext_unpack_float(args[1], \"mix\", 1, &a1)"),
        "{inc}"
    );
    assert!(
        inc.contains("pycc_ext_unpack_bool(args[2], \"mix\", 2, &a2)"),
        "{inc}"
    );
    assert!(
        inc.contains("result = ((double (*)(long long, double, char))fnptr_mix)(a0, a1, a2);"),
        "{inc}"
    );
}

#[test]
fn a_none_returning_wrapper_checks_the_exception_flag_before_returning_none() {
    // The arm with no result to inspect is exactly the one where skipping
    // the check would hand Python a fabricated `None` instead of the
    // exception the compiled function raised.
    let inc = inc_no_classes(
        "m",
        &[ExtExport {
            defaults: Vec::new(),
            name: "risky".to_string(),
            class: None,
            method: None,
            returns_buffer_slice: false,
            receiver: ExtReceiver::None,
            param_names: crate::ext_build::test_param_names(&[Ty::Int]),
            params: vec![Ty::Int],
            param_writable: vec![false; 1],
            return_ty: Ty::None,
            keyword_names: None,
            keyword_receiver: None,
        }],
    );
    let check = inc
        .find("pycc_rt_ext_pending_type() >= 0")
        .expect("the wrapper checks the pending flag");
    let egress = inc
        .find("return Py_NewRef(Py_None);")
        .expect("the wrapper returns None");
    assert!(
        check < egress,
        "the pending check must precede the None egress:\n{inc}"
    );
}

#[test]
fn a_str_export_carries_an_opaque_pointer_in_both_positions() {
    // Part 2 of #1037 (#1049). `ty_to_basic_type` gives `Ty::Str` a pointer,
    // so the cast the wrapper calls through must spell `void *` at the
    // parameter and the return alike -- a width that disagrees with the
    // callee is a silent miscompile, not a compile error. Note the space
    // after the star: the declaration is `{c_type} a{index}`.
    let inc = inc_no_classes(
        "m",
        &[ExtExport {
            defaults: Vec::new(),
            name: "shout".to_string(),
            class: None,
            method: None,
            returns_buffer_slice: false,
            receiver: ExtReceiver::None,
            param_names: crate::ext_build::test_param_names(&[Ty::Str]),
            params: vec![Ty::Str],
            param_writable: vec![false; 1],
            return_ty: Ty::Str,
            keyword_names: None,
            keyword_receiver: None,
        }],
    );
    assert!(inc.contains("    void * result;\n"), "{inc}");
    assert!(inc.contains("    void * a0;\n"), "{inc}");
    assert!(
        inc.contains("pycc_ext_unpack_str(args[0], \"shout\", 0, &a0)"),
        "{inc}"
    );
    assert!(
        inc.contains("result = ((void * (*)(void *))fnptr_shout)(a0);"),
        "{inc}"
    );
    // `pack_str` takes no function name: unlike `pack_int` it refuses no
    // value, so there is no message for a name to appear in.
    assert!(inc.contains("return pycc_ext_pack_str(result);"), "{inc}");
}

#[test]
fn a_str_unpack_failure_releases_every_str_argument_already_taken() {
    // `pycc_ext_unpack_str` hands back a fresh `PyStrObj` that only the
    // compiled function's own parameter slot ever consumes. A `TypeError` on
    // a later argument bails before the call, so without this cleanup every
    // raising call would leak each earlier `str`.
    let inc = inc_no_classes(
        "m",
        &[ExtExport {
            defaults: Vec::new(),
            name: "join".to_string(),
            class: None,
            method: None,
            returns_buffer_slice: false,
            receiver: ExtReceiver::None,
            param_names: crate::ext_build::test_param_names(&[Ty::Str, Ty::Str]),
            params: vec![Ty::Str, Ty::Str],
            param_writable: vec![false; 2],
            return_ty: Ty::Str,
            keyword_names: None,
            keyword_receiver: None,
        }],
    );
    // The first argument's own failure branch owes nothing -- nothing has
    // been taken yet -- and the second's owes exactly the first.
    assert!(
        inc.contains(
            "    if (pycc_ext_unpack_str(args[0], \"join\", 0, &a0) != 0) {\n        return NULL;\n    }\n"
        ),
        "{inc}"
    );
    assert!(
        inc.contains(
            "    if (pycc_ext_unpack_str(args[1], \"join\", 1, &a1) != 0) {\n        pycc_rt_str_decref(a0);\n        return NULL;\n    }\n"
        ),
        "{inc}"
    );

    // The invariant is positional, not an absence: a release may appear
    // before the call and must never appear after it. A decref after the
    // call would be a use-after-free, because the callee's parameter slot
    // already owns the reference and `decref_str_slot_before_store` releases
    // it on any reassignment. Anchored on `))fnptr_` rather than `fnptr_`:
    // the wrapper's first line is `extern void *fnptr_join;`, so a bare
    // search would land at offset 0 and invert the comparison.
    let call = inc.find("))fnptr_join)").expect("the indirect call");
    let egress = inc
        .find("return pycc_ext_pack_str(result);")
        .expect("the egress");
    assert!(call < egress, "{inc}");
    for (offset, _) in inc.match_indices("pycc_rt_str_decref") {
        assert!(
            offset < call,
            "a release at {offset} lands at or after the call at {call}: the callee's \
             parameter slot owns the reference once the call is made, so releasing it \
             here is a use-after-free\n{inc}"
        );
    }
}
