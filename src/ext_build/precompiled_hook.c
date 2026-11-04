/*
 * 1 when the embedded code objects are this interpreter's
 * (`pycc_ext_marshal_loads`). Decided once per process; `marshal.loads`
 * (borrowed from the runtime unit, which keeps it) is kept with it.
 */
static int pycc_ext_chunk_usable = -1;
static PyObject *pycc_ext_chunk_loads = NULL;
/* The unmarshalled table (`pycc_ext_chunk_table_data`), from the hook's
 * first call until the body has run. */
static PyObject *pycc_ext_chunk_table = NULL;
/* `fix(code, orders)` (`ORDERS_FIX` in `src/ext_build/precompiled.rs`),
 * made once from the table's last entry; NULL when unavailable. */
static PyObject *pycc_ext_chunk_fix = NULL;

/* `pycc_ext_chunk_fix`, made by running the table's last entry; NULL (no
 * exception left set) when it cannot be. */
static PyObject *pycc_ext_chunk_fixer(void)
{
    PyObject *code, *ns;

    if (pycc_ext_chunk_fix != NULL) {
        return pycc_ext_chunk_fix;
    }
    code = PyTuple_GetItem(pycc_ext_chunk_table, pycc_ext_chunk_count);
    ns = code == NULL ? NULL : PyDict_New();
    if (ns != NULL && PyDict_SetItemString(ns, "__builtins__", PyEval_GetBuiltins()) == 0) {
        PyObject *done = PyEval_EvalCode(code, ns, ns);

        Py_XDECREF(done);
        if (done != NULL) {
            pycc_ext_chunk_fix = PyDict_GetItemString(ns, "fix");
            Py_XINCREF(pycc_ext_chunk_fix);
        }
    }
    Py_XDECREF(ns);
    PyErr_Clear();
    return pycc_ext_chunk_fix;
}

static int pycc_ext_chunks_usable(void)
{
    if (pycc_ext_chunk_usable >= 0) {
        return pycc_ext_chunk_usable;
    }
    pycc_ext_chunk_loads = pycc_ext_chunk_magic == 0 ? NULL : pycc_ext_marshal_loads(pycc_ext_chunk_magic);
    pycc_ext_chunk_usable = pycc_ext_chunk_loads != NULL;
    return pycc_ext_chunk_usable;
}

/* `compile()` of chunk `chunk`'s embedded source, as the rewrite did. */
static PyObject *pycc_ext_chunk_compile(const struct pycc_ext_chunk *chunk)
{
    PyObject *compile;
    PyObject *source;
    PyObject *flags;
    PyObject *result;

    compile = PyDict_GetItemString(PyEval_GetBuiltins(), "compile");
    if (compile == NULL) {
        PyErr_SetString(PyExc_SystemError, "pycc: builtins.compile is missing");
        return NULL;
    }
    if (chunk->future_annotations) {
        PyObject *future = PyImport_ImportModule("__future__");
        PyObject *feature;
        if (future == NULL) {
            return NULL;
        }
        feature = PyObject_GetAttrString(future, "annotations");
        Py_DECREF(future);
        if (feature == NULL) {
            return NULL;
        }
        flags = PyObject_GetAttrString(feature, "compiler_flag");
        Py_DECREF(feature);
    } else {
        flags = PyLong_FromLong(0);
    }
    if (flags == NULL) {
        return NULL;
    }
    source = PyUnicode_DecodeUTF8(pycc_ext_chunk_text + chunk->source, chunk->source_len, NULL);
    if (source == NULL) {
        Py_DECREF(flags);
        return NULL;
    }
    result = PyObject_CallFunction(compile, "OssOi", source, pycc_ext_chunk_text + chunk->filename,
                                   "exec", flags, 1);
    Py_DECREF(source);
    Py_DECREF(flags);
    return result;
}

/* The table's entry for chunk `index` (the table unmarshalled on the first
 * call); NULL with no exception set when it cannot be had that way. */
static PyObject *pycc_ext_chunk_entry(Py_ssize_t index)
{
    PyObject *entry;

    if (pycc_ext_chunk_table == NULL) {
        PyObject *view = PyMemoryView_FromMemory((char *)pycc_ext_chunk_table_data,
                                                 pycc_ext_chunk_table_len, PyBUF_READ);

        if (view != NULL) {
            pycc_ext_chunk_table = PyObject_CallFunctionObjArgs(pycc_ext_chunk_loads, view, NULL);
            Py_DECREF(view);
        }
        if (pycc_ext_chunk_table != NULL
            && (!PyTuple_Check(pycc_ext_chunk_table)
                || PyTuple_Size(pycc_ext_chunk_table) != pycc_ext_chunk_count + 1)) {
            Py_CLEAR(pycc_ext_chunk_table);
        }
        if (pycc_ext_chunk_table == NULL) {
            PyErr_Clear();
            return NULL;
        }
    }
    entry = PyTuple_GetItem(pycc_ext_chunk_table, index);
    if (entry == NULL || !PyTuple_Check(entry)
        || PyTuple_Size(entry) < 1 || PyTuple_Size(entry) > 4) {
        PyErr_Clear();
        return NULL;
    }
    return entry;
}

/* `code` with its `frozenset` constants rebuilt in source order (`orders`,
 * None when it has none); NULL with no exception set when it cannot be. */
static PyObject *pycc_ext_chunk_reordered(PyObject *code, PyObject *orders)
{
    PyObject *fix;

    if (orders == Py_None) {
        return Py_NewRef(code);
    }
    fix = pycc_ext_chunk_fixer();
    code = fix == NULL ? NULL : PyObject_CallFunctionObjArgs(fix, code, orders, NULL);
    if (code == NULL) {
        PyErr_Clear();
    }
    return code;
}

/* The code object of a `(code, orders)` entry; NULL with no exception set
 * when it cannot be had. */
static PyObject *pycc_ext_chunk_entry_code(PyObject *entry)
{
    return pycc_ext_chunk_reordered(PyTuple_GetItem(entry, 0), PyTuple_GetItem(entry, 1));
}

/* `types.FunctionType`, kept from the first `def` entry run. */
static PyObject *pycc_ext_chunk_function_type = NULL;

/* `types.CellType`, kept from the first signature wrapper made
 * (`pycc_ext_sig_wrapper`): `PyCell_New` is not in the limited API. */
static PyObject *pycc_ext_chunk_cell_type = NULL;

/* `types.FunctionType` (borrowed), imported on the first call, and
 * `types.CellType` with it; NULL with an exception set. */
static PyObject *pycc_ext_chunk_function_type_get(void)
{
    if (pycc_ext_chunk_function_type == NULL || pycc_ext_chunk_cell_type == NULL) {
        PyObject *types = PyImport_ImportModule("types");

        if (types == NULL) {
            return NULL;
        }
        if (pycc_ext_chunk_function_type == NULL) {
            pycc_ext_chunk_function_type = PyObject_GetAttrString(types, "FunctionType");
        }
        if (pycc_ext_chunk_function_type != NULL && pycc_ext_chunk_cell_type == NULL) {
            pycc_ext_chunk_cell_type = PyObject_GetAttrString(types, "CellType");
        }
        Py_DECREF(types);
        if (pycc_ext_chunk_cell_type == NULL) {
            return NULL;
        }
    }
    return pycc_ext_chunk_function_type;
}

/*
 * Runs a `def` entry in `ns` as the chunk's own code would: the chunk is
 * only `def` statements whose defaults, keyword defaults and annotations
 * are constants (no decorator, no closure), each perhaps followed by a
 * `__pycc_forward__(...)` line (`precompile.py`'s `defs`), so it can
 * raise nothing of its own and needs no frame. Each `(code, defaults,
 * kwdefaults, annotations, name, forward, orders)` is a function made as
 * `MAKE_FUNCTION` makes it -- name, qualified name, docstring, `__module__`
 * and builtins from the code (its set constants rebuilt from `orders`) and
 * `ns` -- with the annotations as the dict CPython 3.13 makes from the same
 * tuple on first read, or 3.15's `__annotate__` function made from its
 * code, stored as `name`, and then the forwarders patched.
 * A `(name, code, order, orders)` is a nested statement's scaffold chunk
 * (`src/fallback/nested.rs`), whose `__pycc_fallback_nestspec` call the
 * build made: `name` is bound to its answer, `(code, order)`, and the
 * factory it was asked of is never made. 0; -1 with an exception set; 1 (nothing defined) when a code cannot be
 * had this way, and the chunk is to be compiled.
 */
static int pycc_ext_chunk_define(PyObject *specs, PyObject *ns)
{
    Py_ssize_t i, n = PyTuple_Size(specs);
    PyObject *codes = PyTuple_New(n);
    int status = -1;

    if (codes == NULL) {
        return -1;
    }
    /* Every function's code first, its set constants in source order: when
     * one cannot be had, nothing is defined and the chunk is compiled. */
    for (i = 0; i < n; i++) {
        PyObject *spec = PyTuple_GetItem(specs, i);
        Py_ssize_t size = spec == NULL || !PyTuple_Check(spec) ? 0 : PyTuple_Size(spec);
        PyObject *code = size == 7   ? pycc_ext_chunk_reordered(PyTuple_GetItem(spec, 0), PyTuple_GetItem(spec, 6))
                         : size == 4 ? pycc_ext_chunk_reordered(PyTuple_GetItem(spec, 1), PyTuple_GetItem(spec, 3))
                                     : NULL;

        if (code == NULL || PyTuple_SetItem(codes, i, code) != 0) {
            PyErr_Clear();
            Py_DECREF(codes);
            return 1;
        }
    }
    if (pycc_ext_chunk_function_type_get() == NULL) {
        goto done;
    }
    for (i = 0; i < n; i++) {
        PyObject *spec = PyTuple_GetItem(specs, i);
        PyObject *annotations, *forward, *kwdefaults, *func;
        int stored;

        if (PyTuple_Size(spec) == 4) {
            /* A nested statement's scaffold (`src/fallback/nested.rs`):
             * what `__pycc_fallback_nestspec` answers, its code and the
             * order of its cells. */
            PyObject *value = PyTuple_Pack(2, PyTuple_GetItem(codes, i), PyTuple_GetItem(spec, 2));

            stored = value == NULL ? -1 : PyDict_SetItem(ns, PyTuple_GetItem(spec, 0), value);
            Py_XDECREF(value);
            if (stored != 0) {
                goto done;
            }
            continue;
        }
        annotations = PyTuple_GetItem(spec, 3);
        forward = PyTuple_GetItem(spec, 5);
        /* A fresh keyword-defaults dict per function, as `BUILD_CONST_KEY_MAP`
         * makes one each time the statement runs (the defaults tuple is a
         * constant, shared as `LOAD_CONST` shares it). */
        kwdefaults = PyTuple_GetItem(spec, 2);
        kwdefaults = kwdefaults == Py_None ? Py_NewRef(kwdefaults) : PyDict_Copy(kwdefaults);
        if (kwdefaults == NULL) {
            goto done;
        }
        func = PyObject_CallFunctionObjArgs(pycc_ext_chunk_function_type, PyTuple_GetItem(codes, i), ns,
                                            Py_None, PyTuple_GetItem(spec, 1), Py_None, kwdefaults, NULL);
        Py_DECREF(kwdefaults);
        if (func == NULL) {
            goto done;
        }
        if (annotations != Py_None && !PyTuple_Check(annotations)) {
            /* 3.14+'s lazy annotations (PEP 649): the `__annotate__`
             * function, made from its code in `ns` as `MAKE_FUNCTION`
             * makes it. */
            PyObject *annotate = PyObject_CallFunctionObjArgs(pycc_ext_chunk_function_type,
                                                              annotations, ns, NULL);

            stored = annotate == NULL ? -1 : PyObject_SetAttrString(func, "__annotate__", annotate);
            Py_XDECREF(annotate);
            if (stored != 0) {
                Py_DECREF(func);
                goto done;
            }
        } else if (annotations != Py_None) {
            PyObject *dict = PyDict_New();
            Py_ssize_t at, size = PyTuple_Size(annotations);

            stored = dict == NULL ? -1 : 0;
            for (at = 0; stored == 0 && at + 1 < size; at += 2) {
                stored = PyDict_SetItem(dict, PyTuple_GetItem(annotations, at),
                                        PyTuple_GetItem(annotations, at + 1));
            }
            if (stored == 0) {
                stored = PyObject_SetAttrString(func, "__annotations__", dict);
            }
            Py_XDECREF(dict);
            if (stored != 0) {
                Py_DECREF(func);
                goto done;
            }
        }
        stored = PyDict_SetItem(ns, PyTuple_GetItem(spec, 4), func);
        Py_DECREF(func);
        if (stored != 0) {
            goto done;
        }
        if (forward != Py_None) {
            PyObject *hook = PyDict_GetItemString(ns, "__pycc_forward__");
            PyObject *result;

            if (hook == NULL) {
                PyErr_SetString(PyExc_NameError, "name '__pycc_forward__' is not defined");
                goto done;
            }
            result = PyObject_Call(hook, forward, NULL);
            if (result == NULL) {
                goto done;
            }
            Py_DECREF(result);
        }
    }
    status = 0;
done:
    Py_DECREF(codes);
    return status;
}

/* `"__builtins__"`, interned once. */
static PyObject *pycc_ext_chunk_builtins_name = NULL;

static PyObject *pycc_ext_chunk_builtins_key(void)
{
    if (pycc_ext_chunk_builtins_name == NULL) {
        pycc_ext_chunk_builtins_name = PyUnicode_InternFromString("__builtins__");
    }
    return pycc_ext_chunk_builtins_name;
}

/* Binds `__builtins__` in `ns` when it is not, as `exec` does; 0, or -1
 * with an exception set. */
static int pycc_ext_chunk_bind_builtins(PyObject *ns)
{
    PyObject *key = pycc_ext_chunk_builtins_key();
    int has = key == NULL ? -1 : PyDict_Contains(ns, key);

    if (has == 0) {
        return PyDict_SetItem(ns, key, PyEval_GetBuiltins());
    }
    return has < 0 ? -1 : 0;
}

/* `exec(code, ns)`: `__builtins__` bound first, then the audit event. */
static int pycc_ext_chunk_exec(PyObject *code, PyObject *ns)
{
    PyObject *done;

    if (pycc_ext_chunk_bind_builtins(ns) != 0) {
        return -1;
    }
    if (PySys_Audit("exec", "O", code) < 0) {
        return -1;
    }
    done = PyEval_EvalCode(code, ns, ns);
    if (done == NULL) {
        return -1;
    }
    Py_DECREF(done);
    return 0;
}

/*
 * v0.12: texts a chunk's statements make CPython compile, compiled at build
 * time. An entry `(code, orders, (dataclass_texts, forward_texts))` maps,
 * in each dict (or None), every text the build expects to its code (or
 * `(code, orders)` when it holds set constants, rebuilt in source order as
 * a chunk's are), or to a list of those for a text made more than once:
 *
 *  - `dataclass_texts` (`precompile.py`'s `dcs`): the methods
 *    `dataclasses`' `_FuncBuilder` generates and runs with `exec(text,
 *    globals, ns)`, compiled as `exec` compiles a text;
 *  - `forward_texts` (`fwds`): the string annotations `typing` makes
 *    forward references of (a `TypedDict` or `NamedTuple` class body) and
 *    compiles with `compile(text, "<string>", "eval")`: in `annotationlib`
 *    on CPython 3.14+, in `typing` on 3.13.
 *
 * While such a chunk runs, that module's namespace binds the name (`exec`,
 * `compile`) to a builtin function of this file, which the module's code
 * then calls instead of the builtin: for a text in its dict, called as the
 * module calls it, from a frame with no `__future__` feature, while the
 * builtin is the one CPython started with, it raises the `compile` audit
 * event the builtin raises for the text and answers with the code -- `exec`
 * calls the builtin with the code -- each code used once; anything else it
 * hands to the builtin unchanged.
 *
 * One hook is bound for every chunk running at once (a chunk run inside
 * another, as an import of another module of the package, or one in
 * another thread): its `self` is the list of the running chunks' dicts,
 * each chunk adding its own when it starts and removing it when it ends,
 * and the last to end removes the hook (docs/DEVIATIONS.md). Each module
 * of a package has its own copy of the hooks' C functions, so a hook is
 * told by its docstring, which names the list's layout.
 */

/* The compiler flags a compiled text inherits from the frame compiling it
 * (`PyCF_MASK`: the `__future__` features). */
#define PYCC_SUBST_FUTURE_FLAGS 0x1FE0000L

/* Whether the frame calling a hook compiles a text with no `__future__`
 * feature, as the build compiled it. */
static int pycc_ext_subst_caller_plain(void)
{
    PyFrameObject *frame = PyEval_GetFrame();
    PyCodeObject *code;
    PyObject *flags;
    long value;

    if (frame == NULL) {
        return 0;
    }
    code = PyFrame_GetCode(frame);
    flags = code == NULL ? NULL : PyObject_GetAttrString((PyObject *)code, "co_flags");
    Py_XDECREF(code);
    value = flags == NULL ? -1 : PyLong_AsLong(flags);
    Py_XDECREF(flags);
    if (value == -1) {
        PyErr_Clear();
        return 0;
    }
    return (value & PYCC_SUBST_FUTURE_FLAGS) == 0;
}

/* One substituted builtin: its name, interned, and the builtin itself (the
 * `builtins` module's function of that name when the first such chunk ran,
 * NULL when it was something else then). */
typedef struct PyccSubst {
    const char *name;
    PyObject *key;
    PyObject *builtin;
    int known;
} PyccSubst;

static PyccSubst pycc_ext_subst_exec = {"exec", NULL, NULL, 0};
static PyccSubst pycc_ext_subst_compile = {"compile", NULL, NULL, 0};

/* `sub`'s builtin as the calling frame finds it (a new reference), or NULL
 * with an exception set (CPython's `NameError` when it has none). */
static PyObject *pycc_ext_subst_lookup(PyccSubst *sub)
{
    PyObject *builtins = PyEval_GetBuiltins();
    PyObject *found = NULL;

    if (builtins == NULL || PyMapping_GetOptionalItem(builtins, sub->key, &found) < 0) {
        return NULL;
    }
    if (found == NULL) {
        PyErr_Format(PyExc_NameError, "name '%s' is not defined", sub->name);
    }
    return found;
}

/* The code for `text` in `texts` (removed from it; a new reference), NULL
 * with no exception set when there is none to use, or NULL with an
 * exception set. Each entry is used once: a statement making a text another
 * made before gets code objects of its own, as from its own compile. */
static PyObject *pycc_ext_subst_take_from(PyObject *texts, PyObject *text)
{
    PyObject *entry, *code;
    Py_ssize_t left = 0;

    entry = PyDict_CheckExact(texts) ? PyDict_GetItemWithError(texts, text) : NULL;
    if (entry == NULL) {
        return NULL;
    }
    if (PyList_CheckExact(entry)) {
        left = PyList_Size(entry) - 1;
        entry = left < 0 ? NULL : PyList_GetItem(entry, left);
        if (entry == NULL) {
            PyErr_Clear();
            return NULL;
        }
    }
    code = PyTuple_Check(entry) && PyTuple_Size(entry) == 2
               ? pycc_ext_chunk_reordered(PyTuple_GetItem(entry, 0), PyTuple_GetItem(entry, 1))
               : Py_NewRef(entry);
    if (code == NULL) {
        /* Not rebuilt: the text is compiled instead. */
        PyErr_Clear();
        return NULL;
    }
    if ((left > 0 ? PySequence_DelItem(PyDict_GetItemWithError(texts, text), left)
                  : PyDict_DelItem(texts, text))
        != 0) {
        Py_DECREF(code);
        return NULL;
    }
    return code;
}

/* The code for `text` in one of the running chunks' dicts `tables`, the
 * latest first (a new reference), NULL with no exception set when there is
 * none to use (the text is compiled), or NULL with an exception set; the
 * `compile` audit event raised. */
static PyObject *pycc_ext_subst_take(PyObject *tables, PyObject *text)
{
    PyObject *code = NULL, *bytes;
    Py_ssize_t i;

    if (!PyList_CheckExact(tables) || !PyUnicode_CheckExact(text) || !pycc_ext_subst_caller_plain()) {
        return NULL;
    }
    for (i = PyList_Size(tables) - 1; i >= 0 && code == NULL; i--) {
        code = pycc_ext_subst_take_from(PyList_GetItem(tables, i), text);
        if (code == NULL && PyErr_Occurred()) {
            return NULL;
        }
    }
    if (code == NULL) {
        return NULL;
    }
    bytes = PyUnicode_AsUTF8String(text);
    if (bytes == NULL || PySys_Audit("compile", "Os", bytes, "<string>") < 0) {
        Py_XDECREF(bytes);
        Py_DECREF(code);
        return NULL;
    }
    Py_DECREF(bytes);
    return code;
}

/* `exec` as `dataclasses` finds it: `exec(text, globals, ns)`. */
static PyObject *pycc_ext_subst_exec_call(PyObject *tables, PyObject *const *args, Py_ssize_t nargs,
                                          PyObject *kwnames)
{
    PyObject *exec = pycc_ext_subst_lookup(&pycc_ext_subst_exec);
    PyObject *code = NULL, *result;

    if (exec == NULL) {
        return NULL;
    }
    if (nargs == 3 && kwnames == NULL && exec == pycc_ext_subst_exec.builtin) {
        code = pycc_ext_subst_take(tables, args[0]);
        if (code == NULL && PyErr_Occurred()) {
            Py_DECREF(exec);
            return NULL;
        }
    }
    if (code != NULL) {
        PyObject *argv[3] = {code, args[1], args[2]};

        result = PyObject_Vectorcall(exec, argv, 3, NULL);
        Py_DECREF(code);
    } else {
        result = PyObject_Vectorcall(exec, args, (size_t)nargs, kwnames);
    }
    Py_DECREF(exec);
    return result;
}

/* Whether `obj` is the `str` equal to `ascii`. */
static int pycc_ext_subst_is(PyObject *obj, const char *ascii)
{
    return PyUnicode_CheckExact(obj) && PyUnicode_CompareWithASCIIString(obj, ascii) == 0;
}

/* `compile` as `annotationlib` (`typing` on 3.13) finds it:
 * `compile(text, "<string>", "eval")`. */
static PyObject *pycc_ext_subst_compile_call(PyObject *tables, PyObject *const *args, Py_ssize_t nargs,
                                             PyObject *kwnames)
{
    PyObject *compile = pycc_ext_subst_lookup(&pycc_ext_subst_compile);
    PyObject *code = NULL, *result;

    if (compile == NULL) {
        return NULL;
    }
    if (nargs == 3 && kwnames == NULL && compile == pycc_ext_subst_compile.builtin
        && pycc_ext_subst_is(args[1], "<string>") && pycc_ext_subst_is(args[2], "eval")) {
        code = pycc_ext_subst_take(tables, args[0]);
        if (code == NULL && PyErr_Occurred()) {
            Py_DECREF(compile);
            return NULL;
        }
    }
    result = code != NULL ? code : PyObject_Vectorcall(compile, args, (size_t)nargs, kwnames);
    Py_DECREF(compile);
    return result;
}

#define PYCC_SUBST_HOOK_DOC "pycc: a module's builtin while pre-compiled chunks run (self: their tables)"

static PyMethodDef pycc_ext_subst_exec_def = {"exec", (PyCFunction)(void (*)(void))pycc_ext_subst_exec_call,
                                              METH_FASTCALL | METH_KEYWORDS, PYCC_SUBST_HOOK_DOC};
static PyMethodDef pycc_ext_subst_compile_def = {"compile",
                                                 (PyCFunction)(void (*)(void))pycc_ext_subst_compile_call,
                                                 METH_FASTCALL | METH_KEYWORDS, PYCC_SUBST_HOOK_DOC};

/* The running chunks' tables if `bound` is a pycc extension's hook (a
 * borrowed reference), else NULL (no exception set). */
static PyObject *pycc_ext_subst_hook_tables(PyObject *bound)
{
    PyObject *self, *doc;
    int same;

    if (bound == NULL || !PyCFunction_Check(bound)) {
        return NULL;
    }
    /* A builtin with no `self` (a static method's function) is no hook. */
    self = PyCFunction_GetSelf(bound);
    if (self == NULL || !PyList_CheckExact(self)) {
        PyErr_Clear();
        return NULL;
    }
    doc = PyObject_GetAttrString(bound, "__doc__");
    same = doc != NULL && pycc_ext_subst_is(doc, PYCC_SUBST_HOOK_DOC);
    Py_XDECREF(doc);
    PyErr_Clear();
    return same ? self : NULL;
}

/* Whether `found` is CPython's builtin `sub`: the `builtins` module's own
 * function of that name (not another of its functions bound under it). */
static int pycc_ext_subst_is_builtin(PyObject *found, PyObject *builtins, PyccSubst *sub)
{
    PyObject *name;
    int same;

    if (found == NULL || !PyCFunction_Check(found) || PyCFunction_GetSelf(found) != builtins) {
        return 0;
    }
    name = PyObject_GetAttrString(found, "__name__");
    same = name != NULL && pycc_ext_subst_is(name, sub->name);
    Py_XDECREF(name);
    PyErr_Clear();
    return same;
}

/* One chunk's table added to a hook: the hook, and the table. */
typedef struct PyccSubstBinding {
    PyObject *hook;
    PyObject *texts;
} PyccSubstBinding;

/* Adds `texts` to the running chunks' tables of the hook (`def`) bound as
 * `sub`'s name in module `module`'s namespace, binding one when the module
 * is imported and binds no such name; `binding->hook` stays NULL (no
 * exception set) when nothing was bound. */
static void pycc_ext_subst_install(PyccSubstBinding *binding, const char *module, PyccSubst *sub,
                                   PyMethodDef *def, PyObject *texts)
{
    PyObject *name, *mod, *dict, *bound, *tables, *hook = NULL;

    if (texts == NULL || texts == Py_None) {
        return;
    }
    if (sub->key == NULL) {
        sub->key = PyUnicode_InternFromString(sub->name);
        if (sub->key == NULL) {
            PyErr_Clear();
            return;
        }
    }
    if (!sub->known) {
        PyObject *builtins = PyImport_ImportModule("builtins");
        PyObject *found = builtins == NULL ? NULL : PyObject_GetAttr(builtins, sub->key);

        if (pycc_ext_subst_is_builtin(found, builtins, sub)) {
            sub->builtin = Py_NewRef(found);
        }
        Py_XDECREF(found);
        Py_XDECREF(builtins);
        PyErr_Clear();
        sub->known = 1;
    }
    if (sub->builtin == NULL) {
        return;
    }
    name = PyUnicode_FromString(module);
    mod = name == NULL ? NULL : PyImport_GetModule(name);
    Py_XDECREF(name);
    dict = mod != NULL && PyModule_Check(mod) ? PyModule_GetDict(mod) : NULL;
    bound = dict == NULL ? NULL : PyDict_GetItemWithError(dict, sub->key);
    if (dict != NULL && bound == NULL && !PyErr_Occurred()) {
        tables = PyList_New(0);
        hook = tables == NULL ? NULL : PyCFunction_NewEx(def, tables, NULL);
        Py_XDECREF(tables);
        if (hook != NULL && PyDict_SetItem(dict, sub->key, hook) != 0) {
            Py_CLEAR(hook);
        }
    } else if (pycc_ext_subst_hook_tables(bound) != NULL) {
        hook = Py_NewRef(bound);
    }
    tables = pycc_ext_subst_hook_tables(hook);
    if (tables != NULL && PyList_Append(tables, texts) == 0) {
        binding->hook = hook;
        binding->texts = Py_NewRef(texts);
    } else {
        Py_XDECREF(hook);
    }
    Py_XDECREF(mod);
    PyErr_Clear();
}

/* Removes `binding`'s table from its hook's, and the hook from module
 * `module`'s namespace when no other chunk's table is left and it is still
 * bound there, keeping any exception set. */
static void pycc_ext_subst_remove(PyccSubstBinding *binding, const char *module, PyccSubst *sub)
{
    PyObject *raised, *tables;
    Py_ssize_t i;

    if (binding->hook == NULL) {
        return;
    }
    raised = PyErr_GetRaisedException();
    tables = pycc_ext_subst_hook_tables(binding->hook);
    for (i = tables == NULL ? -1 : PyList_Size(tables) - 1; i >= 0; i--) {
        if (PyList_GetItem(tables, i) == binding->texts) {
            if (PySequence_DelItem(tables, i) != 0) {
                PyErr_Clear();
            }
            break;
        }
    }
    if (tables != NULL && PyList_Size(tables) == 0) {
        PyObject *name = PyUnicode_FromString(module);
        PyObject *mod = name == NULL ? NULL : PyImport_GetModule(name);
        PyObject *dict = mod != NULL && PyModule_Check(mod) ? PyModule_GetDict(mod) : NULL;
        PyObject *bound = dict == NULL ? NULL : PyDict_GetItemWithError(dict, sub->key);

        if (bound == binding->hook && PyDict_DelItem(dict, sub->key) != 0) {
            PyErr_Clear();
        }
        Py_XDECREF(mod);
        Py_XDECREF(name);
    }
    Py_CLEAR(binding->hook);
    Py_CLEAR(binding->texts);
    PyErr_Clear();
    PyErr_SetRaisedException(raised);
}

/* Runs chunk `index` in `ns` (as `exec(<its code>, ns)` when `as_exec`:
 * the audit event and `__builtins__`); 0, or -1 with an exception set. */
static int pycc_ext_chunk_run_in(Py_ssize_t index, PyObject *ns, int as_exec)
{
    const struct pycc_ext_chunk *chunk;
    PyObject *code = NULL;
    PyObject *texts = NULL;
    PyccSubstBinding dc = {NULL, NULL}, fwd = {NULL, NULL};
    const char *fwd_module = Py_Version >= 0x030E0000 ? "annotationlib" : "typing";
    int status;

    if (index < 0 || index >= pycc_ext_chunk_count) {
        PyErr_SetString(PyExc_SystemError, "pycc: no such pre-compiled chunk");
        return -1;
    }
    chunk = &pycc_ext_chunks[index];
    if (chunk->precompiled && pycc_ext_chunks_usable()) {
        /* Owned while it is used: Python code it runs (an audit hook, a
         * re-import of this module) may finish another body run, which
         * frees the table. */
        PyObject *entry = Py_XNewRef(pycc_ext_chunk_entry(index));

        if (entry != NULL && PyTuple_Size(entry) == 4) {
            /* A signature wrapper's function (`pycc_ext_signature_publish`
             * makes it), not the chunk's code: the source is compiled. */
            Py_CLEAR(entry);
        }
        if (entry != NULL && PyTuple_Size(entry) == 1) {
            status = as_exec && pycc_ext_chunk_bind_builtins(ns) != 0
                         ? -1
                         : pycc_ext_chunk_define(PyTuple_GetItem(entry, 0), ns);
            if (status <= 0) {
                Py_DECREF(entry);
                return status;
            }
            Py_CLEAR(entry);
        }
        code = entry == NULL ? NULL : pycc_ext_chunk_entry_code(entry);
        if (code != NULL && PyTuple_Size(entry) == 3) {
            texts = Py_NewRef(PyTuple_GetItem(entry, 2));
        }
        Py_XDECREF(entry);
        /* Unreadable after all: compile the source instead. */
    }
    if (code == NULL) {
        code = pycc_ext_chunk_compile(chunk);
        if (code == NULL) {
            return -1;
        }
    }
    if (texts != NULL) {
        if (PyTuple_Check(texts) && PyTuple_Size(texts) == 2) {
            pycc_ext_subst_install(&dc, "dataclasses", &pycc_ext_subst_exec, &pycc_ext_subst_exec_def,
                                   PyTuple_GetItem(texts, 0));
            pycc_ext_subst_install(&fwd, fwd_module, &pycc_ext_subst_compile, &pycc_ext_subst_compile_def,
                                   PyTuple_GetItem(texts, 1));
        }
        Py_DECREF(texts);
    }
    if (as_exec) {
        status = pycc_ext_chunk_exec(code, ns);
    } else {
        PyObject *done = PyEval_EvalCode(code, ns, ns);

        status = done == NULL ? -1 : 0;
        Py_XDECREF(done);
    }
    pycc_ext_subst_remove(&fwd, fwd_module, &pycc_ext_subst_compile);
    pycc_ext_subst_remove(&dc, "dataclasses", &pycc_ext_subst_exec);
    Py_DECREF(code);
    return status;
}

/* `__pycc_run__(k)`: runs chunk `k` in the namespace (`self`), as
 * `exec(<its code>, ns)` does. */
static PyObject *pycc_ext_run(PyObject *self, PyObject *arg)
{
    Py_ssize_t index = PyLong_AsSsize_t(arg);

    if (index == -1 && PyErr_Occurred()) {
        return NULL;
    }
    if (pycc_ext_chunk_run_in(index, self, 1) != 0) {
        return NULL;
    }
    Py_RETURN_NONE;
}

static PyMethodDef pycc_ext_run_def = {"__pycc_run__", pycc_ext_run, METH_O, NULL};

/* Binds the hook in `module`'s namespace for the body; 0, or -1 with an
 * exception set. */
static int pycc_ext_code_install(PyObject *module)
{
    PyObject *dict = PyModule_GetDict(module);
    PyObject *hook = PyCFunction_NewEx(&pycc_ext_run_def, dict, NULL);
    int status;

    if (hook == NULL) {
        return -1;
    }
    status = PyDict_SetItemString(dict, "__pycc_run__", hook);
    Py_DECREF(hook);
    return status;
}

/* Removes the hook once the body has run, keeping any exception the body
 * raised. */
static void pycc_ext_code_remove(PyObject *module)
{
    PyObject *raised = PyErr_GetRaisedException();
    PyObject *dict = PyModule_GetDict(module);

    if (PyDict_DelItemString(dict, "__pycc_run__") != 0) {
        PyErr_Clear();
    }
    /* No chunk is read after the body: the code objects it made live on in
     * what it defined. */
    Py_CLEAR(pycc_ext_chunk_table);
    PyErr_SetRaisedException(raised);
}

