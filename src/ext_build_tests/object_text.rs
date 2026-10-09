//! The shim's two text renderings of a CPython object: `str(o)`
//! (`pycc_ext_obj_to_str`, PR 4b of #1083) and an f-string's
//! `format(o, '')` (`pycc_ext_obj_format`, #1340).
//!
//! Both copy the UTF-8 out of a *new* CPython reference whose buffer
//! `PyUnicode_AsUTF8AndSize` points into, so the copy must complete before
//! the release -- the reverse order is a use-after-free no hosted test
//! would reliably catch. These tests pin that order in the source text.

use super::*;

/// The body of the shim helper `signature` opens, up to its closing brace.
fn helper_body(signature: &str) -> String {
    let shim = shim_c();
    let start = shim
        .find(signature)
        .unwrap_or_else(|| panic!("no `{signature}` in the shim"));
    let body = &shim[start..];
    let end = body.find("\n}\n").expect("the helper's end");
    body[..end].to_string()
}

/// Asserts that, in `body`, `conversion` runs first, then the copy, then
/// the release of `temporary`, and that the failure path of the UTF-8 read
/// releases `temporary` too.
fn assert_copies_before_releasing(body: &str, conversion: &str, temporary: &str) {
    let convert = body
        .find(conversion)
        .unwrap_or_else(|| panic!("no `{conversion}`:\n{body}"));
    let copy = body
        .find("copied = pycc_rt_str_from_literal((const unsigned char *)utf8, (long long)size);")
        .unwrap_or_else(|| panic!("no copy:\n{body}"));
    let release = body
        .rfind(&format!("Py_DECREF({temporary});"))
        .unwrap_or_else(|| panic!("no release:\n{body}"));
    assert!(
        convert < copy && copy < release,
        "the copy must complete before the release:\n{body}"
    );
    assert!(
        body.contains(&format!(
            "if (utf8 == NULL) {{\n        Py_DECREF({temporary});\n        return -1;\n    }}"
        )),
        "the failing UTF-8 read must release the temporary too:\n{body}"
    );
    assert!(!body.contains("strlen"), "{body}");
}

#[test]
fn the_str_helper_copies_before_releasing() {
    let body = helper_body("int pycc_ext_obj_to_str(PyObject *o, void **out)");
    assert_copies_before_releasing(&body, "converted = PyObject_Str(o);", "converted");
}

/// `f"{o}"` is `format(o, '')`, so the helper passes a NULL spec -- the
/// empty spec -- to `PyObject_Format` and never calls `PyObject_Str`.
#[test]
fn the_format_helper_uses_an_empty_spec_and_copies_before_releasing() {
    let body = helper_body("int pycc_ext_obj_format(PyObject *o, void **out)");
    assert_copies_before_releasing(&body, "formatted = PyObject_Format(o, NULL);", "formatted");
    assert!(!body.contains("PyObject_Str("), "{body}");
}
