//! The embedded C shim's contract with the generated companion.

use super::*;

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
    assert!(
        shim_c().contains(&format!("{USER_EXCEPTION_MATCH_DECL};")),
        "{USER_EXCEPTION_MATCH_DECL}"
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
fn the_shims_bigint_egress_packs_the_value_then_releases_the_reference() {
    // D-180 rule 6 hands the wrapper a retained return value: the bigint arm
    // builds the CPython `int` from the word, then drops the reference.
    assert!(SHIM_C.contains("extern void pycc_rt_bigint_release(long long word);"));
    let body = &SHIM_C[SHIM_C
        .find("static PyObject *pycc_ext_pack_int(")
        .expect("the pack helper")..];
    let bigint_arm = body.find("case PYCC_EXT_INT_BIGINT:").expect("bigint arm");
    let pack = body
        .find("pycc_ext_bigint_pack(encoded);")
        .expect("the pack");
    let release = body
        .find("pycc_rt_bigint_release(encoded);")
        .expect("the release");
    assert!(
        bigint_arm < pack && pack < release,
        "the value must be packed before its reference is dropped"
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
        .find("return pycc_ext_long_encode(obj, out);")
        .expect("conversion");
    assert!(
        bool_check < long_check && long_check < convert,
        "D-141's bool markers must be read before the int conversion flattens them"
    );
    // Any magnitude crosses (#1040): a value beyond `long long` travels as
    // hex text into a heap bigint.
    assert!(SHIM_C.contains("pycc_rt_ext_int_from_hex(text, (size_t)len)"));
}
