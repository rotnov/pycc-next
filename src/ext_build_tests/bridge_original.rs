//! #1316: a bridged pycc exception carries its CPython original.
//!
//! A compiled function body bridges a failed foreign operation into a pycc
//! exception. The original CPython exception travels on that exception
//! object (`pycc_rt_exception_set_foreign`), which `pycc_rt` releases with
//! it, so a caught one is freed when its handler finishes and an escaping
//! one is re-raised as the original. These tests pin that the shim stores
//! and re-raises the original that way, and that no frame keeps a
//! per-thread table of originals any more.

use super::*;

fn plain_export(name: &str) -> ExtExport {
    ExtExport {
        defaults: Vec::new(),
        name: name.to_string(),
        class: None,
        method: None,
        returns_buffer_slice: false,
        receiver: ExtReceiver::None,
        params: vec![Ty::Int],
        param_writable: vec![false],
        return_ty: Ty::Int,
        keyword_names: None,
    }
}

#[test]
fn a_wrapper_raises_the_pending_exception_before_packing() {
    let exports = [plain_export("f")];
    let inc = generate_exports_inc("m", &exports, &[], &[], &[], &[]);
    assert!(
        inc.contains(
            "    if (pycc_rt_ext_pending_type() >= 0) {\n        pycc_ext_raise_pending();\n        \
             return NULL;\n    }\n    return pycc_ext_pack_int(\"f\", result);\n"
        ),
        "{inc}"
    );
    assert!(!inc.contains("bridge_mark"), "{inc}");
}

#[test]
fn the_bridge_stores_the_original_on_the_pycc_exception() {
    let shim = shim_c();
    assert!(!shim.contains("pycc_ext_bridge_table"));
    assert!(!shim.contains("pycc_ext_bridge_mark"));
    let store = shim
        .find("static int pycc_ext_bridge_store(")
        .expect("the shared bridge");
    let body = &shim[store..store + shim[store..].find("\n}\n").expect("its end")];
    assert!(
        body.contains(
            "    pycc_rt_exception_set_foreign(obj, exc);\n    pycc_rt_exception_raise(obj);\n    \
             return 1;"
        ),
        "{body}"
    );
}

#[test]
fn an_escaping_bridged_exception_is_re_raised_as_its_original() {
    let shim = shim_c();
    assert!(shim.contains(
        "    original = pycc_rt_exception_foreign(pycc_rt_exception_value());\n    \
         if (original != NULL) {\n        Py_INCREF(original);\n        \
         PyErr_SetRaisedException(original);\n        pycc_rt_exception_clear();\n        \
         return 1;\n    }\n"
    ));
    let include = SHIM_C
        .find("#include \"pycc_ext_exports.inc\"")
        .expect("the shim splices the generated companion in");
    let at = SHIM_C
        .find("static int pycc_ext_raise_pending(void)")
        .expect("the helper");
    assert!(
        at < include,
        "the helper must precede the generated include"
    );
}
