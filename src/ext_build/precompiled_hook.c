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
 * v0.12: a chunk's dataclasses' methods compiled at build time
 * (`precompile.py`'s `dcs`): its entry is `(code, orders, texts)`, `texts`
 * mapping each text the build expects `dataclasses` to generate to its code,
 * compiled as `exec` compiles it (with its set constants' orders, as a
 * chunk's). While such a chunk runs, `dataclasses`' namespace binds `exec`
 * to `pycc_ext_dc_exec`, which the module's `_FuncBuilder` then calls
 * instead of the builtin: for a text in `texts` it raises the `compile`
 * audit event `exec` of the text raises and calls the builtin `exec` with
 * the code, which runs it as it would the text's; anything else it hands to
 * the builtin unchanged.
 *
 * One hook is bound for every chunk running at once (a chunk run inside
 * another, as an import of another module of the package, or one in
 * another thread): its `self` is the list of the running chunks' tables,
 * each chunk adding its own when it starts and removing it when it ends,
 * and the last to end removes the hook (docs/DEVIATIONS.md). Each module
 * of a package has its own copy of the hook's C function, so a hook is
 * told by its docstring, which names the list's layout.
 */

/* `"exec"`, interned once. */
static PyObject *pycc_ext_dc_exec_name = NULL;
/* The builtin `exec` (`builtins.exec` when the first such chunk ran, if it
 * was the builtin function then); NULL when it was not. */
static PyObject *pycc_ext_dc_builtin_exec = NULL;
static int pycc_ext_dc_builtin_known = 0;

/* The compiler flags `exec` of a text inherits from the frame calling it
 * (`PyCF_MASK`: the `__future__` features). */
#define PYCC_DC_FUTURE_FLAGS 0x1FE0000L

/* Whether the frame calling the hook compiles a text with no `__future__`
 * feature, as the build compiled it. */
static int pycc_ext_dc_caller_plain(void)
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
    return (value & PYCC_DC_FUTURE_FLAGS) == 0;
}

/* The code for `text` in one of the running chunks' `tables` (removed from
 * it: each is used once, a second class generating the same text gets its
 * own code objects, as from its own `exec`; a new reference), NULL with no
 * exception set when there is none, or NULL with an exception set. */
static PyObject *pycc_ext_dc_take(PyObject *tables, PyObject *text)
{
    Py_ssize_t i = PyList_CheckExact(tables) ? PyList_Size(tables) : 0;

    while (i-- > 0) {
        PyObject *texts = PyList_GetItem(tables, i);
        PyObject *code = PyDict_CheckExact(texts) ? PyDict_GetItemWithError(texts, text) : NULL;

        if (code == NULL) {
            if (PyErr_Occurred()) {
                return NULL;
            }
            continue;
        }
        /* Its set constants rebuilt in source order, as the chunks'. */
        code = PyTuple_Check(code) && PyTuple_Size(code) == 2
                   ? pycc_ext_chunk_reordered(PyTuple_GetItem(code, 0), PyTuple_GetItem(code, 1))
                   : Py_NewRef(code);
        if (code == NULL) {
            /* Not rebuilt: the text is compiled instead. */
            PyErr_Clear();
            return NULL;
        }
        if (PyDict_DelItem(texts, text) != 0) {
            Py_DECREF(code);
            return NULL;
        }
        return code;
    }
    return NULL;
}

/* `exec` as `dataclasses` would find it: the hook, bound in its namespace
 * as `exec`, stands in front of the calling frame's builtins. */
static PyObject *pycc_ext_dc_exec(PyObject *tables, PyObject *const *args, Py_ssize_t nargs,
                                  PyObject *kwnames)
{
    PyObject *builtins = PyEval_GetBuiltins();
    PyObject *exec = NULL;
    PyObject *code = NULL;
    PyObject *result;

    if (builtins == NULL || PyMapping_GetOptionalItem(builtins, pycc_ext_dc_exec_name, &exec) < 0) {
        return NULL;
    }
    if (exec == NULL) {
        PyErr_SetString(PyExc_NameError, "name 'exec' is not defined");
        return NULL;
    }
    if (nargs == 3 && kwnames == NULL && exec == pycc_ext_dc_builtin_exec
        && PyUnicode_CheckExact(args[0]) && pycc_ext_dc_caller_plain()) {
        code = pycc_ext_dc_take(tables, args[0]);
        if (code == NULL && PyErr_Occurred()) {
            Py_DECREF(exec);
            return NULL;
        }
    }
    if (code != NULL) {
        PyObject *bytes = PyUnicode_AsUTF8String(args[0]);
        PyObject *argv[3] = {code, args[1], args[2]};

        if (bytes == NULL || PySys_Audit("compile", "Os", bytes, "<string>") < 0) {
            Py_XDECREF(bytes);
            Py_DECREF(code);
            Py_DECREF(exec);
            return NULL;
        }
        Py_DECREF(bytes);
        result = PyObject_Vectorcall(exec, argv, 3, NULL);
        Py_DECREF(code);
    } else {
        result = PyObject_Vectorcall(exec, args, (size_t)nargs, kwnames);
    }
    Py_DECREF(exec);
    return result;
}

#define PYCC_DC_HOOK_DOC "pycc: dataclasses' exec while pre-compiled chunks run (self: their tables)"

static PyMethodDef pycc_ext_dc_exec_def = {"exec", (PyCFunction)(void (*)(void))pycc_ext_dc_exec,
                                           METH_FASTCALL | METH_KEYWORDS, PYCC_DC_HOOK_DOC};

/* The running chunks' tables if `bound` is a pycc extension's hook (a
 * borrowed reference), else NULL (no exception set). */
static PyObject *pycc_ext_dc_hook_tables(PyObject *bound)
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
    same = doc != NULL && PyUnicode_Check(doc) && PyUnicode_CompareWithASCIIString(doc, PYCC_DC_HOOK_DOC) == 0;
    Py_XDECREF(doc);
    PyErr_Clear();
    return same ? self : NULL;
}

/* Whether `exec` is CPython's builtin `exec`: the `builtins` module's own
 * function of that name (not another of its functions bound as `exec`). */
static int pycc_ext_dc_is_builtin_exec(PyObject *exec, PyObject *builtins)
{
    PyObject *name;
    int same;

    if (exec == NULL || !PyCFunction_Check(exec) || PyCFunction_GetSelf(exec) != builtins) {
        return 0;
    }
    name = PyObject_GetAttrString(exec, "__name__");
    same = name != NULL && PyUnicode_Check(name) && PyUnicode_CompareWithASCIIString(name, "exec") == 0;
    Py_XDECREF(name);
    PyErr_Clear();
    return same;
}

/* Adds `texts` to the running chunks' tables, binding the hook as `exec`
 * in `dataclasses`' namespace when it is imported and binds no `exec`
 * (the hook found bound is used); returns the hook (to remove `texts`), or
 * NULL (no exception set) when nothing was bound. */
static PyObject *pycc_ext_dc_install(PyObject *texts)
{
    PyObject *name, *module, *dict, *bound, *tables, *hook = NULL;

    if (pycc_ext_dc_exec_name == NULL) {
        pycc_ext_dc_exec_name = PyUnicode_InternFromString("exec");
        if (pycc_ext_dc_exec_name == NULL) {
            PyErr_Clear();
            return NULL;
        }
    }
    if (!pycc_ext_dc_builtin_known) {
        PyObject *mod = PyImport_ImportModule("builtins");
        PyObject *exec = mod == NULL ? NULL : PyObject_GetAttr(mod, pycc_ext_dc_exec_name);

        if (pycc_ext_dc_is_builtin_exec(exec, mod)) {
            pycc_ext_dc_builtin_exec = Py_NewRef(exec);
        }
        Py_XDECREF(exec);
        Py_XDECREF(mod);
        PyErr_Clear();
        pycc_ext_dc_builtin_known = 1;
    }
    if (pycc_ext_dc_builtin_exec == NULL) {
        return NULL;
    }
    name = PyUnicode_FromString("dataclasses");
    module = name == NULL ? NULL : PyImport_GetModule(name);
    Py_XDECREF(name);
    if (module == NULL || !PyModule_Check(module)) {
        Py_XDECREF(module);
        PyErr_Clear();
        return NULL;
    }
    dict = PyModule_GetDict(module);
    Py_DECREF(module);
    bound = dict == NULL ? NULL : PyDict_GetItemWithError(dict, pycc_ext_dc_exec_name);
    if (bound == NULL && !PyErr_Occurred()) {
        tables = PyList_New(0);
        hook = tables == NULL ? NULL : PyCFunction_NewEx(&pycc_ext_dc_exec_def, tables, NULL);
        Py_XDECREF(tables);
        if (hook != NULL && PyDict_SetItem(dict, pycc_ext_dc_exec_name, hook) != 0) {
            Py_CLEAR(hook);
        }
    } else if ((tables = pycc_ext_dc_hook_tables(bound)) != NULL) {
        hook = Py_NewRef(bound);
    }
    tables = hook == NULL ? NULL : pycc_ext_dc_hook_tables(hook);
    if (tables == NULL || PyList_Append(tables, texts) != 0) {
        Py_CLEAR(hook);
    }
    PyErr_Clear();
    return hook;
}

/* Removes `texts` from `hook`'s tables, and the hook from `dataclasses`'
 * namespace when no other chunk's tables are left and it is still bound
 * there, keeping any exception set. */
static void pycc_ext_dc_remove(PyObject *hook, PyObject *texts)
{
    PyObject *raised = PyErr_GetRaisedException();
    PyObject *tables = pycc_ext_dc_hook_tables(hook);
    Py_ssize_t i = tables == NULL ? 0 : PyList_Size(tables);

    while (i-- > 0) {
        if (PyList_GetItem(tables, i) == texts) {
            if (PySequence_DelItem(tables, i) != 0) {
                PyErr_Clear();
            }
            break;
        }
    }
    if (tables != NULL && PyList_Size(tables) == 0) {
        PyObject *name = PyUnicode_FromString("dataclasses");
        PyObject *module = name == NULL ? NULL : PyImport_GetModule(name);
        PyObject *dict = module != NULL && PyModule_Check(module) ? PyModule_GetDict(module) : NULL;
        PyObject *bound = dict == NULL ? NULL : PyDict_GetItemWithError(dict, pycc_ext_dc_exec_name);

        if (bound == hook && PyDict_DelItem(dict, pycc_ext_dc_exec_name) != 0) {
            PyErr_Clear();
        }
        Py_XDECREF(module);
        Py_XDECREF(name);
    }
    Py_DECREF(hook);
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
    PyObject *hook = NULL;
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
        hook = pycc_ext_dc_install(texts);
    }
    if (as_exec) {
        status = pycc_ext_chunk_exec(code, ns);
    } else {
        PyObject *done = PyEval_EvalCode(code, ns, ns);

        status = done == NULL ? -1 : 0;
        Py_XDECREF(done);
    }
    if (hook != NULL) {
        pycc_ext_dc_remove(hook, texts);
    }
    Py_XDECREF(texts);
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

