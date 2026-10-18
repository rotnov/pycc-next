//! Publishing a function's or method's full signature -- defaults,
//! positional-only and keyword-only parameters, `*args`, `**kwargs` -- to
//! Python callers of an `--ext` module (#1194).
//!
//! `pycc_ast::ext_signatures` plans, for each such public `def`, a compiled
//! entry (`_pycc_sig_<qualname>`, exported like any module-level function)
//! and the source of a Python wrapper with the declared signature. This
//! module renders the C that installs the wrappers:
//!
//! * `pycc_ext_signature_publish`, which the shim's `pycc_ext_publish`
//!   calls when the body publishes an entry (right after the `def`, or
//!   after the class statement for a method): it runs the wrapper's source
//!   in the module's namespace (so a default is evaluated there, once),
//!   calls the resulting `_pycc_make` with the entry, and binds the wrapper
//!   to the function's name -- or, for a method, keeps it for the method's
//!   row of the class's type object. The entry itself is not bound.
//! * one forwarder per method, the `METH_FASTCALL | METH_KEYWORDS` row the
//!   type object carries for it: it calls the method's wrapper with the
//!   instance first, so CPython binds the host's arguments (and words its
//!   `TypeError`s) exactly as for the uncompiled method.
//!
//! The shim compiles the hook only when the generated companion defines
//! `PYCC_EXT_HAS_SIGNATURES`, so a module without such a signature
//! generates exactly the C it did before.

use pycc_ast::ExtSignature;
use std::fmt::Write as _;

use super::keywords::c_name;

/// The C symbol of the forwarder of method signature `slot`.
fn forwarder(slot: usize) -> String {
    format!("pycc_ext_sig_fwd_{slot}")
}

/// The file name a signature wrapper's code is compiled under.
const WRAPPER_FILENAME: &str = "<pycc signature>";

/// `signature`'s wrapper source as a pre-compiled chunk
/// (`super::precompiled`), which the hook loads instead of compiling it.
pub(crate) fn signature_chunk(signature: &ExtSignature) -> crate::fallback::CodeChunk {
    crate::fallback::CodeChunk {
        source: signature.wrapper.clone(),
        filename: WRAPPER_FILENAME.to_string(),
        future_annotations: false,
    }
}

/// The generated companion `inc` with `signatures` installed: the hook and
/// the forwarders after the module-name macros, and each method's row at
/// the head of its class's method table. Signature `k`'s wrapper is the
/// pre-compiled chunk `first_chunk + k` ([`signature_chunk`]). `inc` itself
/// when `signatures` is empty.
pub(crate) fn install(inc: String, signatures: &[ExtSignature], first_chunk: usize) -> String {
    if signatures.is_empty() {
        return inc;
    }
    let methods: Vec<(usize, &str, &str)> = signatures
        .iter()
        .filter_map(|signature| signature.method())
        .enumerate()
        .map(|(slot, (class, method))| (slot, class, method))
        .collect();
    let mut head = String::from("#define PYCC_EXT_HAS_SIGNATURES 1\n\n");
    let _ = writeln!(
        head,
        "/* The Python wrapper of each published method signature. */\n\
         static PyObject *pycc_ext_sig_wrappers[{}];\n",
        methods.len().max(1)
    );
    head.push_str(FORWARD_C);
    for (slot, _, _) in &methods {
        let _ = writeln!(
            head,
            "static PyObject *{}(PyObject *self, PyObject *const *args, Py_ssize_t nargs,\n\
             \x20                                   PyObject *kwnames)\n{{\n    \
             return pycc_ext_sig_forward({slot}, self, args, nargs, kwnames);\n}}\n",
            forwarder(*slot)
        );
    }
    head.push_str(
        "static const struct pycc_ext_signature {\n    const char *entry;\n    \
         const char *target;\n    Py_ssize_t slot;\n    Py_ssize_t chunk;\n} \
         pycc_ext_signatures[] = {\n",
    );
    let mut slot = 0;
    for (chunk, signature) in (first_chunk..).zip(signatures) {
        let index = if signature.method().is_some() {
            slot += 1;
            (slot - 1).to_string()
        } else {
            "-1".to_string()
        };
        let _ = writeln!(
            head,
            "    {{{}, {}, {index}, {chunk}}},",
            c_name(&signature.entry),
            c_name(&signature.qualname),
        );
    }
    head.push_str("    {NULL, NULL, 0, 0},\n};\n\n");
    head.push_str(PUBLISH_C);
    let marker = "#define PYCC_EXT_MODULE_NAME_STR";
    let at = inc
        .find(marker)
        .and_then(|start| inc[start..].find("\n\n").map(|end| start + end + 2))
        .expect("the generated companion defines the module name first");
    let mut out = format!("{}{head}{}", &inc[..at], &inc[at..]);
    for (slot, class, method) in methods {
        let table = format!("static PyMethodDef pycc_ext_type_methods_{class}[] = {{\n");
        if let Some(start) = out.find(&table) {
            let row = format!(
                "    {{{}, (PyCFunction)(void (*)(void)){}, METH_FASTCALL | METH_KEYWORDS, NULL}},\n",
                c_name(method),
                forwarder(slot)
            );
            out.insert_str(start + table.len(), &row);
        }
    }
    out
}

/// `pycc_ext_sig_forward`: calls method wrapper `index` with `self` first.
const FORWARD_C: &str = r#"static PyObject *pycc_ext_sig_forward(Py_ssize_t index, PyObject *self,
                                      PyObject *const *args, Py_ssize_t nargs,
                                      PyObject *kwnames)
{
    PyObject *wrapper = pycc_ext_sig_wrappers[index];
    Py_ssize_t count = nargs + (kwnames == NULL ? 0 : PyTuple_Size(kwnames));
    PyObject **stack;
    PyObject *result;
    Py_ssize_t i;

    if (wrapper == NULL) {
        PyErr_SetString(PyExc_RuntimeError,
                        "pycc: a method was called before its class statement ran");
        return NULL;
    }
    stack = (PyObject **)PyMem_Malloc((size_t)(count + 1) * sizeof(PyObject *));
    if (stack == NULL) {
        return PyErr_NoMemory();
    }
    stack[0] = self;
    for (i = 0; i < count; i++) {
        stack[i + 1] = args[i];
    }
    result = PyObject_Vectorcall(wrapper, stack, (size_t)(nargs + 1), kwnames);
    PyMem_Free(stack);
    return result;
}

"#;

/// `pycc_ext_signature_publish`: 1 when `name` is a signature entry (its
/// wrapper installed), 0 when it is not one, -1 with an exception set.
const PUBLISH_C: &str = r#"static int pycc_ext_signature_publish(PyObject *module, const char *name, PyObject *entry)
{
    const struct pycc_ext_signature *signature;
    PyObject *dict;
    PyObject *code;
    PyObject *result;
    PyObject *make;
    PyObject *wrapper;
    PyObject *old;
    int status;

    for (signature = pycc_ext_signatures; signature->entry != NULL; signature++) {
        if (strcmp(signature->entry, name) != 0) {
            continue;
        }
        dict = PyModule_GetDict(module);
        code = pycc_ext_chunk_code(signature->chunk);
        if (code == NULL) {
            return -1;
        }
        result = PyEval_EvalCode(code, dict, dict);
        Py_DECREF(code);
        if (result == NULL) {
            return -1;
        }
        Py_DECREF(result);
        make = PyDict_GetItemString(dict, "_pycc_make");
        if (make == NULL) {
            PyErr_SetString(PyExc_SystemError, "pycc: a signature wrapper defined no factory");
            return -1;
        }
        Py_INCREF(make);
        if (PyDict_DelItemString(dict, "_pycc_make") != 0) {
            Py_DECREF(make);
            return -1;
        }
        wrapper = PyObject_CallFunctionObjArgs(make, entry, NULL);
        Py_DECREF(make);
        if (wrapper == NULL) {
            return -1;
        }
        if (signature->slot < 0) {
            status = PyModule_AddObjectRef(module, signature->target, wrapper);
            Py_DECREF(wrapper);
            return status < 0 ? -1 : 1;
        }
        old = pycc_ext_sig_wrappers[signature->slot];
        pycc_ext_sig_wrappers[signature->slot] = wrapper;
        Py_XDECREF(old);
        return 1;
    }
    return 0;
}

"#;

#[cfg(test)]
mod tests {
    use super::*;

    const INC: &str = "/* Generated */\n#define PYCC_EXT_MODULE_NAME m\n\
                       #define PYCC_EXT_MODULE_NAME_STR \"m\"\n\nBODY\n\
                       static PyMethodDef pycc_ext_type_methods_Box[] = {\n    ROWS\n};\n";

    fn signature(qualname: &str) -> ExtSignature {
        ExtSignature {
            qualname: qualname.to_string(),
            entry: format!("_pycc_sig_{}", qualname.replace('.', "_d")),
            wrapper: "def _pycc_make(e):\n    return e\n".to_string(),
        }
    }

    #[test]
    fn no_signature_leaves_the_companion_unchanged() {
        assert_eq!(install(INC.to_string(), &[], 0), INC);
    }

    #[test]
    fn a_method_gets_a_forwarder_row_and_a_function_a_table_entry() {
        let out = install(INC.to_string(), &[signature("f"), signature("Box.area")], 3);
        let head = out
            .find("#define PYCC_EXT_HAS_SIGNATURES")
            .expect("the macro");
        assert!(head > out.find("PYCC_EXT_MODULE_NAME_STR").unwrap());
        assert!(head < out.find("BODY").unwrap());
        assert!(out.contains("{\"_pycc_sig_f\", \"f\", -1, 3}"), "{out}");
        assert!(out.contains("\"Box\\056area\", 0, "), "{out}");
        assert!(out.contains(
            "pycc_ext_type_methods_Box[] = {\n    {\"area\", (PyCFunction)(void (*)(void))pycc_ext_sig_fwd_0, \
             METH_FASTCALL | METH_KEYWORDS, NULL},\n    ROWS"
        ), "{out}");
        assert!(out.contains("static PyObject *pycc_ext_sig_wrappers[1];"));
    }
}
