//! The method part of #1140 at the `--ext` host boundary: a method's
//! default parameter values reach the generated `METH_FASTCALL` wrapper and
//! `Py_tp_init`, which then accept the shorter argument counts CPython
//! does. Pinned on the companion generated from real lowered source, so the
//! defaults are the ones `pycc_hir` really records.

use super::*;

/// Parses, lowers and type-checks `source`, returning the resolved module.
fn resolved(source: &str) -> HirModule {
    let module = pycc_parser::parse(source).expect("test fixture must parse");
    let hir = pycc_hir::lower_checked(&module).expect("test fixture must lower");
    pycc_types::check_and_resolve(&hir).expect("test fixture must check")
}

/// The generated companion for `source`'s exports and constructors.
fn companion(source: &str) -> String {
    let module = resolved(source);
    let exports = collect_exports(&module).expect("the module exports");
    let publications = collect_class_publications(&module, &exports);
    let ctors = collect_constructors(&module, &publications);
    generate_exports_inc("m", &exports, &[], &publications, &ctors, &[])
}

const MODULE: &str = "class S:\n\
    \x20   def __init__(self, n: int, label: str = 'x') -> None:\n\
    \x20       self.n = n\n        self.label = label\n\
    \x20   def feed(self, token: int, is_end: bool = False) -> int:\n\
    \x20       return token\n\
    \x20   @staticmethod\n    def make(x: int = 3) -> int:\n        return x\n\
    \x20   @classmethod\n    def scaled(cls, f: float = -1.5) -> float:\n        return f\n\
    \x20   def plain(self, a: int) -> int:\n        return a\n";

#[test]
fn the_carried_defaults_drop_the_receiver_entry() {
    let module = resolved(MODULE);
    let exports = collect_exports(&module).expect("the module exports");
    let defaults = |name: &str| {
        exports
            .iter()
            .find(|export| export.name == name)
            .unwrap_or_else(|| panic!("`{name}` is exported"))
            .defaults
            .clone()
    };
    assert_eq!(
        defaults("S.feed"),
        vec![None, Some(pycc_hir::HirExpr::BoolLiteral(false))]
    );
    // A `@staticmethod` has no receiver to drop.
    assert_eq!(
        defaults("S.make.static"),
        vec![Some(pycc_hir::HirExpr::IntLiteral(3))]
    );
    assert_eq!(
        defaults("S.scaled.classmethod"),
        vec![Some(pycc_hir::HirExpr::FloatLiteral(-1.5))]
    );
    assert!(defaults("S.plain").is_empty());
    let ctors = collect_constructors(&module, &collect_class_publications(&module, &exports));
    assert_eq!(
        ctors[0].defaults,
        vec![
            None,
            Some(pycc_hir::HirExpr::StringLiteral("x".to_string()))
        ]
    );
}

#[test]
fn a_defaulted_method_wrapper_accepts_the_shorter_argument_counts() {
    let inc = companion(MODULE);
    assert!(
        inc.contains(
            "    if (nargs < 1 || nargs > 2) {\n        PyErr_Format(PyExc_TypeError, \
             \"S.feed() takes from 1 to 2 arguments (%zd given)\", nargs);\n"
        ),
        "{inc}"
    );
    assert!(
        inc.contains("    PyObject *v1 = nargs > 1 ? args[1] : Py_False;\n"),
        "{inc}"
    );
    // The defaulted argument goes through the parameter's own unchanged
    // unpack helper.
    assert!(inc.contains("pycc_ext_unpack_bool(v1, "), "{inc}");
    // A `@staticmethod` with every parameter defaulted accepts none.
    assert!(inc.contains("if (nargs < 0 || nargs > 1)"), "{inc}");
    assert!(
        inc.contains("static PyObject *pycc_ext_default_wrap_"),
        "{inc}"
    );
    assert!(inc.contains("PyLong_FromLongLong(3LL)"), "{inc}");
    assert!(inc.contains("PyFloat_FromDouble(-1.5e0)"), "{inc}");
}

#[test]
fn a_defaulted_constructor_accepts_the_shorter_argument_counts() {
    let inc = companion(MODULE);
    assert!(
        inc.contains(
            "    if (PyTuple_Size(args) < 1 || PyTuple_Size(args) > 2) {\n        \
             PyErr_Format(PyExc_TypeError, \"S.__init__() takes from 1 to 2 arguments \
             (%zd given)\", PyTuple_Size(args));\n        return -1;\n    }\n"
        ),
        "{inc}"
    );
    assert!(
        inc.contains(
            "    PyObject *v1 = PyTuple_Size(args) > 1 ? PyTuple_GetItem(args, 1) : \
             pycc_ext_default_init_S_1();\n    if (v1 == NULL) {\n        return -1;\n    }\n"
        ),
        "{inc}"
    );
    assert!(
        inc.contains("static PyObject *pycc_ext_default_init_S_1(void)"),
        "{inc}"
    );
    assert!(
        inc.contains("PyUnicode_FromStringAndSize(\"x\", 1)"),
        "{inc}"
    );
}

#[test]
fn a_method_without_defaults_keeps_the_exact_arity_check() {
    let inc = companion(MODULE);
    assert!(
        inc.contains("\"S.plain() takes exactly 1 argument (%zd given)\""),
        "{inc}"
    );
    let plain = companion(
        "class P:\n    def __init__(self, n: int) -> None:\n        self.n = n\n\
         \x20   def get(self) -> int:\n        return self.n\n",
    );
    assert!(
        plain.contains("\"P.__init__() takes exactly 1 argument (%zd given)\""),
        "{plain}"
    );
    assert!(!plain.contains("pycc_ext_default_"), "{plain}");
    assert!(!plain.contains("takes from"), "{plain}");
}

#[test]
fn a_receiver_exact_copy_carries_its_origin_s_defaults() {
    // `B.g` is compiled for `B` from `A.g`'s source (#1337, D-254), so a
    // host calling `B().g()` omits `k` exactly as on `A`.
    let module = resolved(
        "class A:\n    def __init__(self) -> None:\n        self.x = 0\n\
         \x20   def m(self) -> int:\n        return 1\n\
         \x20   def g(self, k: int = 4) -> int:\n        return self.m() + k\n\
         class B(A):\n    def m(self) -> int:\n        return 2\n",
    );
    let exports = collect_exports(&module).expect("the module exports");
    let copy = exports
        .iter()
        .find(|export| export.name == "B.g")
        .expect("the copy is exported");
    assert_eq!(copy.defaults, vec![Some(pycc_hir::HirExpr::IntLiteral(4))]);
}

/// The generated companion for `source` built as an `--ext` module (D-258,
/// #1409), from real source through the `--ext` frontend.
fn ext_companion(tag: &str, source: &str) -> String {
    let dir = pycc_scratch::ScratchDir::new(tag).expect("scratch");
    let src = dir.join("m.py");
    std::fs::write(&src, source).expect("write source");
    let module = crate::frontend::resolve_frontend_with(
        &src,
        Some("m"),
        crate::modules::RelativeImports::Project,
    )
    .unwrap_or_else(|_| panic!("the fixture must type-check in an ext build: {source}"));
    let exports = collect_exports(&module).expect("the module exports");
    let publications = collect_class_publications(&module, &exports);
    let ctors = collect_constructors(&module, &publications);
    generate_exports_inc("m", &exports, &[], &publications, &ctors, &[])
}

/// #1409: an unannotated defaulted parameter in an `--ext` module is the
/// parameter its default implies, so the companion is byte-identical to its
/// annotated twin's -- `None` as `Any = None`, a literal as its scalar type
/// -- for a constructor, a method and a module-level `def` alike.
#[test]
fn an_unannotated_default_generates_its_annotated_twin_s_companion() {
    let unannotated = ext_companion(
        "1409_unannotated",
        "class P:\n\
         \x20   def __init__(self, n: int, state_stack=None, value_stack=None) -> None:\n\
         \x20       self.n = n\n\
         \x20   def copy(self, deepcopy_values=True, k=-2, r=0.5, s='\\u00e9') -> int:\n\
         \x20       return self.n if deepcopy_values else k\n\
         def f(a=3) -> int:\n    return a\n",
    );
    let annotated = ext_companion(
        "1409_annotated",
        "from typing import Any\n\
         class P:\n\
         \x20   def __init__(self, n: int, state_stack: Any = None, value_stack: Any = None) \
         -> None:\n\
         \x20       self.n = n\n\
         \x20   def copy(self, deepcopy_values: bool = True, k: int = -2, r: float = 0.5, \
         s: str = '\\u00e9') -> int:\n\
         \x20       return self.n if deepcopy_values else k\n\
         def f(a: int = 3) -> int:\n    return a\n",
    );
    assert_eq!(unannotated, annotated);
    for needle in [
        "PyObject *v1 = PyTuple_Size(args) > 1 ? PyTuple_GetItem(args, 1) : Py_None;",
        "PyObject *v0 = nargs > 0 ? args[0] : Py_True;",
        "pycc_ext_unpack_object(v1, ",
        "pycc_ext_unpack_bool(v0, ",
        // A module-level export keeps its exact arity (#1194).
        "\"f() takes exactly 1 argument (%zd given)\"",
    ] {
        assert!(unannotated.contains(needle), "{needle}\n{unannotated}");
    }
}
