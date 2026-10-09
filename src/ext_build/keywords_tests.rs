//! Which exports a host call may name keywords for (#1461), and the C the
//! keyword-enabled ones get. Pinned on companions generated from real
//! lowered source, so the carried parameters and defaults are the ones
//! `pycc_hir` really records; `tests/issue_1461_ext_host_keywords.rs` runs
//! the same shapes against CPython.

use super::super::{
    collect_class_publications, collect_constructors, collect_exports, generate_exports_inc,
};
use super::*;

/// Parses, lowers and type-checks `source`, returning the resolved module.
fn resolved(source: &str) -> HirModule {
    let module = pycc_parser::parse(source).expect("test fixture must parse");
    let hir = pycc_hir::lower_checked(&module).expect("test fixture must lower");
    pycc_types::check_and_resolve(&hir).expect("test fixture must check")
}

/// The exports and constructors of `source`, keyword names bound against
/// `signatures` exactly as `plan_ext` binds them.
fn bound(source: &str, signatures: &SourceSignatures) -> (Vec<ExtExport>, Vec<ExtCtor>, String) {
    let module = resolved(source);
    let mut exports = collect_exports(&module).expect("the module exports");
    bind_keyword_names(&module, signatures, &mut exports);
    let publications = collect_class_publications(&module, &exports);
    let mut ctors = collect_constructors(&module, &publications);
    bind_ctor_keyword_names(&module, signatures, &mut ctors);
    let inc = generate_exports_inc("m", &exports, &[], &publications, &ctors, &[]);
    (exports, ctors, inc)
}

fn names_of<'a>(exports: &'a [ExtExport], name: &str) -> Option<&'a [String]> {
    exports
        .iter()
        .find(|export| export.name == name)
        .unwrap_or_else(|| panic!("`{name}` is exported"))
        .keyword_names
        .as_deref()
}

fn strings(names: &[&str]) -> Vec<String> {
    names.iter().map(|name| (*name).to_string()).collect()
}

const MODULE: &str = "from typing import Generic, TypeVar\n\
T = TypeVar('T')\n\
class P:\n\
\x20   def __init__(self, n: int, step: int = 1) -> None:\n\
\x20       self.n = n\n        self.step = step\n\
\x20   def copy(self, deepcopy_values: bool = True) -> int:\n\
\x20       return self.n\n\
\x20   def add(self, a: int, b: int = 10) -> int:\n\
\x20       return a + b\n\
\x20   def size(self) -> int:\n\
\x20       return self.n\n\
\x20   @staticmethod\n    def mul(x: int, y: int = 2) -> int:\n        return x * y\n\
\x20   @classmethod\n    def tag(cls, t: str) -> str:\n        return t\n\
class Exact:\n\
\x20   def __init__(self, k: int, j: int) -> None:\n\
\x20       self.k = k\n        self.j = j\n\
class State(Generic[T]):\n\
\x20   def __init__(self, k: int) -> None:\n\
\x20       self.k = k\n\
\x20   def copy(self, deepcopy_values: bool = True) -> int:\n\
\x20       return self.k\n\
def rep(a: int, b: str) -> str:\n    return b * a\n\
def pad(a: int, b: int = 2) -> int:\n    return a + b\n\
def only(a: int, /, b: int) -> int:\n    return a - b\n";

#[test]
fn each_export_whose_source_def_the_binder_models_gets_its_source_names() {
    let (exports, ctors, _) = bound(MODULE, &SourceSignatures::from_source(MODULE));
    // The receiver is dropped; a static method has none to drop.
    assert_eq!(
        names_of(&exports, "P.copy"),
        Some(&strings(&["deepcopy_values"])[..])
    );
    assert_eq!(names_of(&exports, "P.add"), Some(&strings(&["a", "b"])[..]));
    assert_eq!(names_of(&exports, "P.size"), Some(&[][..]));
    assert_eq!(
        names_of(&exports, "P.mul.static"),
        Some(&strings(&["x", "y"])[..])
    );
    assert_eq!(
        names_of(&exports, "P.tag.classmethod"),
        Some(&strings(&["t"])[..])
    );
    // lark's `ParserState(Generic[StateT]).copy`: the erased base leaves
    // the plain `Class.method` name.
    assert_eq!(
        names_of(&exports, "State.copy"),
        Some(&strings(&["deepcopy_values"])[..])
    );
    assert_eq!(names_of(&exports, "rep"), Some(&strings(&["a", "b"])[..]));
    // A module-level default is not carried (#1194), so the source's
    // default count disagrees with the export's and the export is refused
    // rather than reporting `b` as missing.
    assert_eq!(names_of(&exports, "pad"), None);
    // `def only(a, /, b)`: lowering drops the `/`, the source keeps it.
    assert_eq!(names_of(&exports, "only"), None);
    let ctor = |class: &str| {
        ctors
            .iter()
            .find(|ctor| ctor.class == class)
            .unwrap_or_else(|| panic!("`{class}` is constructible"))
            .keyword_names
            .clone()
    };
    assert_eq!(ctor("P"), Some(strings(&["n", "step"])));
    assert_eq!(ctor("Exact"), Some(strings(&["k", "j"])));
    assert_eq!(ctor("State"), Some(strings(&["k"])));
}

#[test]
fn an_unparsable_or_silent_source_enables_nothing() {
    // The frontend parsed the same text first, so this is fail-closed
    // rather than reachable; it still must not enable an export.
    let broken = SourceSignatures::from_source("def (:\n");
    assert!(broken.by_name.is_empty());
    let (exports, ctors, inc) = bound(MODULE, &broken);
    assert!(exports.iter().all(|export| export.keyword_names.is_none()));
    assert!(ctors.iter().all(|ctor| ctor.keyword_names.is_none()));
    assert!(!inc.contains("METH_KEYWORDS"), "{inc}");
    assert!(!inc.contains("kwnames"), "{inc}");
    assert!(!inc.contains("kw_slots"), "{inc}");
}

#[test]
fn a_keyword_disabled_companion_is_byte_identical_to_one_never_bound() {
    let module = resolved(MODULE);
    let exports = collect_exports(&module).expect("the module exports");
    let publications = collect_class_publications(&module, &exports);
    let ctors = collect_constructors(&module, &publications);
    let unbound = generate_exports_inc("m", &exports, &[], &publications, &ctors, &[]);
    let (_, _, disabled) = bound(MODULE, &SourceSignatures::default());
    assert_eq!(unbound, disabled);
}

#[test]
fn a_parameter_kind_the_binder_does_not_model_marks_the_signature_unbindable() {
    let table = SourceSignatures::from_source(
        "def plain(a, b=1): pass\n\
         def posonly(a, /, b): pass\n\
         def star(a, *rest): pass\n\
         def kwonly(a, *, k): pass\n\
         def kwargs(a, **kw): pass\n\
         class C:\n    def m(self, x=2, y=3): pass\n    z = 1\n\
         x = 5\n",
    );
    let get = |name: &str| table.by_name[name].clone().expect("defined once");
    assert_eq!(
        get("plain"),
        SourceSignature {
            names: strings(&["a", "b"]),
            bindable: true,
            defaults: 1,
        }
    );
    // Positional-only names are still listed, so the count check sees them.
    assert_eq!(get("posonly").names, strings(&["a", "b"]));
    for name in ["posonly", "star", "kwonly", "kwargs"] {
        assert!(!get(name).bindable, "{name}");
    }
    assert_eq!(
        get("C.m"),
        SourceSignature {
            names: strings(&["self", "x", "y"]),
            bindable: true,
            defaults: 2,
        }
    );
    // Neither a class attribute nor a module statement is a signature.
    assert_eq!(table.by_name.len(), 6);
}

#[test]
fn a_name_defined_twice_is_refused_because_its_source_is_ambiguous() {
    let source = "def f(a: int) -> int:\n    return a\n\
                  def f(b: int) -> int:\n    return b\n";
    let table = SourceSignatures::from_source(source);
    assert_eq!(table.by_name.get("f"), Some(&None));
    let (exports, _, _) = bound(source, &table);
    assert_eq!(names_of(&exports, "f"), None);
}

#[test]
fn a_parameter_count_the_export_does_not_carry_is_refused() {
    let module = resolved(MODULE);
    let table = SourceSignatures::from_source(MODULE);
    assert_eq!(
        table.keyword_names(&module, "rep", false, 2, &[]),
        Some(strings(&["a", "b"]))
    );
    assert_eq!(table.keyword_names(&module, "rep", false, 3, &[]), None);
    assert_eq!(table.keyword_names(&module, "absent", false, 2, &[]), None);
}

#[test]
fn a_c_name_escapes_every_byte_outside_the_identifier_set() {
    assert_eq!(c_name("deepcopy_values1"), "\"deepcopy_values1\"");
    assert_eq!(c_name("\u{e9}t\u{e9}"), "\"\\303\\251t\\303\\251\"");
}

#[test]
fn a_zero_parameter_name_table_still_has_one_element() {
    assert_eq!(
        names_table("wrap_f", &[]),
        "static const char *const pycc_ext_kwnames_wrap_f[] = {NULL};\n"
    );
    assert_eq!(
        names_table("wrap_f", &strings(&["a", "b"])),
        "static const char *const pycc_ext_kwnames_wrap_f[] = {\"a\", \"b\"};\n"
    );
}

#[test]
fn a_keyword_enabled_method_wrapper_binds_before_its_arity_check() {
    let (_, _, inc) = bound(MODULE, &SourceSignatures::from_source(MODULE));
    let symbol = pycc_codegen::mangle_ext_name("P.add");
    let prefix = format!("wrap_{symbol}");
    let wrapper = format!(
        "static const char *const pycc_ext_kwnames_{prefix}[] = {{\"a\", \"b\"}};\n\
         static PyObject *pycc_ext_wrap_{symbol}(PyObject *self, PyObject *const *args, \
         Py_ssize_t nargs, PyObject *kwnames)\n{{\n"
    );
    assert!(inc.contains(&wrapper), "{inc}");
    let binding = format!(
        "    PyObject *kw_slots[2];\n    \
         if (kwnames != NULL && PyTuple_Size(kwnames) != 0) {{\n        \
         if (pycc_ext_kw_bind_fastcall(\"P.add\", pycc_ext_kwnames_{prefix}, 2, 1, kw_slots, \
         args, nargs, kwnames) < 0) {{\n            return NULL;\n        }}\n        \
         if (kw_slots[1] == NULL) {{\n            \
         kw_slots[1] = pycc_ext_default_{prefix}_1();\n            \
         if (kw_slots[1] == NULL) {{\n                return NULL;\n            }}\n        }}\n        \
         args = kw_slots;\n        nargs = 2;\n    }}\n    \
         if (nargs < 1 || nargs > 2) {{\n"
    );
    assert!(inc.contains(&binding), "{inc}");
    let row = format!(
        "{{\"add\", (PyCFunction)(void (*)(void))pycc_ext_wrap_{symbol}, \
         METH_FASTCALL | METH_KEYWORDS, NULL}}"
    );
    assert!(inc.contains(&row), "{inc}");
    // A singleton default needs no NULL check.
    let copy = "        if (kw_slots[0] == NULL) {\n            kw_slots[0] = Py_True;\n        }\n        \
                args = kw_slots;\n        nargs = 1;\n";
    assert!(inc.contains(copy), "{inc}");
    // A zero-parameter method still gets a one-element slot array.
    let size = pycc_codegen::mangle_ext_name("P.size");
    assert!(
        inc.contains(&format!(
            "pycc_ext_kwnames_wrap_{size}[] = {{NULL}};\n\
             static PyObject *pycc_ext_wrap_{size}(PyObject *self, PyObject *const *args, \
             Py_ssize_t nargs, PyObject *kwnames)\n{{\n"
        )),
        "{inc}"
    );
    assert!(
        inc.contains(&format!(
            "    PyObject *kw_slots[1];\n    \
             if (kwnames != NULL && PyTuple_Size(kwnames) != 0) {{\n        \
             if (pycc_ext_kw_bind_fastcall(\"P.size\", pycc_ext_kwnames_wrap_{size}, 0, 0, \
             kw_slots, args, nargs, kwnames) < 0) {{\n"
        )),
        "{inc}"
    );
    for (method, flags) in [
        ("mul", "METH_FASTCALL | METH_STATIC | METH_KEYWORDS"),
        ("tag", "METH_FASTCALL | METH_CLASS | METH_KEYWORDS"),
    ] {
        assert!(
            inc.contains(&format!("{{\"{method}\", ")) && inc.contains(flags),
            "{method}: {inc}"
        );
    }
}

#[test]
fn a_module_level_row_carries_its_own_keyword_flag() {
    let (_, _, inc) = bound(MODULE, &SourceSignatures::from_source(MODULE));
    assert!(
        inc.contains(
            "    {\"rep\", (PyCFunction)(void (*)(void))pycc_ext_wrap_rep, \
             METH_FASTCALL | METH_KEYWORDS, NULL},\n"
        ),
        "{inc}"
    );
    assert!(
        inc.contains(
            "    {\"pad\", (PyCFunction)(void (*)(void))pycc_ext_wrap_pad, \
             METH_FASTCALL, NULL},\n"
        ),
        "{inc}"
    );
    assert!(
        inc.contains(
            "static PyObject *pycc_ext_wrap_pad(PyObject *self, PyObject *const *args, \
             Py_ssize_t nargs)\n"
        ),
        "{inc}"
    );
}

#[test]
fn a_keyword_enabled_constructor_reads_the_bound_array_once_a_keyword_was_given() {
    let (_, _, inc) = bound(MODULE, &SourceSignatures::from_source(MODULE));
    // With defaults: the range check and the defaulted local read through
    // the bound array.
    assert!(
        inc.contains(
            "    PyObject *kw_slots[2];\n    int kw_bound = 0;\n    \
             if (kwds != NULL && PyDict_Size(kwds) != 0) {\n        \
             if (pycc_ext_kw_bind_dict(\"P.__init__\", pycc_ext_kwnames_init_P, 2, 1, \
             kw_slots, args, kwds) < 0) {\n            return -1;\n        }\n"
        ),
        "{inc}"
    );
    assert!(
        inc.contains(
            "    if ((kw_bound ? 2 : PyTuple_Size(args)) < 1 || \
             (kw_bound ? 2 : PyTuple_Size(args)) > 2) {\n"
        ),
        "{inc}"
    );
    assert!(
        inc.contains(
            "PyObject *v1 = (kw_bound ? 2 : PyTuple_Size(args)) > 1 ? \
             (kw_bound ? kw_slots[1] : PyTuple_GetItem(args, 1)) : pycc_ext_default_init_P_1();\n"
        ),
        "{inc}"
    );
    // Without defaults: the exact check and every item.
    assert!(
        inc.contains("static const char *const pycc_ext_kwnames_init_Exact[] = {\"k\", \"j\"};\n"),
        "{inc}"
    );
    assert!(
        inc.contains(
            "    if ((kw_bound ? 2 : PyTuple_Size(args)) != 2) {\n        \
             PyErr_Format(PyExc_TypeError, \"Exact.__init__() takes exactly 2 arguments \
             (%zd given)\", (kw_bound ? 2 : PyTuple_Size(args)));\n        return -1;\n    }\n"
        ),
        "{inc}"
    );
    assert!(
        inc.contains("(kw_bound ? kw_slots[0] : PyTuple_GetItem(args, 0))"),
        "{inc}"
    );
    // The hand-written refusal is gone for an enabled constructor.
    assert!(
        !inc.contains("Exact.__init__() takes no keyword arguments"),
        "{inc}"
    );
}

#[test]
fn the_tp_init_prologue_fails_with_minus_one() {
    let defaults = [None, Some(HirExpr::IntLiteral(4))];
    let prologue = tp_init_prologue("init_C", "C.__init__", &strings(&["a", "b"]), &defaults);
    assert!(
        prologue.body.contains(
            "            if (kw_slots[1] == NULL) {\n                return -1;\n            }\n        }\n        \
             kw_bound = 1;\n    }\n"
        ),
        "{}",
        prologue.body
    );
    assert_eq!(tp_init_count(3), "(kw_bound ? 3 : PyTuple_Size(args))");
    assert_eq!(
        tp_init_item(2),
        "(kw_bound ? kw_slots[2] : PyTuple_GetItem(args, 2))"
    );
}

#[test]
fn the_flags_add_meth_keywords_exactly_for_a_keyword_enabled_export() {
    let (mut exports, _, _) = bound(MODULE, &SourceSignatures::default());
    let export = exports
        .iter_mut()
        .find(|export| export.name == "rep")
        .expect("`rep` is exported");
    assert_eq!(method_flags("METH_FASTCALL", export), "METH_FASTCALL");
    export.keyword_names = Some(strings(&["a", "b"]));
    assert_eq!(
        method_flags("METH_FASTCALL", export),
        "METH_FASTCALL | METH_KEYWORDS"
    );
}
