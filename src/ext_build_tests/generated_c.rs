//! Generated-text tests for the `--ext` build seam: the exact C the driver
//! writes into the exports companion, and the properties of the tracked shim
//! the generated wrappers depend on by name.
//!
//! Split out of `ext_build_tests.rs` by #1049 under `AGENTS.md`'s
//! decomposability rule; the tests themselves are unchanged except for the
//! `str` coverage #1049 added.

use super::*;

/// [`generate_exports_inc`] for a program that declares no user exception
/// class -- every case in this file that predates #1066, and the one that
/// asserts both generated class functions are still emitted with empty
/// bodies so an artifact with no such class still links.
pub(super) fn inc_no_classes(module_name: &str, exports: &[ExtExport]) -> String {
    generate_exports_inc(
        module_name,
        exports,
        &[],
        &flat_publications(exports),
        &[],
        &[],
    )
}

/// The publication list of a program whose classes inherit nothing: one
/// entry per exporting class, in first-export order, carrying exactly that
/// class's own exports.
///
/// The inheritance-resolving walk is [`collect_class_publications`]', and it
/// needs a `HirModule` to see an MRO at all. Every case here hands
/// [`generate_exports_inc`] a hand-built export list instead, so this helper
/// states the base-less shape those cases mean, and the resolver's own
/// behaviour is pinned separately in `exports.rs` against real lowered HIR.
fn flat_publications(exports: &[ExtExport]) -> Vec<ExtPublishedClass> {
    let mut published: Vec<ExtPublishedClass> = Vec::new();
    for export in exports {
        let Some(class) = &export.class else {
            continue;
        };
        match published.iter_mut().find(|held| held.class == *class) {
            Some(held) => held.methods.push(export.clone()),
            None => published.push(ExtPublishedClass {
                class: class.clone(),
                methods: vec![export.clone()],
                mro: vec![class.clone()],
            }),
        }
    }
    published
}

/// The generated companion for a program lowered from real source, so the
/// tags and the class order are the ones `pycc_hir::program::finalize`
/// really assigns rather than ones a hand-built `HirClassDef` asserts into
/// existence (`module()` here builds an empty `class_defs`).
fn inc_from_source(source: &str) -> String {
    let dir = pycc_scratch::ScratchDir::new("ext_exception_classes").expect("scratch");
    let src = dir.join("m.py");
    std::fs::write(&src, source).expect("write source");
    let module = crate::frontend::resolve_frontend(&src, Some("m"))
        .unwrap_or_else(|_| panic!("the fixture must type-check"));
    generate_exports_inc(
        "m",
        &[],
        &collect_user_exception_classes(&module),
        &[],
        &[],
        &[],
    )
}

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
            params: Vec::new(),
            param_writable: Vec::new(),
            return_ty: Ty::Int,
            keyword_names: None,
        }],
    );
    assert!(inc.contains("extern void *fnptr_answer;"), "{inc}");
    assert!(
        inc.contains("result = ((long long (*)(void))fnptr_answer)();"),
        "{inc}"
    );
    assert!(
        inc.contains("takes exactly 0 arguments (%zd given)"),
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
            params: vec![Ty::Int],
            param_writable: vec![false; 1],
            return_ty: Ty::Int,
            keyword_names: None,
        }],
    );
    assert!(
        inc.contains("takes exactly 1 argument (%zd given)"),
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
        inc.contains("return pycc_ext_activation_exit(pycc_ext_pack_int(\"square\", result));"),
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
            params: vec![Ty::Int, Ty::Int],
            param_writable: vec![false; 2],
            return_ty: Ty::Int,
            keyword_names: None,
        }],
    );
    assert!(
        inc.contains("takes exactly 2 arguments (%zd given)"),
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
            params: vec![Ty::Int],
            param_writable: vec![false; 1],
            return_ty: Ty::Int,
            keyword_names: None,
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
            params: vec![Ty::Float],
            param_writable: vec![false; 1],
            return_ty: Ty::Float,
            keyword_names: None,
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
    assert!(
        inc.contains("return pycc_ext_activation_exit(pycc_ext_pack_float(result));"),
        "{inc}"
    );
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
            params: vec![Ty::Bool],
            param_writable: vec![false; 1],
            return_ty: Ty::Bool,
            keyword_names: None,
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
    assert!(
        inc.contains("return pycc_ext_activation_exit(pycc_ext_pack_bool(result));"),
        "{inc}"
    );
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
            params: vec![Ty::Int],
            param_writable: vec![false; 1],
            return_ty: Ty::None,
            keyword_names: None,
        }],
    );
    // LLVM emits a `None` return as `void`, so there is nothing to hold and
    // nothing to assign -- both would be C type errors.
    assert!(!inc.contains("result"), "{inc}");
    assert!(
        inc.contains("((void (*)(long long))fnptr_sink)(a0);"),
        "{inc}"
    );
    assert!(
        inc.contains("    return pycc_ext_activation_exit(Py_NewRef(Py_None));"),
        "{inc}"
    );
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
            params: vec![Ty::Int, Ty::Float, Ty::Bool],
            param_writable: vec![false; 3],
            return_ty: Ty::Float,
            keyword_names: None,
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
            params: vec![Ty::Int],
            param_writable: vec![false; 1],
            return_ty: Ty::None,
            keyword_names: None,
        }],
    );
    let check = inc
        .find("pycc_rt_ext_pending_type() >= 0")
        .expect("the wrapper checks the pending flag");
    let egress = inc
        .find("return pycc_ext_activation_exit(Py_NewRef(Py_None));")
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
            params: vec![Ty::Str],
            param_writable: vec![false; 1],
            return_ty: Ty::Str,
            keyword_names: None,
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
    assert!(
        inc.contains("return pycc_ext_activation_exit(pycc_ext_pack_str(result));"),
        "{inc}"
    );
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
            params: vec![Ty::Str, Ty::Str],
            param_writable: vec![false; 2],
            return_ty: Ty::Str,
            keyword_names: None,
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
        .find("return pycc_ext_activation_exit(pycc_ext_pack_str(result));")
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

#[test]
fn the_embedded_shim_is_the_tracked_c_file_and_declares_the_limited_api_floor() {
    assert!(
        SHIM_C.contains("#define Py_LIMITED_API 0x030D0000"),
        "{SHIM_C}"
    );
    // The multi-phase init contract: `PyInit_` only returns the def.
    assert!(SHIM_C.contains("PyModuleDef_Init(&pycc_ext_moduledef)"));
    // The free-threaded guard, and the detector that actually works:
    // the version string, not `sys._is_gil_enabled()`.
    assert!(SHIM_C.contains("strstr(Py_GetVersion(), \"free-threading build\")"));
    // It includes the generated companion under the exact name this module
    // writes it as, so the two really do land side by side.
    assert!(SHIM_C.contains(&format!("#include \"{EXPORTS_INC_NAME}\"")));
    assert_eq!(SHIM_C_NAME, "pycc_ext_module.c");
    // The helpers the generated wrappers call by name for each carried
    // type. A wrapper that names a helper the shim does not define fails
    // at C compile time, which no unit test here can reach.
    for helper in [
        "static int pycc_ext_unpack_int(PyObject *obj",
        "static int pycc_ext_unpack_float(PyObject *obj",
        "static int pycc_ext_unpack_bool(PyObject *obj",
        "static PyObject *pycc_ext_pack_int(",
        "static PyObject *pycc_ext_pack_float(double value)",
        "static PyObject *pycc_ext_pack_bool(char value)",
        "static int pycc_ext_unpack_str(PyObject *obj",
        "static PyObject *pycc_ext_pack_str(void *result)",
        // Part 1 of #1027. No packer: a `memoryview` is admitted at a
        // parameter position only, so there is no symmetric pair here.
        "static int pycc_ext_unpack_memoryview(PyObject *obj",
        // The POD the wrapper hands to compiled code, typedef'd above the
        // point the generated companion is included at, or every generated
        // buffer local would be an undeclared type in clang.
        "} PyccExtBufferView;",
        // #1427: what the generated comparison and hash slots call.
        "static Py_hash_t pycc_ext_finish_hash(PyObject *res)",
        "static Py_hash_t pycc_ext_identity_hash(PyObject *self)",
        "static int pycc_ext_slot_operand_is(PyObject *other, const char *class_name);",
    ] {
        assert!(SHIM_C.contains(helper), "{helper}");
    }
    // R3: the lookup's forward declaration here and its definition in the
    // generated companion are two hand-written spellings of one signature,
    // bound by nothing until a C compiler runs. Both are asserted against
    // the same constant, so a drift fails an ordinary `cargo test`.
    assert!(
        shim_c().contains(&format!("{USER_EXCEPTION_LOOKUP_DECL};")),
        "{USER_EXCEPTION_LOOKUP_DECL}"
    );
    // Part 7 of #1371: the compiled-class `isinstance` is the same
    // forward-declared-above, defined-in-the-companion arrangement.
    assert!(
        shim_c().contains(&format!("{COMPILED_CLASS_ISINSTANCE_DECL};")),
        "{COMPILED_CLASS_ISINSTANCE_DECL}"
    );
    // ... and the fall-through it calls is defined there under the one
    // spelling the generator uses.
    assert!(
        shim_c().contains(&format!(
            "static int {UNPUBLISHED_CLASS_ISINSTANCE}(PyObject *o)\n{{"
        )),
        "{UNPUBLISHED_CLASS_ISINSTANCE}"
    );
    // The registration call is the same two-spellings problem, so it is
    // built from its own constant rather than hand-typed: a rename that
    // updates the constant, the generator and the shim together would
    // otherwise leave a stale literal here and fail only in a C compiler.
    let register_name = USER_EXCEPTION_REGISTER_DECL
        .split('(')
        .next()
        .and_then(|head| head.rsplit(' ').next())
        .expect("the register declaration names a function");
    assert!(
        shim_c().contains(&format!("if ({register_name}(module) != 0) {{")),
        "{SHIM_C}"
    );
    // The module body runs in the exec slot, never in `PyInit_`.
    assert!(SHIM_C.contains("{Py_mod_exec, (void *)pycc_ext_exec_module}"));
    assert!(SHIM_C.contains("Py_MOD_MULTIPLE_INTERPRETERS_NOT_SUPPORTED"));
    // The symbol codegen emits for the module body under `--ext`.
    assert!(SHIM_C.contains(pycc_codegen::EXT_MODULE_EXEC_SYMBOL));
}

#[test]
fn the_exec_slot_only_raises_its_generic_import_error_when_nothing_else_did() {
    // A module body that fails reports through one of two channels:
    // `pycc_rt`'s thread-local pending state, or an exception CPython
    // itself set with no pycc pending state at all. `pycc_ext_raise_pending`
    // returns 0 in the second case, so a guard that tests it alone replaces
    // the real exception (a `ModuleNotFoundError` from a failed host
    // import, say) with "pycc module body failed". The guard must also ask
    // `PyErr_Occurred`, and must ask `pycc_ext_raise_pending` first: that
    // call is what sets the exception, so short-circuiting must not skip
    // it. Only a C compiler and a live interpreter can execute this path,
    // and no source the current tree compiles reaches it -- the end-to-end
    // `ModuleNotFoundError` assertion arrives with foreign imports -- so
    // the shim text is asserted here the way every other shim invariant in
    // this file is.
    assert!(
        SHIM_C.contains("if (!pycc_ext_raise_pending() && !PyErr_Occurred()) {"),
        "{SHIM_C}"
    );
    assert!(
        !SHIM_C.contains("if (!pycc_ext_raise_pending()) {"),
        "the unguarded form overwrites a CPython-set exception\n{SHIM_C}"
    );
    // The fallback it guards is still there: a body that fails with neither
    // channel set must not import successfully.
    assert!(
        SHIM_C.contains("PyErr_SetString(PyExc_ImportError, \"pycc module body failed\");"),
        "{SHIM_C}"
    );
}

#[test]
fn the_shims_bigint_egress_releases_the_reference_before_it_raises() {
    // D-180 rule 6 hands the wrapper a retained return value, so the arm
    // the D-244 amendment turns into an `OverflowError` has to drop it --
    // otherwise every refused bigint result leaks a `BigIntObj`.
    assert!(SHIM_C.contains("extern void pycc_rt_bigint_release(long long word);"));
    let body = &SHIM_C[SHIM_C
        .find("static PyObject *pycc_ext_pack_int(")
        .expect("the pack helper")..];
    let bigint_arm = body.find("case PYCC_EXT_INT_BIGINT:").expect("bigint arm");
    let release = body
        .find("pycc_rt_bigint_release(encoded);")
        .expect("the release");
    let raise = body.find("PyExc_OverflowError").expect("the OverflowError");
    assert!(
        bigint_arm < release && release < raise,
        "the reference must be dropped on the bigint arm, before the raise"
    );
    // The unclassifiable-word arm must not release: it is not a pointer
    // this code knows to be live.
    let fallback = &body[body.find("    default:").expect("the default arm")..];
    assert!(!fallback.contains("pycc_rt_bigint_release"), "{fallback}");
}

#[test]
fn the_shims_str_egress_releases_the_reference_unconditionally() {
    // D-180 rule 6 hands the wrapper a retained `str`, and the CPython
    // object `PyUnicode_FromStringAndSize` builds is an independent copy --
    // so the pycc-side reference dies here on every path, the
    // allocation-failure one included. Releasing only on success would leak
    // a `PyStrObj` exactly when the interpreter is already out of memory.
    assert!(SHIM_C.contains("extern void pycc_rt_str_decref(void *s);"));
    assert!(
        SHIM_C.contains("extern const unsigned char *pycc_rt_ext_str_bytes(void *s, size_t *len);")
    );
    let shim = shim_c();
    let body = &shim[shim
        .find("static PyObject *pycc_ext_pack_str(void *result)")
        .expect("the pack helper")..];
    let end = body.find("\n}\n").expect("the helper's end");
    let body = &body[..end];
    let pack = body
        .find("packed = PyUnicode_FromStringAndSize(")
        .expect("the CPython copy");
    let release = body
        .find("pycc_rt_str_decref(result);")
        .expect("the release");
    let ret = body.find("return packed;").expect("the return");
    assert!(
        pack < release && release < ret,
        "the reference must be released after the copy and before the return, on \
         both the success and the failure path:\n{body}"
    );
    // No `if (packed == NULL)` guard around the release: an early return on
    // the failure path is exactly the leak this pins shut.
    assert!(!body.contains("if (packed"), "{body}");
}

#[test]
fn the_shims_str_ingress_checks_the_type_before_the_converter_and_carries_a_length() {
    // `PyUnicode_Check` first and no converter fallback, for the same reason
    // `unpack_float` refuses an `int`: D-244 rule 7 defers to
    // `docs/TYPE_SYSTEM.md` rule 4 (D-086), so an `os.PathLike` or a
    // `__str__` duck type is a `TypeError` here rather than a coercion.
    let shim = shim_c();
    let body = &shim[shim
        .find("static int pycc_ext_unpack_str(PyObject *obj")
        .expect("the unpack helper")..];
    // Bounded at this helper's own closing brace, the way the egress test
    // above bounds `pack_str`: the negative `strlen` assertion below would
    // otherwise read every later helper's prose too, and
    // `pycc_ext_obj_to_str` documents the same no-`strlen` rule in its own
    // comment.
    let end = body.find("\n}\n").expect("the helper's end");
    let body = &body[..end];
    let check = body.find("!PyUnicode_Check(obj)").expect("the type check");
    let type_error = body.find("PyExc_TypeError").expect("the refusal");
    let convert = body
        .find("utf8 = PyUnicode_AsUTF8AndSize(obj, &size);")
        .expect("the conversion");
    assert!(
        check < type_error && type_error < convert,
        "the type must be refused before the converter runs:\n{body}"
    );
    // The length is carried explicitly rather than re-derived: a Python
    // `str` may hold embedded NUL bytes, and the empty string is legal.
    assert!(!body.contains("strlen"), "{body}");
    assert!(
        body.contains("pycc_rt_str_from_literal((const unsigned char *)utf8, (long long)size)"),
        "{body}"
    );
    // A lone surrogate has no UTF-8 encoding: `PyUnicode_AsUTF8AndSize`
    // raises `UnicodeEncodeError`, which the D-244 amendment propagates
    // verbatim rather than translating into a `TypeError`.
    assert!(
        body.contains("if (utf8 == NULL) {\n        return -1;\n    }"),
        "{body}"
    );
}

#[test]
fn the_shims_unpack_order_puts_bool_before_int_before_the_type_error() {
    // Searched inside the function body, past its own doc comment, so the
    // comment's prose ordering cannot stand in for the code's. `_at` is the
    // body (#1050 added the element-index parameter there); the
    // `pycc_ext_unpack_int` of the scalar generated C is now a one-line
    // forwarder onto it, and both share this one ordering.
    let body = &SHIM_C[SHIM_C
        .find("static int pycc_ext_unpack_int_at(PyObject *obj")
        .expect("the unpack helper")..];
    let bool_check = body.find("PyBool_Check(obj)").expect("bool arm");
    let long_check = body.find("!PyLong_Check(obj)").expect("int arm");
    let convert = body
        .find("raw = PyLong_AsLongLongAndOverflow")
        .expect("conversion");
    assert!(
        bool_check < long_check && long_check < convert,
        "D-141's bool markers must be read before the int conversion flattens them"
    );
    // The inline-integer range, never `i64`: the gate is the runtime's own
    // encoder, called after CPython's overflow check (#1040).
    assert!(SHIM_C.contains("pycc_rt_ext_int_encode(raw, out) != 0"));
    assert!(SHIM_C.contains("2**62"));
}

#[test]
fn a_tuple_parameter_is_checked_once_then_unpacked_element_by_element() {
    let inc = inc_no_classes(
        "m",
        &[ExtExport {
            defaults: Vec::new(),
            name: "total".to_string(),
            class: None,
            method: None,
            returns_buffer_slice: false,
            receiver: ExtReceiver::None,
            params: vec![Ty::Tuple(Box::new(vec![Ty::Int, Ty::Float]))],
            param_writable: vec![false; 1],
            return_ty: Ty::Int,
            keyword_names: None,
        }],
    );
    // One `tuple` argument, so the arity message still says one: the
    // wrapper's argument space is the Python one and never the flattened C
    // one (#1050).
    assert!(
        inc.contains("takes exactly 1 argument (%zd given)"),
        "{inc}"
    );
    // Shape and length first -- that check is what makes every
    // `PyTuple_GetItem` below infallible.
    assert!(
        inc.contains("pycc_ext_unpack_tuple(args[0], \"total\", 0, 2) != 0"),
        "{inc}"
    );
    assert!(
        inc.contains(
            "pycc_ext_unpack_int_at(PyTuple_GetItem(args[0], 0), \"total\", 0, 0, &a0_0) != 0"
        ),
        "{inc}"
    );
    assert!(
        inc.contains(
            "pycc_ext_unpack_float_at(PyTuple_GetItem(args[0], 1), \"total\", 0, 1, &a0_1) != 0"
        ),
        "{inc}"
    );
    // The locals are per element and the call spreads them in place; no
    // aggregate is ever spelled in C.
    assert!(
        inc.contains("    long long a0_0;\n    double a0_1;\n"),
        "{inc}"
    );
    assert!(
        inc.contains("    result = pycc_ext_thunk_total(a0_0, a0_1);\n"),
        "{inc}"
    );
    assert!(
        inc.contains("extern long long pycc_ext_thunk_total(long long, double);\n"),
        "{inc}"
    );
}

#[test]
fn a_tuple_return_arrives_through_out_pointers_and_is_packed_afterwards() {
    let inc = inc_no_classes(
        "m",
        &[ExtExport {
            defaults: Vec::new(),
            name: "split".to_string(),
            class: None,
            method: None,
            returns_buffer_slice: false,
            receiver: ExtReceiver::None,
            params: vec![Ty::Int],
            param_writable: vec![false; 1],
            return_ty: Ty::Tuple(Box::new(vec![Ty::Int, Ty::Bool])),
            keyword_names: None,
        }],
    );
    assert!(
        inc.contains("extern void pycc_ext_thunk_split(long long, long long *, char *);\n"),
        "{inc}"
    );
    // No `result` local: a tuple return has no single value, and declaring
    // one would be an unused local at best and a type error at worst.
    assert!(!inc.contains("    result;"), "{inc}");
    assert!(!inc.contains("result = pycc_ext_thunk_split"), "{inc}");
    assert!(
        inc.contains("    pycc_ext_thunk_split(a0, &r0, &r1);\n"),
        "{inc}"
    );
    // The pending-exception check stands between the call and any read of
    // the out-pointer locals: a raising call leaves them uninitialized, so
    // packing first would be undefined behaviour rather than a wrong value.
    let call = inc.find("pycc_ext_thunk_split(a0").expect("the call");
    let pending = inc
        .find("pycc_rt_ext_pending_type() >= 0")
        .expect("the check");
    let first_pack = inc.find("e0 = pycc_ext_pack_int").expect("the first pack");
    assert!(call < pending && pending < first_pack, "{inc}");
    // Every element is packed before any failure is acted on: the retain
    // above gives this wrapper a reference the packer then discharges, so
    // bailing at the first failure would leak the rest.
    let second_pack = inc
        .find("e1 = pycc_ext_pack_bool(r1);")
        .expect("the second");
    let null_test = inc.find("if (e0 == NULL || e1 == NULL)").expect("the test");
    assert!(second_pack < null_test, "{inc}");
    assert!(
        inc.contains("        Py_XDECREF(e0);\n        Py_XDECREF(e1);\n"),
        "{inc}"
    );
    // `PyTuple_New` is reached with a fully-owned set, so its own failure
    // path releases unconditionally rather than with `Py_XDECREF`.
    assert!(
        inc.contains(
            "    packed = PyTuple_New(2);\n    if (packed == NULL) {\n        \
             Py_DECREF(e0);\n        Py_DECREF(e1);\n"
        ),
        "{inc}"
    );
    assert!(
        inc.contains(
            "    PyTuple_SetItem(packed, 0, e0);\n    PyTuple_SetItem(packed, 1, e1);\n    \
             return pycc_ext_activation_exit(packed);\n"
        ),
        "{inc}"
    );
}

#[test]
fn a_tuple_return_retains_each_int_element_before_packing_it() {
    // #1050 regression: a returned tuple's fields arrive *borrowed*. Codegen
    // retains at a `return` only for a `Scalar::Int`, and the export thunk
    // `extractvalue`s each field straight into its out-pointer, so a stored
    // tuple (`saved = (2**62,)`; `return saved`) hands the wrapper a word the
    // module global still owns. `pycc_ext_pack_int` releases that word on its
    // `OverflowError` path, so without this retain the second call to the
    // export faults inside the host interpreter.
    let inc = inc_no_classes(
        "m",
        &[ExtExport {
            defaults: Vec::new(),
            name: "split".to_string(),
            class: None,
            method: None,
            returns_buffer_slice: false,
            receiver: ExtReceiver::None,
            params: vec![],
            param_writable: vec![],
            return_ty: Ty::Tuple(Box::new(vec![Ty::Int, Ty::Bool, Ty::Float])),
            keyword_names: None,
        }],
    );
    assert!(
        inc.contains(
            "    pycc_rt_bigint_retain(r0);\n    e0 = pycc_ext_pack_int(\"split\", r0);\n"
        ),
        "{inc}"
    );
    // Retain and release share one predicate, so the pairing has to come from
    // one list: only the `int` slots take a reference, because only the `int`
    // packer discharges one. A retain at a `bool` or `float` slot would be an
    // unbalanced +1 on every successful call.
    assert!(!inc.contains("pycc_rt_bigint_retain(r1)"), "{inc}");
    assert!(!inc.contains("pycc_rt_bigint_retain(r2)"), "{inc}");
    assert_eq!(inc.matches("pycc_rt_bigint_retain(").count(), 1, "{inc}");
}

#[test]
fn a_one_element_tuple_keeps_its_tuple_shape_in_both_directions() {
    // Arity 1 is the shape a flattening bug erases first: `tuple[int]` and
    // `int` occupy the same single slot at the thunk, and only the
    // `PyTuple_Check` on the way in and the `PyTuple_New(1)` on the way out
    // keep the Python-level types apart.
    let inc = inc_no_classes(
        "m",
        &[ExtExport {
            defaults: Vec::new(),
            name: "wrap".to_string(),
            class: None,
            method: None,
            returns_buffer_slice: false,
            receiver: ExtReceiver::None,
            params: vec![Ty::Tuple(Box::new(vec![Ty::Int]))],
            param_writable: vec![false; 1],
            return_ty: Ty::Tuple(Box::new(vec![Ty::Int])),
            keyword_names: None,
        }],
    );
    assert!(
        inc.contains("extern void pycc_ext_thunk_wrap(long long, long long *);\n"),
        "{inc}"
    );
    assert!(
        inc.contains("pycc_ext_unpack_tuple(args[0], \"wrap\", 0, 1) != 0"),
        "{inc}"
    );
    assert!(inc.contains("    packed = PyTuple_New(1);\n"), "{inc}");
}

#[test]
fn several_tuple_parameters_keep_one_local_namespace_each() {
    let inc = inc_no_classes(
        "m",
        &[ExtExport {
            defaults: Vec::new(),
            name: "dot".to_string(),
            class: None,
            method: None,
            returns_buffer_slice: false,
            receiver: ExtReceiver::None,
            params: vec![
                Ty::Tuple(Box::new(vec![Ty::Int, Ty::Int])),
                Ty::Tuple(Box::new(vec![Ty::Float, Ty::Bool])),
            ],
            param_writable: vec![false; 2],
            return_ty: Ty::Float,
            keyword_names: None,
        }],
    );
    // `a{argument}_{element}` and never a single running counter: the second
    // tuple's first element is `a1_0`, not `a2`.
    assert!(
        inc.contains(
            "    long long a0_0;\n    long long a0_1;\n    double a1_0;\n    char a1_1;\n"
        ),
        "{inc}"
    );
    assert!(
        inc.contains("    result = pycc_ext_thunk_dot(a0_0, a0_1, a1_0, a1_1);\n"),
        "{inc}"
    );
    // Each tuple is checked against its own arity, at its own argument
    // index, and each element reports its own position.
    assert!(
        inc.contains("pycc_ext_unpack_tuple(args[1], \"dot\", 1, 2) != 0"),
        "{inc}"
    );
    assert!(
        inc.contains(
            "pycc_ext_unpack_bool_at(PyTuple_GetItem(args[1], 1), \"dot\", 1, 1, &a1_1) != 0"
        ),
        "{inc}"
    );
    assert!(
        inc.contains("takes exactly 2 arguments (%zd given)"),
        "{inc}"
    );
}

#[test]
fn an_earlier_str_argument_is_released_when_a_later_tuple_is_refused() {
    // The only owning argument type is `str`, and a refused `tuple` after it
    // exits before the call that would have consumed the reference. A tuple
    // owes nothing itself: its elements are copied out by value.
    let inc = inc_no_classes(
        "m",
        &[ExtExport {
            defaults: Vec::new(),
            name: "tag".to_string(),
            class: None,
            method: None,
            returns_buffer_slice: false,
            receiver: ExtReceiver::None,
            params: vec![Ty::Str, Ty::Tuple(Box::new(vec![Ty::Int]))],
            param_writable: vec![false; 2],
            return_ty: Ty::Str,
            keyword_names: None,
        }],
    );
    assert!(
        inc.contains(
            "    if (pycc_ext_unpack_tuple(args[1], \"tag\", 1, 1) != 0) {\n        \
             pycc_rt_str_decref(a0);\n        return NULL;\n    }\n"
        ),
        "{inc}"
    );
    assert!(
        inc.contains(
            "    if (pycc_ext_unpack_int_at(PyTuple_GetItem(args[1], 0), \"tag\", 1, 0, &a1_0) \
             != 0) {\n        pycc_rt_str_decref(a0);\n        return NULL;\n    }\n"
        ),
        "{inc}"
    );
    assert!(
        inc.contains("    result = pycc_ext_thunk_tag(a0, a1_0);\n"),
        "{inc}"
    );
}

#[test]
fn a_nullary_export_returning_a_tuple_declares_only_its_out_pointers() {
    let inc = inc_no_classes(
        "m",
        &[ExtExport {
            defaults: Vec::new(),
            name: "origin".to_string(),
            class: None,
            method: None,
            returns_buffer_slice: false,
            receiver: ExtReceiver::None,
            params: Vec::new(),
            param_writable: Vec::new(),
            return_ty: Ty::Tuple(Box::new(vec![Ty::Float, Ty::Float])),
            keyword_names: None,
        }],
    );
    // The combined list is non-empty even though the export takes nothing,
    // so `void` as a parameter list would be wrong here -- the out-pointers
    // are real parameters.
    assert!(
        inc.contains("extern void pycc_ext_thunk_origin(double *, double *);\n"),
        "{inc}"
    );
    assert!(
        inc.contains("    pycc_ext_thunk_origin(&r0, &r1);\n"),
        "{inc}"
    );
    assert!(
        inc.contains("takes exactly 0 arguments (%zd given)"),
        "{inc}"
    );
    // `pack_float` takes no function name: `PyFloat_FromDouble` cannot fail
    // on a value, unlike D-141's bigint egress.
    assert!(inc.contains("    e0 = pycc_ext_pack_float(r0);\n"), "{inc}");
}

#[test]
fn the_thunk_is_declared_as_a_function_and_called_without_a_cast() {
    // The load-bearing structural property of the whole seam. pycc's
    // aggregate calling convention is not the platform C struct ABI --
    // measured on aarch64-apple-darwin, a pycc function returning
    // `tuple[int x 5]` hands the words back in `x0`-`x4` where clang would
    // pass a hidden `sret` pointer -- so the thunk exists to keep every
    // aggregate on the LLVM side. Declaring it the way the scalar path
    // declares `fnptr_<name>` (a `void *` global, cast at the call to a
    // function-pointer type) reproduced a SIGBUS: the cast form type-checks
    // against nothing, and the `--ext` link defers undefined symbols
    // (`-undefined dynamic_lookup` on Mach-O, `-Bsymbolic` on ELF) so a
    // disagreement here links cleanly and dies on the first call.
    let inc = inc_no_classes(
        "m",
        &[ExtExport {
            defaults: Vec::new(),
            name: "pair".to_string(),
            class: None,
            method: None,
            returns_buffer_slice: false,
            receiver: ExtReceiver::None,
            params: vec![Ty::Tuple(Box::new(vec![Ty::Int, Ty::Int]))],
            param_writable: vec![false; 1],
            return_ty: Ty::Tuple(Box::new(vec![Ty::Int, Ty::Int])),
            keyword_names: None,
        }],
    );
    assert!(
        inc.contains(
            "extern void pycc_ext_thunk_pair(long long, long long, long long *, \
             long long *);\n"
        ),
        "{inc}"
    );
    assert!(!inc.contains("extern void *pycc_ext_thunk_"), "{inc}");
    assert!(
        !inc.contains("(*)(long long, long long, long long *"),
        "{inc}"
    );
    // The call goes through the thunk alone; `fnptr_pair` is read only by
    // #1199's null guard, never cast to a function type.
    assert!(!inc.contains("fnptr_pair)("), "{inc}");
    assert_eq!(inc.matches("fnptr_pair").count(), 2, "{inc}");
    // A scalar-only export in the same module keeps the legacy spelling
    // byte for byte: #1050 widens the boundary without rewriting what
    // #1036 through #1049 already emit.
    let scalar = inc_no_classes(
        "m",
        &[ExtExport {
            defaults: Vec::new(),
            name: "square".to_string(),
            class: None,
            method: None,
            returns_buffer_slice: false,
            receiver: ExtReceiver::None,
            params: vec![Ty::Int],
            param_writable: vec![false; 1],
            return_ty: Ty::Int,
            keyword_names: None,
        }],
    );
    assert!(scalar.contains("extern void *fnptr_square;"), "{scalar}");
    assert!(
        scalar.contains("result = ((long long (*)(long long))fnptr_square)(a0);"),
        "{scalar}"
    );
    assert!(!scalar.contains("pycc_ext_thunk_"), "{scalar}");
}

#[test]
fn a_tuple_carrying_export_returning_none_assigns_nothing_and_fabricates_none() {
    let inc = inc_no_classes(
        "m",
        &[ExtExport {
            defaults: Vec::new(),
            name: "record".to_string(),
            class: None,
            method: None,
            returns_buffer_slice: false,
            receiver: ExtReceiver::None,
            params: vec![Ty::Tuple(Box::new(vec![Ty::Int, Ty::Bool]))],
            param_writable: vec![false; 1],
            return_ty: Ty::None,
            keyword_names: None,
        }],
    );
    assert!(
        inc.contains("extern void pycc_ext_thunk_record(long long, char);\n"),
        "{inc}"
    );
    assert!(
        inc.contains("    pycc_ext_thunk_record(a0_0, a0_1);\n"),
        "{inc}"
    );
    assert!(!inc.contains("result"), "{inc}");
    assert!(
        inc.contains("    return pycc_ext_activation_exit(Py_NewRef(Py_None));\n"),
        "{inc}"
    );
}

#[test]
fn a_program_with_no_user_exception_class_still_emits_both_class_functions() {
    // Both are emitted unconditionally, with empty bodies: the shim calls
    // them by name whatever the program declares, so an artifact that
    // omitted them would fail to link. A program whose only exception
    // classes are the seeded builtins is this same case -- they carry
    // `None` or a fixed tag below `FIRST_USER_EXCEPTION_TYPE_TAG` and are
    // never table entries.
    let inc = inc_no_classes("m", &[]);
    assert!(
        inc.contains(&format!(
            "{USER_EXCEPTION_LOOKUP_DECL}\n{{\n    (void)tag;\n    return NULL;\n}}\n"
        )),
        "{inc}"
    );
    assert!(
        inc.contains(&format!(
            "{USER_EXCEPTION_REGISTER_DECL}\n{{\n    (void)module;\n    return 0;\n}}\n"
        )),
        "{inc}"
    );
    assert!(!inc.contains("pycc_ext_user_exception_classes["), "{inc}");
    let builtins_only = inc_from_source("try:\n    x = 1\nexcept ValueError:\n    x = 2\n");
    assert!(
        !builtins_only.contains("pycc_ext_user_exception_classes["),
        "{builtins_only}"
    );
}

#[test]
fn a_class_subclassing_exception_is_registered_under_the_qualified_module_name() {
    let inc = inc_from_source("class MyError(Exception):\n    pass\n\nx = 1\n");
    assert!(
        inc.contains("static PyObject *pycc_ext_user_exception_classes[1];"),
        "{inc}"
    );
    assert!(
        inc.contains(
            "        pycc_ext_user_exception_classes[0] = PyErr_NewException(\n            \
             PYCC_EXT_MODULE_NAME_STR \".MyError\", PyExc_Exception, NULL);\n"
        ),
        "{inc}"
    );
    // The cache owns the strong reference and `AddObjectRef` takes its own,
    // so the slot is assigned before the registration is checked.
    assert!(
        inc.contains(
            "    if (PyModule_AddObjectRef(module, \"MyError\", \
             pycc_ext_user_exception_classes[0]) < 0) {\n        return -1;\n    }\n"
        ),
        "{inc}"
    );
    // A non-NULL slot is reused rather than re-minted, so a re-import that
    // re-runs `Py_mod_exec` keeps class identity.
    assert!(
        inc.contains("    if (pycc_ext_user_exception_classes[0] == NULL) {\n"),
        "{inc}"
    );
    // Every tag the table does not hold -- tag 0, the 23..=24 pair C3 keeps
    // as `Exception`, and any sparse gap -- leaves the `switch` through an
    // explicit `return NULL`, never off the end of a non-`void` function.
    assert!(
        inc.contains("    default:\n        return NULL;\n    }\n}\n"),
        "{inc}"
    );
}

#[test]
fn a_class_subclassing_a_builtin_carries_that_builtin_as_its_base() {
    // The C1 case: without the base, a host-side `except ValueError:` does
    // not match what `except ValueError:` catches natively.
    let inc = inc_from_source("class MyValueError(ValueError):\n    pass\n\nx = 1\n");
    assert!(
        inc.contains("PYCC_EXT_MODULE_NAME_STR \".MyValueError\", PyExc_ValueError, NULL);"),
        "{inc}"
    );
}

#[test]
fn a_class_subclassing_a_user_class_names_that_class_slot_and_follows_it() {
    let inc = inc_from_source(
        "class MyValueError(ValueError):\n    pass\n\n\
         class Deep(MyValueError):\n    pass\n\nx = 1\n",
    );
    assert!(
        inc.contains("static PyObject *pycc_ext_user_exception_classes[2];"),
        "{inc}"
    );
    assert!(
        inc.contains(
            "PYCC_EXT_MODULE_NAME_STR \".Deep\", pycc_ext_user_exception_classes[0], NULL);"
        ),
        "{inc}"
    );
    // The base is created before the subclass that names it. The source is
    // lowered for real here, so this exercises `finalize`'s program-order
    // tag assignment and `resolve_mro`'s base-before-subclass requirement
    // rather than an order a hand-built `HirClassDef` asserted into being.
    let base = inc.find("\".MyValueError\"").expect("the base entry");
    let derived = inc.find("\".Deep\"").expect("the derived entry");
    assert!(base < derived, "{inc}");
}

#[test]
fn a_class_with_two_exception_bases_is_created_from_a_tuple_of_both() {
    // Natively `except ValueError:` catches this class, because matching
    // walks the whole MRO. Taking a single base out of the linearized MRO
    // would pick `MyBase` and drop `ValueError`, so the tuple is what keeps
    // the host side in step.
    let inc = inc_from_source(
        "class MyBase(Exception):\n    pass\n\n\
         class E(MyBase, ValueError):\n    pass\n\nx = 1\n",
    );
    assert!(
        inc.contains(
            "        PyObject *bases = PyTuple_Pack(2, \
             pycc_ext_user_exception_classes[0], PyExc_ValueError);\n"
        ),
        "{inc}"
    );
    assert!(
        inc.contains("PYCC_EXT_MODULE_NAME_STR \".E\", bases, NULL);"),
        "{inc}"
    );
    assert!(inc.contains("        Py_DECREF(bases);\n"), "{inc}");
}

#[test]
fn a_group_derived_class_is_excluded_without_shifting_the_class_beside_it() {
    // The sparse-tag case, and the reason entries are keyed by explicit tag
    // rather than indexed by `tag - FIRST_USER_EXCEPTION_TYPE_TAG`: the
    // group class still *consumes* the first user tag while being excluded
    // from the table, so an array sized by entry count and indexed by that
    // offset would send `E` past its bounds, return NULL, and silently
    // flatten `E` to `Exception` with no compile or link error.
    let inc = inc_from_source(
        "class G(ExceptionGroup):\n    pass\n\nclass E(ValueError):\n    pass\n\nx = 1\n",
    );
    let group_tag = FIRST_USER_EXCEPTION_TYPE_TAG;
    let user_tag = FIRST_USER_EXCEPTION_TYPE_TAG + 1;
    assert!(
        inc.contains("static PyObject *pycc_ext_user_exception_classes[1];"),
        "{inc}"
    );
    assert!(
        inc.contains(&format!(
            "    case {user_tag}:\n        return pycc_ext_user_exception_classes[0];\n"
        )),
        "{inc}"
    );
    assert!(!inc.contains(&format!("    case {group_tag}:")), "{inc}");
    assert!(
        inc.contains("PYCC_EXT_MODULE_NAME_STR \".E\", PyExc_ValueError, NULL);"),
        "{inc}"
    );
    assert!(!inc.contains("\".G\""), "{inc}");
}

#[test]
fn a_non_exception_base_is_dropped_from_the_synthesized_class() {
    // A method-only mixin passes the attribute-layout prefix check, so
    // `class E(Mixin, ValueError)` compiles -- but it is not an exception
    // class and has no host-side counterpart, so the synthesized class
    // carries `ValueError` alone and `isinstance(e, Mixin)` diverges from
    // native pycc. `docs/RUNTIME.md` records that residual.
    let inc = inc_from_source(
        "class Mixin:\n    pass\n\nclass E(Mixin, ValueError):\n    pass\n\nx = 1\n",
    );
    assert!(
        inc.contains("PYCC_EXT_MODULE_NAME_STR \".E\", PyExc_ValueError, NULL);"),
        "{inc}"
    );
    assert!(!inc.contains("PyTuple_Pack"), "{inc}");
    assert!(!inc.contains("Mixin"), "{inc}");
}

#[test]
fn the_shims_float_tuple_unpack_is_a_strict_container_with_converting_elements() {
    // PR 4c of #1083's admission rule, read off the shim itself. The two
    // halves answer two different questions and a reviewer must be able to
    // see both: `PyTuple_Check` plus an exact-arity gate for the container
    // (D-115/D-116 leave no shape for a differently sized sequence), and
    // `PyNumber_Float` for the elements (the author wrote `float` in the
    // annotation, so CPython's own conversion is the contract -- PR 4a's
    // rule-7 paragraph).
    let shim = shim_c();
    let body = &shim[shim
        .find("int pycc_ext_obj_unpack_float_tuple(PyObject *o, long long arity, double *out)")
        .expect("the unpack helper")..];
    // Bounded at this helper's own closing brace, the way the `unpack_str`
    // test above bounds its own: the assertions below would otherwise read
    // every later helper's prose too.
    let end = body.find("\n}\n").expect("the helper's end");
    let body = &body[..end];
    let container = body.find("!PyTuple_Check(o)").expect("the container check");
    let arity = body
        .find("if (size != (Py_ssize_t)arity) {")
        .expect("the exact-arity gate");
    let convert = body
        .find("converted = PyNumber_Float(PyTuple_GetItem(o, index));")
        .expect("the element conversion");
    assert!(
        container < arity && arity < convert,
        "the container must be settled before any element is converted:\n{body}"
    );
    // `PyTuple_Check`, never `PyTuple_CheckExact`: a `tuple` subclass is a
    // tuple, and the elements are copied out by value.
    assert!(!body.contains("PyTuple_CheckExact"), "{body}");
    // The elements are converted, never type-checked: a `PyFloat_Check`
    // here would be the closed-seam rule applied to an explicit conversion.
    assert!(!body.contains("PyFloat_Check"), "{body}");
    // Each temporary is released inside the iteration that produced it, so
    // the failing exit holds nothing (#1092 stays where it is).
    let release = body.find("Py_DECREF(converted);").expect("the release");
    let store = body.find("out[index] = value;").expect("the store");
    assert!(
        convert < release && release < store,
        "the temporary must be released before the next iteration:\n{body}"
    );
    // Two distinct refusals, each naming the declared arity: a reviewer
    // reading "strict container" must find exactly the wrong-type and the
    // wrong-length message, and neither may borrow the thunk seam's
    // function-name-and-argument-index phrasing, which has no meaning at an
    // assignment.
    assert!(
        body.contains("\"expected a tuple of %zd floats, got a '%U' object\""),
        "{body}"
    );
    assert!(
        body.contains("\"expected a tuple of %zd floats, got a tuple of length %zd\""),
        "{body}"
    );
    assert!(!body.contains("() argument"), "{body}");
}

#[test]
fn the_shims_float_tuple_unpack_takes_its_arity_as_a_parameter() {
    // The admission rule is *any* fixed arity with every element `float`,
    // so no side of this seam may hard-code the three of
    // `tuple[float, float, float]`. The declaration carries `long long
    // arity`, matching the `i64` argument
    // `crates/pycc_codegen/src/foreign_len.rs` emits, and the helper's name
    // is `pycc_ext_obj_unpack_float_tuple` -- never `pycc_ext_unpack_*`,
    // whose prefix `refusal_completeness.rs` scans and whose suffix set it
    // pins exactly.
    let shim = shim_c();
    assert!(
        shim.contains(
            "int pycc_ext_obj_unpack_float_tuple(PyObject *o, long long arity, double *out)"
        ),
        "{shim}"
    );
    assert!(!shim.contains("pycc_ext_unpack_float_tuple"), "{shim}");
}

/// One export taking `count` `memoryview` parameters and returning
/// `return_ty`, as generated C.
fn memoryview_inc(name: &str, count: usize, return_ty: Ty) -> String {
    inc_no_classes(
        "m",
        &[ExtExport {
            defaults: Vec::new(),
            name: name.to_string(),
            class: None,
            method: None,
            returns_buffer_slice: false,
            receiver: ExtReceiver::None,
            params: vec![Ty::MemoryView; count],
            param_writable: vec![false; count],
            return_ty,
            keyword_names: None,
        }],
    )
}

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
             return pycc_ext_activation_exit(pycc_ext_pack_float(result));\n}\n\n"
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
             return pycc_ext_activation_exit(NULL);\n    }\n"
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
        inc.contains("    PyBuffer_Release(&b0);\n    return pycc_ext_activation_exit(Py_NewRef(Py_None));\n}\n\n"),
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
        inc.contains(
            "    PyBuffer_Release(&b1);\n    return pycc_ext_activation_exit(pycc_ext_pack_int("
        ),
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
             pycc_ext_bridge_release_to(bridge_mark);\n        return pycc_ext_activation_exit(NULL);\n    }\n    \
             pycc_ext_bridge_release_to(bridge_mark);\n    return pycc_ext_activation_exit(pycc_ext_pack_str(result));\n"
        ),
        "{inc}"
    );
}

// --- #1143: the per-class type object and its method table --------------

/// An exported `@staticmethod`, receiver-free.
pub(super) fn static_export(
    class: &str,
    method: &str,
    params: Vec<Ty>,
    return_ty: Ty,
) -> ExtExport {
    ExtExport {
        defaults: Vec::new(),
        name: format!("{class}.{method}.static"),
        class: Some(class.to_string()),
        method: Some(method.to_string()),
        returns_buffer_slice: false,
        receiver: ExtReceiver::None,
        param_writable: vec![false; params.len()],
        params,
        return_ty,
        keyword_names: None,
    }
}

/// An exported `@classmethod`. `params` is the receiver-free tail: `cls`
/// never crosses the boundary.
fn class_export(class: &str, method: &str, params: Vec<Ty>, return_ty: Ty) -> ExtExport {
    ExtExport {
        defaults: Vec::new(),
        name: format!("{class}.{method}.classmethod"),
        class: Some(class.to_string()),
        method: Some(method.to_string()),
        returns_buffer_slice: false,
        receiver: ExtReceiver::NullCls,
        param_writable: vec![false; params.len()],
        params,
        return_ty,
        keyword_names: None,
    }
}

#[test]
fn a_program_with_no_exported_method_still_defines_the_registration_function() {
    // Unconditional, so `src/ext/pycc_ext_module.c` can call it without a
    // preprocessor guard: an artifact whose program exports no method still
    // links.
    let inc = inc_no_classes("m", &[]);
    assert!(inc.contains(METHOD_TYPE_REGISTER_DECL), "{inc}");
    assert!(
        inc.contains("    (void)module;\n    return 0;\n}\n"),
        "{inc}"
    );
    assert!(!inc.contains("PyType_FromSpec"), "{inc}");
    // Part 7 of #1371: the compiled-class `isinstance` is defined too, and
    // with nothing published every class name falls through to the shim's
    // unpublished-class answer.
    assert!(
        inc.contains(&format!(
            "{COMPILED_CLASS_ISINSTANCE_DECL}\n{{\n    (void)name;\n    \
             return {UNPUBLISHED_CLASS_ISINSTANCE}(o);\n}}\n"
        )),
        "{inc}"
    );
    assert!(!inc.contains("pycc_ext_type_object_"), "{inc}");
}

#[test]
fn an_exported_method_gets_a_method_table_a_slot_table_and_a_non_instantiable_spec() {
    let inc = inc_no_classes(
        "m",
        &[
            static_export("Grid", "scale", vec![Ty::Int], Ty::Int),
            class_export("Grid", "make", vec![Ty::Int], Ty::Int),
        ],
    );
    assert!(
        inc.contains(
            "static PyMethodDef pycc_ext_type_methods_Grid[] = {\n    \
             {\"scale\", (PyCFunction)(void (*)(void))pycc_ext_wrap_0m4_Grid5_scale6_static, \
             METH_FASTCALL | METH_STATIC, NULL},\n    \
             {\"make\", (PyCFunction)(void (*)(void))pycc_ext_wrap_0m4_Grid4_make11_classmethod, \
             METH_FASTCALL | METH_CLASS, NULL},\n    \
             {\"__copy__\", (PyCFunction)(void (*)(void))pycc_ext_instance_copy, \
             METH_NOARGS, NULL},\n    {NULL, NULL, 0, NULL},\n};\n"
        ),
        "{inc}"
    );
    // Every published type is a carrier type (#1435), so even one with no
    // constructor carries the shared deallocator.
    assert!(
        inc.contains(
            "static PyType_Slot pycc_ext_type_slots_Grid[] = {\n    \
             {Py_tp_methods, pycc_ext_type_methods_Grid},\n    \
             {Py_tp_dealloc, pycc_ext_instance_dealloc},\n    {0, NULL},\n};\n"
        ),
        "{inc}"
    );
    // `basicsize` is the carrier's, because an instance crossing as a call
    // argument is boxed in this type (#1435); `Py_TPFLAGS_DISALLOW_INSTANTIATION`
    // plus `Py_TPFLAGS_IMMUTABLETYPE` are what make `mod.Grid()` and
    // `mod.Grid.scale = ...` both `TypeError`.
    assert!(
        inc.contains(
            "static PyType_Spec pycc_ext_type_spec_Grid = {\n    \
             PYCC_EXT_MODULE_NAME_STR \".Grid\",\n    sizeof(PyccExtInstance),\n    0,\n    \
             Py_TPFLAGS_DEFAULT | Py_TPFLAGS_DISALLOW_INSTANTIATION | \
             Py_TPFLAGS_IMMUTABLETYPE,\n    pycc_ext_type_slots_Grid,\n};\n"
        ),
        "{inc}"
    );
    // The type enters the shim's carrier-type cache, and a failure releases
    // the local reference. It is not added to the module here (#1199):
    // `pycc_ext_publish` binds it from its static once its class statement
    // has run.
    assert!(
        inc.contains(
            "    type = PyType_FromSpec(&pycc_ext_type_spec_Grid);\n    \
             if (type == NULL) {\n        return -1;\n    }\n    \
             if (pycc_ext_carrier_register(\"Grid\", type) < 0) {\n        \
             Py_DECREF(type);\n        return -1;\n    }\n    \
             Py_XDECREF(pycc_ext_type_object_Grid);\n    pycc_ext_type_object_Grid = type;\n"
        ),
        "{inc}"
    );
}

#[test]
fn an_exported_method_is_never_a_flat_module_level_entry() {
    // The host surface is `mod.Grid.scale(...)`. A flat
    // `mod."Grid.scale"` attribute is not published, in this release or any
    // later one: `pycc_ext_methods[]` carries module-level functions only.
    let inc = inc_no_classes(
        "m",
        &[
            ExtExport {
                defaults: Vec::new(),
                name: "plain".to_string(),
                class: None,
                method: None,
                returns_buffer_slice: false,
                receiver: ExtReceiver::None,
                params: vec![Ty::Int],
                param_writable: vec![false; 1],
                return_ty: Ty::Int,
                keyword_names: None,
            },
            static_export("Grid", "scale", vec![Ty::Int], Ty::Int),
        ],
    );
    let table = inc
        .split("static PyMethodDef pycc_ext_methods[]")
        .nth(1)
        .expect("the module-level table is emitted")
        .split("};")
        .next()
        .expect("the table is terminated");
    assert!(table.contains("\"plain\""), "{table}");
    assert!(!table.contains("Grid"), "{table}");
}

#[test]
fn a_non_ascii_class_name_is_spliced_verbatim_into_the_type_object_identifiers() {
    // `mangle_ext_name` encodes the dot separator; it is not an ASCII fold,
    // and it copies each segment's bytes verbatim. Routing a class name
    // through it would therefore change nothing here, and nothing needs to:
    // a Python identifier is `XID_Start XID_Continue*`, which carries no
    // character that escapes a C identifier or a C string literal, and both
    // clang and GCC accept UTF-8 identifiers. Verified end to end at
    // `5b1fb3a1`: a module whose class is named `Grid\u{e9}` builds with
    // `--ext`, imports, and answers `Grid\u{e9}.scale(21) == 42`.
    let inc = inc_no_classes(
        "m",
        &[static_export("Grid\u{e9}", "scale", vec![Ty::Int], Ty::Int)],
    );
    assert!(
        inc.contains("static PyMethodDef pycc_ext_type_methods_Grid\u{e9}[]"),
        "{inc}"
    );
    assert!(
        inc.contains("static PyType_Spec pycc_ext_type_spec_Grid\u{e9} = {"),
        "{inc}"
    );
    // The host-visible name is the same bytes, in a string literal.
    assert!(
        inc.contains("PYCC_EXT_MODULE_NAME_STR \".Grid\u{e9}\""),
        "{inc}"
    );
}

#[test]
fn a_class_method_wrapper_prepends_the_null_receiver_and_a_static_one_does_not() {
    let inc = inc_no_classes(
        "m",
        &[
            static_export("Grid", "scale", vec![Ty::Int], Ty::Int),
            class_export("Grid", "make", vec![Ty::Int], Ty::Int),
        ],
    );
    // The `@staticmethod` thunk takes exactly the carried parameters.
    assert!(
        inc.contains("extern void *fnptr_0m4_Grid5_scale6_static;"),
        "{inc}"
    );
    assert!(
        inc.contains("extern void *fnptr_0m4_Grid4_make11_classmethod;"),
        "{inc}"
    );
    // The `@classmethod` thunk's native signature still leads with the
    // receiver slot, which every native `Grid.make(...)` call site fills
    // with a null pointer; the wrapper passes `NULL` there and the compiled
    // body never dereferences it.
    let make = thunk_call_line(&inc, "fnptr_0m4_Grid4_make11_classmethod");
    assert!(make.contains("void *,"), "{make}");
    assert!(make.contains("NULL,"), "{make}");
    // The `@staticmethod`'s own cast carries no receiver slot. Anchor on the
    // cast line rather than on the wrapper's first statement: the body opens
    // with `(void)self;`, so a negative assertion cut at the first `;` would
    // hold whatever the cast below it said.
    let scale = thunk_call_line(&inc, "fnptr_0m4_Grid5_scale6_static");
    assert!(!scale.contains("void *,"), "{scale}");
}

/// The single line of generated C that casts `symbol` to its native
/// signature and calls it -- the text that decides whether a wrapper
/// prepends a receiver.
fn thunk_call_line<'a>(inc: &'a str, symbol: &str) -> &'a str {
    inc.lines()
        .find(|line| line.contains(&format!(")){symbol})")))
        .unwrap_or_else(|| panic!("no call through `{symbol}` in:\n{inc}"))
}

#[test]
fn a_nullary_class_method_declares_the_receiver_as_its_whole_parameter_list() {
    // `c_param_list` answers `"void"` for an empty carried list, because an
    // empty C parameter list means "unspecified" rather than "no
    // arguments". A `@classmethod` whose receiver-free tail is empty still
    // takes the receiver slot natively, so the declaration must read
    // `(void *)` -- never `(void *, void)`, which does not compile, and
    // never `(void)`, which would disagree with the compiled arity.
    let inc = inc_no_classes("m", &[class_export("Grid", "make", vec![], Ty::Int)]);
    let decl = inc
        .split("extern void *fnptr_0m4_Grid4_make11_classmethod;")
        .nth(1)
        .expect("the classmethod's function-pointer global is emitted");
    assert!(!decl.contains("void *, void"), "{decl}");
    let call = inc
        .split("pycc_ext_wrap_0m4_Grid4_make11_classmethod(")
        .nth(1)
        .expect("the classmethod wrapper is emitted");
    assert!(
        call.contains("(void *))fnptr_0m4_Grid4_make11_classmethod"),
        "{call}"
    );
    assert!(call.contains("(NULL)"), "{call}");
}

#[test]
fn a_method_wrapper_renders_the_source_level_spelling_in_every_host_visible_message() {
    // A message the host reads must never carry a spelling the user did not
    // write: the arity error says `Grid.scale()`, not
    // `Grid.scale.static()` and not `0m4_Grid5_scale6_static()`.
    let inc = inc_no_classes(
        "m",
        &[static_export("Grid", "scale", vec![Ty::Int], Ty::Int)],
    );
    assert!(inc.contains("Grid.scale() takes exactly"), "{inc}");
    assert!(!inc.contains("Grid.scale.static()"), "{inc}");
    assert!(!inc.contains("0m4_Grid5_scale6_static()"), "{inc}");
}

#[test]
fn two_classes_are_emitted_in_first_export_order() {
    // The companion must be byte-identical across runs, so the class order
    // is the export order and never a hash-map walk.
    let inc = inc_no_classes(
        "m",
        &[
            static_export("Zeta", "one", vec![], Ty::Int),
            static_export("Alpha", "two", vec![], Ty::Int),
            static_export("Zeta", "three", vec![], Ty::Int),
        ],
    );
    let zeta = inc.find("pycc_ext_type_methods_Zeta").expect("Zeta");
    let alpha = inc.find("pycc_ext_type_methods_Alpha").expect("Alpha");
    assert!(zeta < alpha, "{inc}");
    let table = inc
        .split("static PyMethodDef pycc_ext_type_methods_Zeta[] = {")
        .nth(1)
        .expect("Zeta's table")
        .split("};")
        .next()
        .expect("terminated");
    assert!(table.contains("\"one\""), "{table}");
    assert!(table.contains("\"three\""), "{table}");
    assert!(!table.contains("\"two\""), "{table}");
}

#[test]
fn a_tuple_carrying_exported_class_method_goes_through_its_thunk() {
    // A `tuple` in the signature routes the call through
    // `pycc_ext_thunk_<symbol>` (#1050) rather than the `fnptr_` global
    // directly, and the mangled symbol travels into that spelling too.
    let inc = inc_no_classes(
        "m",
        &[class_export(
            "Grid",
            "pair",
            vec![Ty::Int],
            Ty::Tuple(Box::new(vec![Ty::Int, Ty::Int])),
        )],
    );
    assert!(
        inc.contains("pycc_ext_thunk_0m4_Grid4_pair11_classmethod"),
        "{inc}"
    );
}

// --- #1145: the constructible type object and its `tp_init` --------------

/// An exported instance method. `params` is the receiver-free tail, exactly
/// as `collect_exports` hands it over.
fn instance_export(class: &str, method: &str, params: Vec<Ty>, return_ty: Ty) -> ExtExport {
    ExtExport {
        defaults: Vec::new(),
        name: format!("{class}.{method}"),
        class: Some(class.to_string()),
        method: Some(method.to_string()),
        returns_buffer_slice: false,
        receiver: ExtReceiver::SelfInstance,
        param_writable: vec![false; params.len()],
        params,
        return_ty,
        keyword_names: None,
    }
}

fn grid_ctor(params: Vec<Ty>, slot_names: &[&str]) -> ExtCtor {
    ExtCtor {
        defaults: Vec::new(),
        class: "Grid".to_string(),
        name: "Grid.__init__".to_string(),
        param_writable: vec![false; params.len()],
        params,
        slot_names: slot_names.iter().map(ToString::to_string).collect(),
        keyword_names: None,
    }
}

#[test]
fn an_exported_instance_method_is_a_plain_fastcall_row_that_unwraps_its_receiver() {
    let inc = inc_no_classes("m", &[instance_export("Grid", "area", vec![], Ty::Int)]);
    // Neither `METH_STATIC` nor `METH_CLASS`: a plain `METH_FASTCALL` row is
    // what makes CPython deliver the instance in `self` -- and, through its
    // method-descriptor machinery, type-check that instance before the
    // wrapper is entered.
    assert!(
        inc.contains(
            "    {\"area\", (PyCFunction)(void (*)(void))pycc_ext_wrap_0m4_Grid4_area, \
             METH_FASTCALL, NULL},\n"
        ),
        "{inc}"
    );
    // The NULL guard is reachable, not defensive: `mod.Grid.__new__(mod.Grid)`
    // runs `PyType_GenericNew` and never `tp_init`.
    assert!(
        inc.contains(
            "    void *self_inst = ((PyccExtInstance *)self)->inst;\n    \
             if (self_inst == NULL) {\n        PyErr_SetString(PyExc_TypeError, \
             \"Grid.area() called on an uninitialized instance\");\n        \
             return NULL;\n    }\n"
        ),
        "{inc}"
    );
    // The receiver reaches the compiled call as the real pointer, where a
    // `@classmethod`'s `cls` reaches it as `NULL`.
    assert!(inc.contains("fnptr_0m4_Grid4_area)(self_inst)"), "{inc}");
    // No flat module attribute: the module's own table stays receiver-free.
    assert!(!inc.contains("{\"Grid.area\""), "{inc}");
}

#[test]
fn a_constructible_class_gets_a_tp_init_three_slots_and_a_carrier_sized_spec() {
    let inc = generate_exports_inc(
        "m",
        &[instance_export("Grid", "area", vec![], Ty::Int)],
        &[],
        &flat_publications(&[instance_export("Grid", "area", vec![], Ty::Int)]),
        &[grid_ctor(vec![Ty::Int, Ty::Int], &["w", "h"])],
        &[],
    );
    assert!(
        inc.contains(
            "extern void *fnptr_0m4_Grid8___init__;\n\
             static int pycc_ext_tp_init_Grid(PyObject *self, PyObject *args, PyObject *kwds)\n\
             {\n    void *inst;\n    long long a0;\n    long long a1;\n"
        ),
        "{inc}"
    );
    // D-244 rule 7's keyword boundary for a constructor that is not
    // keyword-enabled (#1461; this fixture's `keyword_names` is `None`),
    // written out by hand because a `tp_init` is not a `METH_FASTCALL`
    // entry point and CPython refuses nothing on its behalf. Without this, a
    // correct positional count plus a keyword would silently ignore the
    // keyword.
    assert!(
        inc.contains(
            "    if (kwds != NULL && PyDict_Size(kwds) != 0) {\n        \
             PyErr_SetString(PyExc_TypeError, \
             \"Grid.__init__() takes no keyword arguments\");\n        return -1;\n    }\n"
        ),
        "{inc}"
    );
    assert!(
        inc.contains(
            "    if (PyTuple_Size(args) != 2) {\n        PyErr_Format(PyExc_TypeError, \
             \"Grid.__init__() takes exactly 2 arguments (%zd given)\", \
             PyTuple_Size(args));\n        return -1;\n    }\n"
        ),
        "{inc}"
    );
    // The same `pycc_ext_unpack_*` helper an ordinary export uses for a
    // declared `int`, bridged through `PyTuple_GetItem` because `tp_init`
    // receives a tuple rather than a `METH_FASTCALL` vector. A
    // `PyArg_ParseTuple` shortcut here would give `mod.Grid(True, 4)` and
    // `mod.Grid(3, 4).area()` two different admissibility matrices for one
    // declared type.
    assert!(
        inc.contains(
            "    if (pycc_ext_unpack_int(PyTuple_GetItem(args, 0), \"Grid.__init__\", 0, &a0) \
             != 0) {\n        return -1;\n    }\n    \
             if (pycc_ext_unpack_int(PyTuple_GetItem(args, 1), \"Grid.__init__\", 1, &a1) \
             != 0) {\n        return -1;\n    }\n"
        ),
        "{inc}"
    );
    // The slot count is `flat_attr_layout`'s, not the argument count, and
    // the compiled constructor is called through its `fnptr_` slot with the
    // instance first -- the same shape `MirExpr::Instantiate` emits.
    assert!(
        inc.contains(
            "    inst = pycc_rt_instance_new(2, \"Grid\\000w\\000h\", 8);\n    \
             Py_ssize_t bridge_mark = pycc_ext_bridge_mark();\n    \
             ((void (*)(void *, long long, long long))fnptr_0m4_Grid8___init__)\
             (inst, a0, a1);\n"
        ),
        "{inc}"
    );
    // A constructor that raised must not publish its carrier: the pending
    // flag is read before the store, exactly as `wrapper_for` reads it
    // before packing a result.
    assert!(
        inc.contains(
            "    if (pycc_rt_ext_pending_type() >= 0) {\n        \
             pycc_ext_raise_pending();\n        pycc_ext_bridge_release_to(bridge_mark);\n        \
             return pycc_ext_activation_exit_status(-1);\n    }\n    pycc_ext_bridge_release_to(bridge_mark);\n    \
             pycc_ext_carrier_bind(self, inst);\n    return pycc_ext_activation_exit_status(0);\n}\n"
        ),
        "{inc}"
    );
    super::bridge_watermark::assert_every_exit_after_the_mark_counts_out(&inc);
    assert!(
        inc.contains(
            "static PyType_Slot pycc_ext_type_slots_Grid[] = {\n    \
             {Py_tp_methods, pycc_ext_type_methods_Grid},\n    \
             {Py_tp_new, PyType_GenericNew},\n    \
             {Py_tp_init, pycc_ext_tp_init_Grid},\n    \
             {Py_tp_dealloc, pycc_ext_instance_dealloc},\n    {0, NULL},\n};\n"
        ),
        "{inc}"
    );
    // `DISALLOW_INSTANTIATION` is gone, while `IMMUTABLETYPE` stays and `BASETYPE` is still absent: a host may
    // build an instance but may neither rebind the type's attributes nor
    // subclass it.
    assert!(
        inc.contains(
            "static PyType_Spec pycc_ext_type_spec_Grid = {\n    \
             PYCC_EXT_MODULE_NAME_STR \".Grid\",\n    sizeof(PyccExtInstance),\n    0,\n    \
             Py_TPFLAGS_DEFAULT | Py_TPFLAGS_IMMUTABLETYPE,\n    \
             pycc_ext_type_slots_Grid,\n};\n"
        ),
        "{inc}"
    );
    assert!(!inc.contains("Py_TPFLAGS_BASETYPE"), "{inc}");
}

#[test]
fn a_zero_argument_constructor_declares_a_receiver_only_parameter_list() {
    // `c_param_list` answers `"void"` for an empty carried list, which would
    // declare `void (*)(void, void *)` if it were pasted after the receiver.
    // The receiver-only spelling is `void *` alone.
    let inc = generate_exports_inc(
        "m",
        &[instance_export("Grid", "area", vec![], Ty::Int)],
        &[],
        &flat_publications(&[instance_export("Grid", "area", vec![], Ty::Int)]),
        &[grid_ctor(Vec::new(), &[])],
        &[],
    );
    assert!(
        inc.contains(
            "    if (PyTuple_Size(args) != 0) {\n        PyErr_Format(PyExc_TypeError, \
             \"Grid.__init__() takes exactly 0 arguments (%zd given)\", \
             PyTuple_Size(args));\n        return -1;\n    }\n"
        ),
        "{inc}"
    );
    assert!(
        inc.contains(
            "    inst = pycc_rt_instance_new(0, \"Grid\", 4);\n    \
             Py_ssize_t bridge_mark = pycc_ext_bridge_mark();\n    \
             ((void (*)(void *))fnptr_0m4_Grid8___init__)(inst);\n"
        ),
        "{inc}"
    );
}

#[test]
fn a_one_argument_constructor_says_argument_in_the_singular() {
    let inc = generate_exports_inc(
        "m",
        &[instance_export("Grid", "area", vec![], Ty::Int)],
        &[],
        &flat_publications(&[instance_export("Grid", "area", vec![], Ty::Int)]),
        &[grid_ctor(vec![Ty::Int], &["w"])],
        &[],
    );
    assert!(
        inc.contains("takes exactly 1 argument (%zd given)"),
        "{inc}"
    );
}

#[test]
fn a_memoryview_constructor_releases_its_buffer_on_every_exit_past_the_acquire() {
    // `memoryview` is admitted at a parameter position, so it is admitted at
    // a constructor's too -- and a `tp_init` owes the same release discipline
    // a wrapper does, on the raising arm as well as the falling-through one.
    let inc = generate_exports_inc(
        "m",
        &[instance_export("Grid", "area", vec![], Ty::Int)],
        &[],
        &flat_publications(&[instance_export("Grid", "area", vec![], Ty::Int)]),
        &[grid_ctor(vec![Ty::MemoryView], &["w"])],
        &[],
    );
    assert!(
        inc.contains("    Py_buffer b0;\n    PyccExtBufferView a0;\n"),
        "{inc}"
    );
    assert!(
        inc.contains(
            "    if (pycc_ext_unpack_memoryview(PyTuple_GetItem(args, 0), \"Grid.__init__\", \
             0, 0, &b0) != 0) {\n        return -1;\n    }\n    a0.ptr = b0.buf;\n    \
             a0.len = (long long)b0.shape[0];\n"
        ),
        "{inc}"
    );
    assert!(
        inc.contains(
            "    if (pycc_rt_ext_pending_type() >= 0) {\n        PyBuffer_Release(&b0);\n        \
             pycc_ext_raise_pending();\n        pycc_ext_bridge_release_to(bridge_mark);\n        \
             return pycc_ext_activation_exit_status(-1);\n    }\n    pycc_ext_bridge_release_to(bridge_mark);\n    \
             PyBuffer_Release(&b0);\n"
        ),
        "{inc}"
    );
}

#[test]
fn a_published_class_with_no_constructor_descriptor_is_a_non_instantiable_carrier() {
    // The scope line: publication is decided by `publications` and
    // constructibility by `ctors`, so a class absent from `ctors` gets no
    // `tp_init` and keeps `DISALLOW_INSTANTIATION` -- but since #1435 it is
    // still a carrier type, with the carrier's `basicsize` and the shared
    // deallocator, because its instances can cross as call arguments.
    let exports = [instance_export("Grid", "area", vec![], Ty::Int)];
    let without = generate_exports_inc("m", &exports, &[], &flat_publications(&exports), &[], &[]);
    assert!(!without.contains("pycc_ext_tp_init_Grid"), "{without}");
    assert!(
        without.contains(
            "static PyType_Slot pycc_ext_type_slots_Grid[] = {\n    \
             {Py_tp_methods, pycc_ext_type_methods_Grid},\n    \
             {Py_tp_dealloc, pycc_ext_instance_dealloc},\n    {0, NULL},\n};\n"
        ),
        "{without}"
    );
    assert!(!without.contains("Py_tp_new"), "{without}");
    assert!(
        without.contains(
            "static PyType_Spec pycc_ext_type_spec_Grid = {\n    \
             PYCC_EXT_MODULE_NAME_STR \".Grid\",\n    sizeof(PyccExtInstance),\n    0,\n    \
             Py_TPFLAGS_DEFAULT | Py_TPFLAGS_DISALLOW_INSTANTIATION | \
             Py_TPFLAGS_IMMUTABLETYPE,\n    pycc_ext_type_slots_Grid,\n};\n"
        ),
        "{without}"
    );
}

#[test]
fn a_constructor_descriptor_for_an_unpublished_class_emits_nothing() {
    // The class list is `publications` alone, so a constructor descriptor
    // whose class publishes no method must not conjure a type object -- the
    // two sets are deliberately not the same set.
    let inc = generate_exports_inc("m", &[], &[], &[], &[grid_ctor(vec![Ty::Int], &["w"])], &[]);
    assert!(!inc.contains("pycc_ext_tp_init_Grid"), "{inc}");
    assert!(!inc.contains("PyType_FromSpec"), "{inc}");
}

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
        inc.contains(
            "    return pycc_ext_activation_exit(pycc_ext_pack_memoryview(result));\n}\n\n"
        ),
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
            "    PyBuffer_Release(&b0);\n    if (caller_owned) {\n        return pycc_ext_activation_exit(borrowed);\n    \
             }\n    return pycc_ext_activation_exit(pycc_ext_pack_memoryview(result));\n}\n\n"
        ),
        "{inc}"
    );
    // Exactly two releases: the two exits reachable with the buffer held.
    assert_eq!(inc.matches("PyBuffer_Release(&b0);").count(), 2, "{inc}");
    super::bridge_watermark::assert_every_exit_after_the_mark_counts_out(&inc);
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
        inc.contains("    PyBuffer_Release(&b0);\n    return pycc_ext_activation_exit(pycc_ext_pack_float(result));\n"),
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
            "        pycc_ext_bridge_release_to(bridge_mark);\n        return pycc_ext_activation_exit(NULL);\n    }\n",
            "    pycc_ext_bridge_release_to(bridge_mark);\n",
            "    return pycc_ext_activation_exit(pycc_ext_pack_memoryview(result));\n",
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
        inc.contains("    return pycc_ext_activation_exit(pycc_ext_pack_memoryview(result));\n"),
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
/// `pycc_ext_unpack_object` and packed by `pycc_ext_pack_object` -- and the
/// shim's two helpers take a new reference in both directions. Not
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
        "return pycc_ext_activation_exit(pycc_ext_pack_object(result));",
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
            "return Py_NewRef((PyObject *)result);",
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
