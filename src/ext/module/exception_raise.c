/*
 * Raising a pending pycc exception as a CPython one: what an `--ext`
 * export returns through, and what binding a caught pycc exception as a
 * CPython object (`object_except.c`) goes through once. Part of the one shim
 * translation unit (`src/ext_build.rs`'s `SHIM_C`), right after `bridge.c`,
 * whose exception tables (`pycc_ext_builtin_exception_class`, the user
 * exception classes) it reads.
 */

/*
 * `KeyError(ast.literal_eval(text))` as a new exception object, or NULL (no
 * Python error set) when `text` is not a literal: the caller then keeps the
 * text itself as the argument.
 */
static PyObject *pycc_ext_key_error_from_repr(PyObject *text)
{
    PyObject *ast = PyImport_ImportModule("ast");
    PyObject *literal_eval = (ast == NULL) ? NULL : PyObject_GetAttrString(ast, "literal_eval");
    PyObject *key = (literal_eval == NULL) ? NULL : PyObject_CallFunctionObjArgs(literal_eval, text, NULL);
    PyObject *raised = (key == NULL) ? NULL : PyObject_CallFunctionObjArgs(PyExc_KeyError, key, NULL);
    Py_XDECREF(key);
    Py_XDECREF(literal_eval);
    Py_XDECREF(ast);
    if (raised == NULL) {
        PyErr_Clear();
    }
    return raised;
}

/* `frames.c`: a pycc exception's compiled-frame traceback entries. */
static void pycc_ext_tb_sync(void *native, PyObject *obj);

/* `pycc_ext_tb_sync` on the CPython exception raised now. */
static void pycc_ext_tb_sync_raised(void *native)
{
    PyObject *raised = PyErr_GetRaisedException();

    pycc_ext_tb_sync(native, raised);
    PyErr_SetRaisedException(raised);
}

/*
 * CPython's implicit chaining (`_PyErr_SetObject`) for `value`, raised now
 * by a path that does not chain by itself (`PyErr_SetRaisedException`, a
 * compiled `raise` of an exception that already is a CPython object): the
 * exception being handled becomes its `__context__`, unless it is `value`
 * itself. If `value` is already in the handled exception's context chain,
 * the link that would close a cycle is cut, and a cycle already there ends
 * the walk (Floyd's test), both as CPython does.
 */
static void pycc_ext_chain_context(PyObject *value)
{
    PyObject *handled = PyErr_GetHandledException();
    PyObject *o;
    PyObject *slow;
    PyObject *context;
    int advance_slow = 0;

    if (handled == NULL || handled == value) {
        Py_XDECREF(handled);
        return;
    }
    o = handled;
    slow = handled;
    while ((context = PyException_GetContext(o)) != NULL) {
        /* Borrowed from here on: `o` keeps it alive. */
        Py_DECREF(context);
        if (context == value) {
            PyException_SetContext(o, NULL);
            break;
        }
        o = context;
        if (o == slow) {
            break;
        }
        if (advance_slow) {
            PyObject *next = PyException_GetContext(slow);

            Py_XDECREF(next);
            slow = next;
        }
        advance_slow = !advance_slow;
    }
    /* Steals the reference `PyErr_GetHandledException` returned. */
    PyException_SetContext(value, handled);
}

/* `object_except.c`: the CPython exception a pycc exception stands for. */
static PyObject *pycc_ext_exception_cpython(void *exc);
extern void *pycc_rt_exception_cause(void *obj);
extern int pycc_rt_exception_suppresses_context(void *obj);
extern void pycc_rt_exception_incref(void *obj);
extern void pycc_rt_exception_decref(void *obj);
/* `frames.c`: whether compiled code has raised a pycc exception. */
static int pycc_ext_exception_was_raised(void *exc);

/*
 * `__cause__` and `__suppress_context__` have one authority at a time. Until
 * a pycc exception has a CPython object, compiled code records them on the
 * exception itself (`raise X from Y`, `from None`), and they are given to
 * the object once, when it is made (`pycc_ext_raised_links`). From then on
 * the object holds them: a compiled `raise e from ...` writes to it
 * (`pycc_ext_raise_with_cause`, `pycc_ext_raise_from_none`), and so may
 * Python code, whose edits stay.
 *
 * A cause chain deeper than `PYCC_EXT_CAUSE_DEPTH`, which only a cycle of
 * causes not yet made reaches, is cut there.
 */
#define PYCC_EXT_CAUSE_DEPTH 64

/*
 * The CPython object for the pycc exception `cause` given as a `__cause__`
 * (borrowed from it), or NULL, with no CPython error set, when it cannot be
 * made. A cause compiled code never raised (`raise X from E(...)`) has no
 * `__context__` in CPython. It is made here while the handled exception is
 * published, which `PyErr_SetObject` would chain onto it, so that link is
 * dropped. One raised and bound by an `except ... as` was made where the
 * handler published it, with its context; one raised where nothing
 * publishes (a `finally`, a generator's handler: DEVIATIONS.md) keeps the
 * link. CPython's error indicator is kept.
 */
static PyObject *pycc_ext_cause_object(void *cause)
{
    static int depth = 0;
    PyObject *saved;
    PyObject *object;
    int fresh;

    if (depth >= PYCC_EXT_CAUSE_DEPTH) {
        return NULL;
    }
    fresh = pycc_rt_exception_foreign(cause) == NULL && !pycc_ext_exception_was_raised(cause);
    saved = PyErr_GetRaisedException();
    depth++;
    object = pycc_ext_exception_cpython(cause);
    depth--;
    if (object == NULL) {
        PyErr_Clear();
    } else if (fresh) {
        PyException_SetContext(object, NULL);
    }
    PyErr_SetRaisedException(saved);
    return object;
}

/*
 * The CPython exception just made for the pending pycc exception is raised:
 * gives it the links compiled code recorded on that one, then clears the
 * pending state.
 */
static void pycc_ext_raised_links(void)
{
    void *native = pycc_rt_exception_value();
    void *cause = pycc_rt_exception_cause(native);
    PyObject *raised;

    if (cause == NULL && !pycc_rt_exception_suppresses_context(native)) {
        pycc_rt_exception_clear();
        return;
    }
    /* The pending state's reference may be the last: kept past the clear. */
    pycc_rt_exception_incref(native);
    pycc_rt_exception_clear();
    raised = PyErr_GetRaisedException();
    if (raised != NULL) {
        PyObject *cause_object = cause == native ? raised
                                 : cause == NULL ? NULL
                                                 : pycc_ext_cause_object(cause);

        /* Steals the cause's reference; a NULL cause still suppresses. */
        PyException_SetCause(raised, cause_object == NULL ? NULL : Py_NewRef(cause_object));
    }
    pycc_rt_exception_decref(native);
    PyErr_SetRaisedException(raised);
}

extern void pycc_rt_exception_raise_with_cause(void *obj, void *cause);
extern void pycc_rt_exception_suppress_context(void *obj);

/*
 * A compiled `raise exc from cause`, both references handed over: records
 * the cause on `exc` while it has no CPython object, else sets it on the
 * object, and raises `exc`.
 */
void pycc_ext_raise_with_cause(void *exc, void *cause)
{
    PyObject *object = exc == NULL ? NULL : pycc_rt_exception_foreign(exc);
    PyObject *cause_object;

    if (object == NULL) {
        pycc_rt_exception_raise_with_cause(exc, cause);
        return;
    }
    cause_object = cause == exc ? object : cause == NULL ? NULL : pycc_ext_cause_object(cause);
    PyException_SetCause(object, cause_object == NULL ? NULL : Py_NewRef(cause_object));
    pycc_rt_exception_decref(cause);
    pycc_rt_exception_raise(exc);
}

/*
 * A compiled `raise exc from None`, before the raise: marks `exc` while it
 * has no CPython object, else clears the object's cause, which sets its
 * `__suppress_context__`.
 */
void pycc_ext_raise_from_none(void *exc)
{
    PyObject *object = exc == NULL ? NULL : pycc_rt_exception_foreign(exc);

    if (object == NULL) {
        pycc_rt_exception_suppress_context(exc);
        return;
    }
    PyException_SetCause(object, NULL);
}

/*
 * Translates a pending pycc exception into a CPython one and clears it.
 * Returns 1 when it raised, 0 when nothing was pending.
 *
 * This is what stops an `ext` artifact killing its host. `native` mode's
 * handler, `pycc_rt_exception_print_and_exit`, calls `exit(1)`; inside an
 * imported extension module that terminates the interpreter rather than
 * failing the call. Both `ext` callers use this instead, with the two
 * different conventions CPython imposes on them: a per-export wrapper then
 * returns NULL, and the `Py_mod_exec` slot returns -1.
 *
 * The tag values are fixed by `pycc_hir::exception`'s
 * `BUILTIN_EXCEPTION_CLASSES` array order, which this file cannot see from
 * C: tags 0..=6 are the flat seven, which `pycc_rt::exception` also names as
 * `EXCEPTION_TYPE_*` constants, and tags 7..=22 are the PEP 3151 `OSError`
 * family; 25..=29 are `OverflowError`, `ImportError`,
 * `ModuleNotFoundError`, `AssertionError` and `AttributeError`, and 23..=24 (the PEP 654
 * groups) fall to `default:`. Two tests are this switch's drift guard -- `ext_bridge`'s
 * `exception_type_tags_match_the_c_shims_hardcoded_switch` for the seven
 * constants, and `ext_build_tests`'
 * `every_exception_tag_the_c_shim_switches_on_still_names_that_class` for
 * the full array.
 */
static int pycc_ext_raise_pending(void)
{
    int tag = pycc_rt_ext_pending_type();
    PyObject *exc_type;
    PyObject *user_class;
    PyObject *message;
    PyObject *original;
    size_t len = 0;
    const unsigned char *bytes;

    if (tag < 0) {
        return 0;
    }
    /*
     * #1293: a pending exception a bridge produced, escaping unchanged,
     * re-raises CPython's own original object rather than a rebuilt one.
     * Anything pycc raised itself, including an `except*` remainder's
     * freshly allocated group, carries none and takes the switch.
     */
    original = pycc_rt_exception_foreign(pycc_rt_exception_value());
    if (original != NULL) {
        pycc_ext_tb_sync(pycc_rt_exception_value(), original);
        Py_INCREF(original);
        PyErr_SetRaisedException(original);
        /* Its links are the object's own (`pycc_ext_raised_links`). */
        pycc_rt_exception_clear();
        return 1;
    }
    exc_type = pycc_ext_builtin_exception_class(tag);
    if (tag == 31 && pycc_rt_ext_pending_is_unbound_local()) {
        /* A function local read before its assignment ran raises CPython's
         * `UnboundLocalError`, which subclasses `NameError`. */
        exc_type = PyExc_UnboundLocalError;
    }
    if (exc_type == NULL) {
        /*
         * Tag 0 is `Exception`. So, deliberately, are the two remaining
         * builtin tags -- a hole in the otherwise contiguous switched range,
         * since tags 25..=29 above sit past them -- and every user-defined class:
         *
         *  - tags 23..=24 are `BaseExceptionGroup`/`ExceptionGroup`. The C
         *    API exposes no `PyExc_ExceptionGroup` at all, and the type it
         *    does expose cannot be constructed from a lone message -- PEP
         *    654 requires `(msg, exceptions)`, so `PyErr_SetObject` would
         *    fail during normalization and surface a `TypeError` about the
         *    constructor instead of the program's own error. `Exception`
         *    with the right message is the more truthful of the two.
         *  - a user-defined exception class carries a module-assigned tag
         *    this shim knows nothing about, so the lookup below -- not any
         *    tag arithmetic here -- decides whether the artifact registered
         *    a class for it. A hit replaces `Exception` with the real
         *    class, restoring `except m.MyError:`, `except ValueError:` for
         *    a builtin subclass, and `type(e).__name__`. A miss keeps
         *    `Exception`, which is what leaves the two group tags above (and
         *    any class derived from them, which the table excludes for the
         *    same reason) on the honest fallback.
         */
        exc_type = PyExc_Exception;
        user_class = pycc_ext_user_exception_class((unsigned char)tag);
        if (user_class != NULL) {
            exc_type = user_class;
        }
    }
    bytes = pycc_rt_ext_pending_message(&len);
    /* Not NUL-terminated: a `PyStrObj` carries an explicit length. */
    message = (bytes == NULL) ? PyUnicode_FromString("")
                              : PyUnicode_FromStringAndSize((const char *)bytes, (Py_ssize_t)len);
    if (message == NULL) {
        /* A MemoryError is already set; keep it and drop the pycc-side one. */
        pycc_rt_exception_clear();
        return 1;
    }
    if (tag == 3 && pycc_rt_ext_pending_key_is_repr()) {
        /* A `KeyError` for a key that is not a `str` carries the key's
         * `repr` (an `int`, `float`, `bool` or a tuple of those): rebuild the
         * key so CPython's exception holds it (`KeyError: 0`, `e.args ==
         * (0,)`), not its text (`KeyError: '0'`). */
        PyObject *raised = pycc_ext_key_error_from_repr(message);
        if (raised != NULL) {
            Py_DECREF(message);
            /* `PyErr_SetObject` would have chained it. */
            pycc_ext_chain_context(raised);
            PyErr_SetRaisedException(raised);
            pycc_ext_tb_sync_raised(pycc_rt_exception_value());
            pycc_ext_raised_links();
            return 1;
        }
    }
    PyErr_SetObject(exc_type, message);
    Py_DECREF(message);
    pycc_ext_tb_sync_raised(pycc_rt_exception_value());
    pycc_ext_raised_links();
    return 1;
}
