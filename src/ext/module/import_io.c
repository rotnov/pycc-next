/*
 * Part 1 of #1026: the module-import helper compiled code calls for a
 * foreign `import numpy`.
 *
 * A thin `PyImport_ImportModule` wrapper and nothing more -- it returns a
 * new reference to the imported module, or NULL with the CPython exception
 * (a `ModuleNotFoundError` for a module that is not on the interpreter's
 * `sys.path`) already set, which is exactly the convention the generated
 * call site checks for. Every *decision* about the import stays with
 * CPython, keeping this file's "only moves `PyObject*`s" division of labour
 * with `pycc_rt` intact.
 *
 * Not `static`, unlike every other helper here: LLVM-generated code
 * declares and calls it by this name (`EXT_OBJ_IMPORT_SYMBOL` in
 * `crates/pycc_codegen/src/ext.rs`).
 *
 * The generated module body stores the returned reference in a
 * module-level global, which owns it: a re-import or rebind of that
 * global releases the previous value, and the last value is
 * never released, because the artifact has no teardown hook to release it
 * from; the module object is in the interpreter's `sys.modules` for that
 * whole lifetime anyway. `docs/RUNTIME.md` records the rule.
 */
PyObject *pycc_ext_obj_import(const char *name)
{
    return PyImport_ImportModule(name);
}

/*
 * #1278: raise CPython's `IMPORT_FROM` `ImportError` for `name` missing from
 * `module`, and return NULL. `pycc_ext_obj_import_from` below documents the
 * contract; this only builds the exception.
 */
static PyObject *pycc_ext_import_from_error(PyObject *module, PyObject *name)
{
    PyObject *pkgname = NULL;
    PyObject *shown = NULL;
    PyObject *pkgpath = NULL;
    PyObject *message = NULL;
    PyObject *raised = NULL;

    if (PyObject_GetOptionalAttrString(module, "__name__", &pkgname) < 0) {
        return NULL;
    }
    if (pkgname != NULL && !PyUnicode_Check(pkgname)) {
        Py_CLEAR(pkgname);
    }
    shown = (pkgname != NULL) ? Py_NewRef(pkgname)
                              : PyUnicode_FromString("<unknown module name>");
    if (shown == NULL) {
        goto done;
    }
    if (PyModule_Check(module)) {
        pkgpath = PyModule_GetFilenameObject(module);
        if (pkgpath == NULL) {
            if (!PyErr_ExceptionMatches(PyExc_SystemError)) {
                goto done;
            }
            /* A module without `__file__`, such as the builtin `itertools`. */
            PyErr_Clear();
        }
    }
    if (pkgpath == NULL || !PyUnicode_Check(pkgpath)) {
        Py_CLEAR(pkgpath);
        message = PyUnicode_FromFormat("cannot import name %R from %R (unknown location)",
                                       name, shown);
    } else {
        message = PyUnicode_FromFormat("cannot import name %R from %R (%S)",
                                       name, shown, pkgpath);
    }
    if (message == NULL) {
        goto done;
    }
    PyErr_SetImportError(message, pkgname, pkgpath);
    /*
     * `PyErr_SetImportError` leaves `name_from` unset; CPython's own
     * `IMPORT_FROM` sets it, and its "Did you mean" suggestion reads it.
     */
    raised = PyErr_GetRaisedException();
    if (raised != NULL && PyObject_SetAttrString(raised, "name_from", name) < 0) {
        Py_DECREF(raised);
        goto done;
    }
    PyErr_SetRaisedException(raised);
done:
    Py_XDECREF(message);
    Py_XDECREF(pkgpath);
    Py_XDECREF(shown);
    Py_XDECREF(pkgname);
    return NULL;
}

/*
 * One CPython `IMPORT_FROM` of `name` from `module` (#1278, shared with
 * #1381's dotted walk): a new reference to the attribute when `module` has
 * one; on a missing attribute only, the `sys.modules` entry
 * `<module.__name__>.<name>`; otherwise CPython's `cannot import name`
 * `ImportError` (`pycc_ext_import_from_error`). Any other attribute-lookup
 * failure propagates unchanged. Returns NULL with the exception set on
 * failure; borrows both arguments.
 */
static PyObject *pycc_ext_import_from_step(PyObject *module, PyObject *name)
{
    PyObject *pkgname = NULL;
    PyObject *fullname = NULL;
    PyObject *value = NULL;

    if (PyObject_GetOptionalAttr(module, name, &value) != 0) {
        /* Found (1) or a non-`AttributeError` failure (-1): done either way. */
        return value;
    }
    if (PyObject_GetOptionalAttrString(module, "__name__", &pkgname) < 0) {
        return NULL;
    }
    if (pkgname != NULL && PyUnicode_Check(pkgname)) {
        fullname = PyUnicode_FromFormat("%U.%U", pkgname, name);
        if (fullname == NULL) {
            goto done;
        }
        value = PyImport_GetModule(fullname);
        if (value != NULL || PyErr_Occurred()) {
            goto done;
        }
    }
    pycc_ext_import_from_error(module, name);
done:
    Py_XDECREF(fullname);
    Py_XDECREF(pkgname);
    return value;
}

/*
 * #1278: the helper compiled code calls for each name of a foreign
 * `from itertools import product, chain`.
 *
 * It mirrors CPython 3.14's `IMPORT_NAME` with a fromlist followed by one
 * `IMPORT_FROM`, and returns a new reference to the value bound to
 * `fromlist[index]`, or NULL with the CPython exception set:
 *
 * 1. The import runs `builtins.__import__(module, None, None, fromlist, 0)`
 *    with the statement's *whole* fromlist, as `IMPORT_NAME` does, so a
 *    package whose submodule import sets another listed attribute still
 *    works. It runs once per name rather than once per statement; that
 *    repeats no binding and raises nothing new, because
 *    `_handle_fromlist` skips every name already bound. The two
 *    observable differences -- an overridden `__import__` sees N calls,
 *    and a failed finder lookup for a missing `pkg.<name>` repeats -- are
 *    recorded in `docs/RUNTIME.md`.
 * 2. `IMPORT_FROM`: the attribute when the module has one. On a missing
 *    attribute only, the `sys.modules` entry `<mod.__name__>.<name>`, so a
 *    submodule the fromlist import just loaded (`from xml import dom`) is
 *    found. Any other attribute-lookup failure propagates unchanged.
 * 3. Otherwise CPython's own `ImportError`: `cannot import name 'n' from
 *    'm' (<__file__>)`, or `(unknown location)` when the module has no
 *    file, with `.name`, `.path` and `.name_from` set as CPython sets them.
 *    The message, `.name` and the fallback key come from the module
 *    object's `__name__`, not from the requested string, as CPython's do.
 *    CPython's circular-import and stdlib-shadowing variants of the
 *    message are not reproduced; `docs/RUNTIME.md` records both.
 *
 * `level` is `0` for an absolute import, which passes `None` as the
 * globals. A relative one (#1366; `level` is its dot count, and `module` is
 * the name as written after the dots, `""` for `from . import x`) is only
 * emitted under `pycc build --ext --foreign-relative-imports`, and only as a
 * module-body item, so it runs inside `Py_mod_exec`: it passes the executing
 * module's own dict as the globals, which CPython's `_calc___package__`
 * reads the package from, so a missing sibling, a climb beyond the
 * top-level package and a top-level import with no parent package raise
 * exactly what the `.py` form raises. Outside an exec (never emitted) it
 * raises `SystemError` rather than resolve against the wrong package.
 *
 * Not `static`: LLVM-generated code declares and calls it by this name
 * (`EXT_OBJ_IMPORT_FROM_SYMBOL` in `crates/pycc_codegen/src/ext.rs`).
 * `module` and every `fromlist` entry are NUL-terminated UTF-8 constants
 * the artifact owns; `index` is always inside `[0, nfrom)` because the
 * compiler emits both. The returned reference is owned by the module
 * global that binds it, on the same rule as `pycc_ext_obj_import`: a
 * re-import or rebind releases it, and the last value is never released.
 */
PyObject *pycc_ext_obj_import_from(const char *module_name, const char *const *fromlist,
                                   long long nfrom, long long index, long long level)
{
    PyObject *globals = Py_None;
    PyObject *import = NULL;
    PyObject *names = NULL;
    PyObject *module = NULL;
    PyObject *name = NULL;
    PyObject *value = NULL;
    long long i;
    int found;

    if (level > 0) {
        PyObject *target = NULL;

        if (pycc_ext_exec_target_key != NULL) {
            target = (PyObject *)PyThread_tss_get(pycc_ext_exec_target_key);
        }
        if (target == NULL) {
            PyErr_SetString(PyExc_SystemError, "pycc: relative import outside Py_mod_exec");
            return NULL;
        }
        /* Borrowed: the module outlives its own exec. */
        globals = PyModule_GetDict(target);
        if (globals == NULL) {
            return NULL;
        }
    }
    found = PyDict_GetItemStringRef(PyEval_GetBuiltins(), "__import__", &import);
    if (found < 0) {
        return NULL;
    }
    if (found == 0) {
        PyErr_SetString(PyExc_ImportError, "__import__ not found");
        return NULL;
    }
    names = PyTuple_New((Py_ssize_t)nfrom);
    if (names == NULL) {
        goto done;
    }
    for (i = 0; i < nfrom; i++) {
        PyObject *item = PyUnicode_FromString(fromlist[i]);
        if (item == NULL) {
            goto done;
        }
        /* Steals `item`, even on failure. */
        if (PyTuple_SetItem(names, (Py_ssize_t)i, item) < 0) {
            goto done;
        }
    }
    module = PyObject_CallFunction(import, "sOOOi", module_name, globals, Py_None, names,
                                   (int)level);
    if (module == NULL) {
        goto done;
    }
    name = PyUnicode_FromString(fromlist[index]);
    if (name == NULL) {
        goto done;
    }
    value = pycc_ext_import_from_step(module, name);
done:
    Py_XDECREF(name);
    Py_XDECREF(module);
    Py_XDECREF(names);
    Py_XDECREF(import);
    return value;
}

/*
 * #1381 (Part 3 of #1138): the helper compiled code calls for a foreign
 * plain dotted import, `import a.b.c` or `import a.b.c as d`. An undotted
 * `import a` keeps `pycc_ext_obj_import`.
 *
 * It mirrors CPython 3.14's bytecode for both forms and returns a new
 * reference to the bound object, or NULL with the CPython exception set:
 *
 * 1. `IMPORT_NAME`: `builtins.__import__(name, None, None, None, 0)`. With
 *    no fromlist, `__import__` imports every package on the way to the leaf
 *    and returns the *root* package, which `import a.b.c` binds to `a`
 *    (`bind_root` non-zero): the call's result is returned as is.
 * 2. Otherwise `import a.b.c as d` walks from the root with one
 *    `IMPORT_FROM` per remaining segment (`b`, then `c`), exactly as the
 *    compiled `IMPORT_FROM`/`SWAP`/`POP_TOP` chain does, through the step
 *    `pycc_ext_obj_import_from` shares: the attribute, else the
 *    `sys.modules` entry `<__name__>.<segment>`, else CPython's `cannot
 *    import name` `ImportError`. So a package whose `__init__` rebinds a
 *    submodule's name binds that attribute, as in CPython.
 *
 * Not `static`: LLVM-generated code declares and calls it by this name
 * (`EXT_OBJ_IMPORT_DOTTED_SYMBOL` in `crates/pycc_codegen/src/ext.rs`).
 * `name` is a NUL-terminated UTF-8 constant the artifact owns, always
 * holding at least one dot. The returned reference is owned by the module
 * global that binds it, on the same rule as `pycc_ext_obj_import`: a
 * re-import or rebind releases it, and the last value is never released.
 */
PyObject *pycc_ext_obj_import_dotted(const char *name, long long bind_root)
{
    PyObject *import = NULL;
    PyObject *current = NULL;
    PyObject *segment = NULL;
    const char *rest;
    int found;

    found = PyDict_GetItemStringRef(PyEval_GetBuiltins(), "__import__", &import);
    if (found < 0) {
        return NULL;
    }
    if (found == 0) {
        PyErr_SetString(PyExc_ImportError, "__import__ not found");
        return NULL;
    }
    current = PyObject_CallFunction(import, "sOOOi", name, Py_None, Py_None, Py_None, 0);
    Py_DECREF(import);
    if (current == NULL || bind_root) {
        return current;
    }
    rest = strchr(name, '.');
    while (rest != NULL) {
        const char *start = rest + 1;
        PyObject *next;

        rest = strchr(start, '.');
        segment = (rest != NULL)
                      ? PyUnicode_FromStringAndSize(start, (Py_ssize_t)(rest - start))
                      : PyUnicode_FromString(start);
        if (segment == NULL) {
            Py_DECREF(current);
            return NULL;
        }
        next = pycc_ext_import_from_step(current, segment);
        Py_DECREF(segment);
        Py_DECREF(current);
        if (next == NULL) {
            return NULL;
        }
        current = next;
    }
    return current;
}

/*
 * #1293 (Part 3 of #1282): the failure edge of a foreign import nested in a
 * module-level `if`/`try` block, called only after `pycc_ext_obj_import`
 * returned NULL with CPython's exception set.
 *
 * An `ImportError` (a `ModuleNotFoundError` or a subclass of it becomes
 * pycc's tag 27, any other `ImportError` tag 26) is translated into a
 * pending pycc exception whose message is CPython's own `str(exc)` and which
 * carries the original (see `pycc_ext_bridge_store`).
 * Returns 1 with CPython's error indicator clear, and the generated code
 * branches to the enclosing handler exactly as an explicit `raise` does.
 *
 * Anything else -- a module body's own `ValueError`, a `SyntaxError`, a
 * `BaseException` -- is left exactly as CPython set it, and so is an
 * `ImportError` this function cannot translate (`str(exc)` fails): it returns 0, and the generated code takes the module
 * body's foreign failure edge (Part 1 of #1096): `pycc_ext_obj_error_bridge`
 * inside a module-level `try`, and the direct `-1` edge out of
 * `Py_mod_exec` outside every one. Every fallible step
 * runs before any pycc state is touched, so a 0 never leaves a pycc
 * exception pending, and a 1 never leaves a CPython one set.
 *
 * Not `static`: LLVM-generated code declares and calls it by this name
 * (`EXT_IMPORT_ERROR_BRIDGE_SYMBOL` in `crates/pycc_codegen/src/ext.rs`).
 */
int pycc_ext_import_error_bridge(void)
{
    PyObject *exc = PyErr_GetRaisedException();
    unsigned char tag;
    const char *class_name;
    size_t class_name_len;

    if (exc == NULL) {
        return 0;
    }
    if (!PyErr_GivenExceptionMatches(exc, PyExc_ImportError)) {
        PyErr_SetRaisedException(exc);
        return 0;
    }
    if (PyErr_GivenExceptionMatches(exc, PyExc_ModuleNotFoundError)) {
        tag = PYCC_EXT_TAG_MODULE_NOT_FOUND_ERROR;
        class_name = PYCC_EXT_NAME_MODULE_NOT_FOUND_ERROR;
        class_name_len = sizeof(PYCC_EXT_NAME_MODULE_NOT_FOUND_ERROR) - 1;
    } else {
        tag = PYCC_EXT_TAG_IMPORT_ERROR;
        class_name = PYCC_EXT_NAME_IMPORT_ERROR;
        class_name_len = sizeof(PYCC_EXT_NAME_IMPORT_ERROR) - 1;
    }
    if (!pycc_ext_bridge_store(exc, tag, class_name, class_name_len)) {
        PyErr_SetRaisedException(exc);
        return 0;
    }
    return 1;
}

/*
 * #1316: the foreign-operation bridge's class mapping. Each line maps one
 * CPython class to the pycc builtin tag and class-name literal a bridged
 * instance of it carries, and the first match wins, so a subclass is
 * listed before its base: `ModuleNotFoundError` before `ImportError`, the
 * four `ConnectionError` children before `ConnectionError`, every PEP 3151
 * subclass before `OSError`. A CPython class pycc does not model maps to
 * its nearest modelled base, and anything else under `Exception` to tag 0.
 * That is sound for compiled code -- `except NameError` is compile-time
 * `T0021`, so no pycc handler can tell -- and
 * the host always sees the original, which the pycc exception carries.
 *
 * `ext_build_tests`' `the_c_shims_foreign_error_mapping_names_its_classes`
 * parses these lines against `pycc_hir::BUILTIN_EXCEPTION_CLASSES` and
 * `builtin_exception_parent`, so a renumbered tag, a drifted name, or a
 * base listed ahead of its subclass fails there.
 */
#define PYCC_EXT_OBJ_TAG(py_class, tag_value, literal)                                           \
    if (PyErr_GivenExceptionMatches(exc, py_class)) {                                            \
        *class_name = literal;                                                                   \
        *class_name_len = sizeof(literal) - 1;                                                   \
        return tag_value;                                                                        \
    }

static unsigned char pycc_ext_obj_error_tag(PyObject *exc, const char **class_name,
                                            size_t *class_name_len)
{
    PYCC_EXT_OBJ_TAG(PyExc_ModuleNotFoundError, 27, "ModuleNotFoundError")
    PYCC_EXT_OBJ_TAG(PyExc_ImportError, 26, "ImportError")
    PYCC_EXT_OBJ_TAG(PyExc_OverflowError, 25, "OverflowError")
    PYCC_EXT_OBJ_TAG(PyExc_AssertionError, 28, "AssertionError")
    PYCC_EXT_OBJ_TAG(PyExc_AttributeError, 29, "AttributeError")
    PYCC_EXT_OBJ_TAG(PyExc_EOFError, 30, "EOFError")
    PYCC_EXT_OBJ_TAG(PyExc_NameError, 31, "NameError")
    PYCC_EXT_OBJ_TAG(PyExc_MemoryError, 32, "MemoryError")
    PYCC_EXT_OBJ_TAG(PyExc_StopIteration, 33, "StopIteration")
    PYCC_EXT_OBJ_TAG(PyExc_GeneratorExit, 34, "GeneratorExit")
    PYCC_EXT_OBJ_TAG(PyExc_BrokenPipeError, 19, "BrokenPipeError")
    PYCC_EXT_OBJ_TAG(PyExc_ConnectionAbortedError, 20, "ConnectionAbortedError")
    PYCC_EXT_OBJ_TAG(PyExc_ConnectionRefusedError, 21, "ConnectionRefusedError")
    PYCC_EXT_OBJ_TAG(PyExc_ConnectionResetError, 22, "ConnectionResetError")
    PYCC_EXT_OBJ_TAG(PyExc_BlockingIOError, 8, "BlockingIOError")
    PYCC_EXT_OBJ_TAG(PyExc_ChildProcessError, 9, "ChildProcessError")
    PYCC_EXT_OBJ_TAG(PyExc_ConnectionError, 10, "ConnectionError")
    PYCC_EXT_OBJ_TAG(PyExc_FileExistsError, 11, "FileExistsError")
    PYCC_EXT_OBJ_TAG(PyExc_FileNotFoundError, 12, "FileNotFoundError")
    PYCC_EXT_OBJ_TAG(PyExc_InterruptedError, 13, "InterruptedError")
    PYCC_EXT_OBJ_TAG(PyExc_IsADirectoryError, 14, "IsADirectoryError")
    PYCC_EXT_OBJ_TAG(PyExc_NotADirectoryError, 15, "NotADirectoryError")
    PYCC_EXT_OBJ_TAG(PyExc_PermissionError, 16, "PermissionError")
    PYCC_EXT_OBJ_TAG(PyExc_ProcessLookupError, 17, "ProcessLookupError")
    PYCC_EXT_OBJ_TAG(PyExc_TimeoutError, 18, "TimeoutError")
    PYCC_EXT_OBJ_TAG(PyExc_OSError, 7, "OSError")
    PYCC_EXT_OBJ_TAG(PyExc_ZeroDivisionError, 5, "ZeroDivisionError")
    PYCC_EXT_OBJ_TAG(PyExc_KeyError, 3, "KeyError")
    PYCC_EXT_OBJ_TAG(PyExc_IndexError, 4, "IndexError")
    PYCC_EXT_OBJ_TAG(PyExc_ValueError, 1, "ValueError")
    PYCC_EXT_OBJ_TAG(PyExc_TypeError, 2, "TypeError")
    PYCC_EXT_OBJ_TAG(PyExc_RuntimeError, 6, "RuntimeError")
    PYCC_EXT_OBJ_TAG(PyExc_Exception, 0, "Exception")
    /* Not an `Exception` at all: `SystemExit`, `KeyboardInterrupt`,
     * `GeneratorExit`, or a `BaseExceptionGroup` that is not an
     * `ExceptionGroup`. `except Exception` must not catch it. */
    *class_name = "BaseException";
    *class_name_len = sizeof("BaseException") - 1;
    return PYCC_EXT_TAG_FOREIGN_BASE;
}

#undef PYCC_EXT_OBJ_TAG

/*
 * #1316: the foreign-operation bridge. Called by compiled code in any
 * function body, and in the module body inside a module-level `try`
 * (Part 1 of #1096), when a foreign operation
 * (`pycc_ext_obj_getattr`, `_call`, `_len`, ...) failed with CPython's
 * exception set. It translates that exception into a pending pycc one that
 * carries the original, so an enclosing `try` in the
 * compiled function runs exactly as CPython's would, and an exception that
 * escapes unchanged reaches the host as the original object.
 *
 * Total, unlike `pycc_ext_import_error_bridge`: on return a pycc exception
 * is always pending and CPython's error indicator is always clear, because
 * the generated code branches straight to the innermost handler and has no
 * other edge. Its degraded paths:
 *
 *  - No exception set. The shim that failed broke CPython's contract;
 *    bridge a `SystemError` saying so rather than raising nothing.
 *  - `str(exc)` fails. Raise a pycc exception of
 *    the mapped class whose message is that class's name, and drop the
 *    original. The mapped tag, not tag 0, is kept deliberately, so a
 *    degraded `SystemExit` still escapes `except Exception`.
 *
 * Returns 1 always; the `int` return mirrors the import bridge's shape.
 * Not `static`: LLVM-generated code declares and calls it by this name
 * (`EXT_OBJ_ERROR_BRIDGE_SYMBOL` in `crates/pycc_codegen/src/ext.rs`).
 */
int pycc_ext_obj_error_bridge(void)
{
    PyObject *exc = PyErr_GetRaisedException();
    const char *class_name;
    size_t class_name_len;
    unsigned char tag;
    void *message;

    if (exc == NULL) {
        PyErr_SetString(PyExc_SystemError,
                        "pycc: foreign operation failed without setting an exception");
        exc = PyErr_GetRaisedException();
    }
    tag = pycc_ext_obj_error_tag(exc, &class_name, &class_name_len);
    if (pycc_ext_bridge_store(exc, tag, class_name, class_name_len)) {
        return 1;
    }
    /* Defensive: reached only when an allocation or `str(exc)` fails. */
    message = pycc_rt_str_from_literal((const unsigned char *)class_name,
                                       (long long)class_name_len);
    pycc_rt_exception_raise(pycc_rt_exception_alloc(
        tag, (const unsigned char *)class_name, class_name_len, message));
    Py_DECREF(exc);
    PyErr_Clear();
    return 1;
}

/*
 * Native standard I/O through CPython's own `sys` streams. Once an
 * interpreter is present -- this module imported by a host, or a native
 * build embedding CPython -- `sys.stdin` and `sys.stdout` buffer the same
 * file descriptors pycc_rt would otherwise read and write directly, and two
 * buffers over one descriptor reorder output and steal each other's input
 * lines. `pycc_ext_exec_module` therefore hands these three functions to
 * `pycc_rt_io_install` before the module body runs, and every native
 * `print`, `input()`, `sys.stdin.readline()`, ... then uses CPython's
 * streams (including a stream the program rebinds). A raising stream call
 * is bridged into pycc's pending exception like any foreign operation.
 */
extern void pycc_rt_object_install(void (*retain)(void *), void (*release)(void *));
extern void pycc_rt_io_install(int (*write)(const unsigned char *, size_t),
                               void *(*read)(int), int (*flush)(void));

static int pycc_ext_io_write(const unsigned char *ptr, size_t len)
{
    PyObject *stream = PySys_GetObject("stdout"); /* borrowed */
    PyObject *text;
    PyObject *result;

    /* `print` to a `None` `sys.stdout` writes nothing, as in CPython. */
    if (stream == NULL || stream == Py_None) {
        return 0;
    }
    text = PyUnicode_DecodeUTF8((const char *)ptr, (Py_ssize_t)len, "surrogateescape");
    if (text == NULL) {
        return pycc_ext_obj_error_bridge();
    }
    result = PyObject_CallMethod(stream, "write", "O", text);
    Py_DECREF(text);
    if (result == NULL) {
        return pycc_ext_obj_error_bridge();
    }
    Py_DECREF(result);
    return 0;
}

static void *pycc_ext_io_read(int all)
{
    PyObject *stream = PySys_GetObject("stdin"); /* borrowed */
    PyObject *result;
    const char *utf8;
    Py_ssize_t len;
    void *text;

    if (stream == NULL || stream == Py_None) {
        PyErr_SetString(PyExc_RuntimeError, "lost sys.stdin");
        (void)pycc_ext_obj_error_bridge();
        return NULL;
    }
    result = PyObject_CallMethod(stream, all ? "read" : "readline", NULL);
    if (result == NULL) {
        (void)pycc_ext_obj_error_bridge();
        return NULL;
    }
    if (PyBytes_Check(result)) {
        /* A rebound binary stream: decode its bytes as pycc_rt does. */
        PyObject *decoded = PyUnicode_DecodeUTF8(PyBytes_AsString(result),
                                                 PyBytes_Size(result), "replace");
        Py_DECREF(result);
        if (decoded == NULL) {
            (void)pycc_ext_obj_error_bridge();
            return NULL;
        }
        result = decoded;
    }
    utf8 = PyUnicode_AsUTF8AndSize(result, &len);
    if (utf8 == NULL) {
        Py_DECREF(result);
        (void)pycc_ext_obj_error_bridge();
        return NULL;
    }
    text = pycc_rt_str_from_literal((const unsigned char *)utf8, (long long)len);
    Py_DECREF(result);
    return text;
}

static int pycc_ext_io_flush(void)
{
    PyObject *stream = PySys_GetObject("stdout"); /* borrowed */
    PyObject *result;

    if (stream == NULL || stream == Py_None) {
        return 0;
    }
    result = PyObject_CallMethod(stream, "flush", NULL);
    if (result == NULL) {
        return pycc_ext_obj_error_bridge();
    }
    Py_DECREF(result);
    return 0;
}

/*
 * #1316: a compiled function read a module-level foreign name before the
 * module body bound it -- `f()` called above `import copy`, which D-041's
 * checker cannot rule out. CPython raises `NameError` there, so this does
 * too, and bridges it like any other failed foreign operation. The caller
 * then branches to its innermost handler.
 *
 * `name` is the binding's local name (`c` for `from copy import copy as
 * c`), `len` bytes of UTF-8, not NUL-terminated. Not `static`: LLVM-
 * generated code calls it (`EXT_NAME_ERROR_SYMBOL`).
 */
void pycc_ext_name_error(const unsigned char *name, long long len)
{
    PyErr_Format(PyExc_NameError, "name '%.*s' is not defined", (int)len, (const char *)name);
    (void)pycc_ext_obj_error_bridge();
}

