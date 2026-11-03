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
/// The wrapper's code is given its qualified name and
/// [`pycc_ast::WRAPPER_FLAG`] at build time, so `_pycc_make` skips its own
/// `code.replace` when it runs from the table.
pub(crate) fn signature_chunk(signature: &ExtSignature) -> crate::fallback::CodeChunk {
    let name = signature
        .qualname
        .rsplit('.')
        .next()
        .unwrap_or(&signature.qualname);
    crate::fallback::CodeChunk {
        source: signature.wrapper.clone(),
        filename: WRAPPER_FILENAME.to_string(),
        future_annotations: false,
        rewrites: vec![crate::fallback::CodeRewrite::Retag {
            path: vec!["_pycc_make".to_string(), name.to_string()],
            qualname: signature.qualname.clone(),
            flags: pycc_ast::WRAPPER_FLAG,
        }],
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
    let methods: Vec<(usize, &str, &str, bool)> = signatures
        .iter()
        .filter_map(|signature| Some((signature.method()?, signature.static_method)))
        .enumerate()
        .map(|(slot, ((class, method), is_static))| (slot, class, method, is_static))
        .collect();
    let mut head = String::from("#define PYCC_EXT_HAS_SIGNATURES 1\n\n");
    let _ = writeln!(
        head,
        "/* The Python wrapper of each published method signature. */\n\
         static PyObject *pycc_ext_sig_wrappers[{}];\n",
        methods.len().max(1)
    );
    head.push_str(FORWARD_C);
    for (slot, _, _, _) in &methods {
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
    for (slot, class, method, is_static) in methods {
        let table = format!("static PyMethodDef pycc_ext_type_methods_{class}[] = {{\n");
        if let Some(start) = out.find(&table) {
            // A static method's row hands the forwarder `self == NULL`.
            let flags = if is_static {
                "METH_FASTCALL | METH_KEYWORDS | METH_STATIC"
            } else {
                "METH_FASTCALL | METH_KEYWORDS"
            };
            let row = format!(
                "    {{{}, (PyCFunction)(void (*)(void)){}, {flags}, NULL}},\n",
                c_name(method),
                forwarder(slot)
            );
            out.insert_str(start + table.len(), &row);
        }
    }
    out
}

/// `pycc_ext_sig_forward`: calls method wrapper `index` with `self` first
/// -- none for a static method, whose row hands it `NULL`.
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
    if (self == NULL) {
        return PyObject_Vectorcall(wrapper, args, (size_t)nargs, kwnames);
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
const PUBLISH_C: &str = r#"/* The builtins the factory `_pycc_make` binds its parameters to, after
 * `_pycc_entry` (`_pycc_missing` is a fresh `object()`), in its parameters'
 * order (`SIG` in `precompile.py`), and their spellings interned. */
#define PYCC_EXT_SIG_BOUND 10
static const char pycc_ext_sig_bound[] =
    "object\000tuple\000isinstance\000type\000TypeError\000int\000float\000str\000bool\000"
    "BaseException\000";
static PyObject *pycc_ext_sig_keys[PYCC_EXT_SIG_BOUND];

/*
 * The wrapper of a signature whose table entry is the wrapper function's
 * `(code, defaults, kwdefaults, cells)` (`sig` in `precompile.py`), made as
 * running the chunk and calling its `_pycc_make` with `entry` makes it,
 * without either: the factory's defaults are read from `ns` and then its
 * builtins as the chunk's `LOAD_NAME`s read them, every one before
 * `object()` is called, and the function is made in `ns` with closure cell
 * `i` holding factory parameter `cells[i]`. A new reference; NULL with an
 * exception set; NULL with none when it cannot be made this way (a name
 * the factory reads is unbound: running the chunk raises as CPython would).
 */
static PyObject *pycc_ext_sig_wrapper(PyObject *spec, PyObject *ns, PyObject *entry)
{
    PyObject *bound[PYCC_EXT_SIG_BOUND + 1] = {NULL};
    PyObject *builtins = NULL;
    PyObject *closure = NULL, *kwdefaults = NULL, *wrapper = NULL;
    PyObject *cells = PyTuple_GetItem(spec, 3);
    const char *name;
    char *at;
    Py_ssize_t i, n;
    int found;

    if (pycc_ext_sig_keys[0] == NULL) {
        for (i = 0, name = pycc_ext_sig_bound; i < PYCC_EXT_SIG_BOUND; i++, name += strlen(name) + 1) {
            pycc_ext_sig_keys[i] = PyUnicode_InternFromString(name);
            if (pycc_ext_sig_keys[i] == NULL) {
                return NULL;
            }
        }
    }
    if (cells == NULL || !PyBytes_Check(cells) || PyBytes_AsStringAndSize(cells, &at, &n) != 0) {
        PyErr_Clear();
        return NULL;
    }
    found = PyMapping_GetOptionalItem(ns, pycc_ext_chunk_builtins_key(), &builtins);
    if (found < 0) {
        return NULL;
    }
    if (found == 0) {
        builtins = Py_NewRef(PyEval_GetBuiltins());
    } else if (PyModule_Check(builtins)) {
        PyObject *module = builtins;

        builtins = Py_NewRef(PyModule_GetDict(module));
        Py_DECREF(module);
    }
    for (i = 0; i < PYCC_EXT_SIG_BOUND; i++) {
        found = PyMapping_GetOptionalItem(ns, pycc_ext_sig_keys[i], &bound[i + 1]);
        if (found == 0) {
            found = PyMapping_GetOptionalItem(builtins, pycc_ext_sig_keys[i], &bound[i + 1]);
        }
        if (found <= 0) {
            goto done;
        }
    }
    /* `_pycc_missing=object()`: the call, in place of `object`. */
    {
        PyObject *object = bound[1];

        bound[1] = PyObject_CallNoArgs(object);
        Py_DECREF(object);
        if (bound[1] == NULL) {
            goto done;
        }
    }
    bound[0] = Py_NewRef(entry);
    if (pycc_ext_chunk_function_type_get() == NULL) {
        goto done;
    }
    closure = PyTuple_New(n);
    for (i = 0; closure != NULL && i < n; i++) {
        unsigned char k = (unsigned char)at[i];
        PyObject *cell = k <= PYCC_EXT_SIG_BOUND
                             ? PyObject_CallFunctionObjArgs(pycc_ext_chunk_cell_type, bound[k], NULL)
                             : NULL;

        if (cell == NULL) {
            if (!PyErr_Occurred()) {
                PyErr_SetString(PyExc_SystemError, "pycc: a signature wrapper closes over an unknown name");
            }
            Py_CLEAR(closure);
        } else if (PyTuple_SetItem(closure, i, cell) != 0) {
            Py_CLEAR(closure);
        }
    }
    if (closure == NULL) {
        goto done;
    }
    /* A fresh keyword-defaults dict, as `BUILD_CONST_KEY_MAP` makes one. */
    kwdefaults = PyTuple_GetItem(spec, 2);
    kwdefaults = kwdefaults == Py_None ? Py_NewRef(kwdefaults) : PyDict_Copy(kwdefaults);
    if (kwdefaults != NULL) {
        wrapper = PyObject_CallFunctionObjArgs(pycc_ext_chunk_function_type, PyTuple_GetItem(spec, 0), ns,
                                               Py_None, PyTuple_GetItem(spec, 1), closure, kwdefaults, NULL);
    }
done:
    for (i = 0; i <= PYCC_EXT_SIG_BOUND; i++) {
        Py_XDECREF(bound[i]);
    }
    Py_XDECREF(builtins);
    Py_XDECREF(closure);
    Py_XDECREF(kwdefaults);
    return wrapper;
}

static int pycc_ext_signature_publish(PyObject *module, const char *name, PyObject *entry)
{
    const struct pycc_ext_signature *signature;
    PyObject *dict;
    PyObject *make;
    PyObject *wrapper;
    PyObject *old;
    int status;

    for (signature = pycc_ext_signatures; signature->entry != NULL; signature++) {
        if (strcmp(signature->entry, name) != 0) {
            continue;
        }
        dict = PyModule_GetDict(module);
        wrapper = NULL;
        if (pycc_ext_chunks[signature->chunk].precompiled && pycc_ext_chunks_usable()) {
            /* Owned while it is used, as `pycc_ext_chunk_run_in` owns it. */
            PyObject *spec = Py_XNewRef(pycc_ext_chunk_entry(signature->chunk));

            if (spec != NULL && PyTuple_Size(spec) == 4) {
                wrapper = pycc_ext_sig_wrapper(spec, dict, entry);
            }
            Py_XDECREF(spec);
            if (wrapper == NULL && PyErr_Occurred()) {
                return -1;
            }
        }
        if (wrapper == NULL) {
            if (pycc_ext_chunk_run_in(signature->chunk, dict, 0) != 0) {
                return -1;
            }
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
            static_method: false,
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
