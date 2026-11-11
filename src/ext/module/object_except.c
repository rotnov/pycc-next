/*
 * Non-strict mode: an `except` clause whose type is a CPython object
 * (`except struct.error:`, `except mod.Error as e:`). `pycc_hir` lowers
 * such a `try` to a clause catching `Exception` whose body tests the caught
 * pycc exception against the object (`pycc_ext_obj_exc_match`) and binds
 * the CPython exception it is (`pycc_ext_obj_exc_object`), re-raising it
 * when no clause matches. Part of the one shim translation unit
 * (`src/ext_build.rs`'s `SHIM_C`), after `bridge.c`, whose
 * `pycc_ext_raise_pending` builds the CPython exception.
 */

extern void pycc_rt_exception_incref(void *obj);

/*
 * The CPython exception the pycc exception `exc` (borrowed) stands for: the
 * original it was bridged from, else the exception the export boundary would
 * raise for it (`pycc_ext_raise_pending`), which becomes its original from
 * then on, so it is built once and an escaping `exc` re-raises that very
 * object, given the traceback entries compiled code added since. Borrowed
 * from `exc`; NULL with a CPython exception set on failure.
 * Must be called with no pycc exception pending (inside a handler).
 */
static PyObject *pycc_ext_exception_cpython(void *exc)
{
    PyObject *original = pycc_rt_exception_foreign(exc);
    PyObject *saved;
    PyObject *built;

    if (original != NULL) {
        pycc_ext_tb_sync(exc, original);
        return original;
    }
    saved = PyErr_GetRaisedException();
    /* Pending with a reference of its own, which `pycc_ext_raise_pending`
     * releases when it clears the pending state. */
    pycc_rt_exception_incref(exc);
    pycc_rt_exception_raise(exc);
    pycc_ext_raise_pending();
    built = PyErr_GetRaisedException();
    PyErr_SetRaisedException(saved);
    if (built == NULL) {
        PyErr_SetString(PyExc_SystemError, "pycc: an exception could not be translated");
        return NULL;
    }
    pycc_rt_exception_set_foreign(exc, built);
    return built;
}

/* Whether `cls` may be caught: an exception class, or a tuple of them
 * (CPython's `except` check). 1, or 0 with `TypeError` set. */
static int pycc_ext_catchable(PyObject *cls)
{
    Py_ssize_t i;

    if (PyTuple_Check(cls)) {
        for (i = 0; i < PyTuple_Size(cls); i++) {
            PyObject *item = PyTuple_GetItem(cls, i);
            if (!(PyType_Check(item) && PyType_IsSubtype((PyTypeObject *)item,
                                                         (PyTypeObject *)PyExc_BaseException))) {
                goto refuse;
            }
        }
        return 1;
    }
    if (PyType_Check(cls) && PyType_IsSubtype((PyTypeObject *)cls,
                                              (PyTypeObject *)PyExc_BaseException)) {
        return 1;
    }
refuse:
    PyErr_SetString(PyExc_TypeError,
                    "catching classes that do not inherit from BaseException is not allowed");
    return 0;
}

/* Whether the caught pycc exception `exc` matches the `except` clause type
 * `cls` (both borrowed). A failure is bridged into pycc's pending exception
 * (the `except` clause's own `TypeError` replaces the caught one, as in
 * CPython). */
signed char pycc_ext_obj_exc_match(void *exc, PyObject *cls)
{
    PyObject *e;

    if (!pycc_ext_catchable(cls)) {
        pycc_ext_obj_error_bridge();
        return 0;
    }
    e = pycc_ext_exception_cpython(exc);
    if (e == NULL) {
        pycc_ext_obj_error_bridge();
        return 0;
    }
    return (signed char)PyErr_GivenExceptionMatches(e, cls);
}

/* The caught pycc exception `exc` (borrowed) as the CPython exception it
 * is: a new reference, what an `except T as e:` clause binds. */
PyObject *pycc_ext_obj_exc_object(void *exc)
{
    PyObject *e = pycc_ext_exception_cpython(exc);

    if (e == NULL) {
        pycc_ext_obj_error_bridge();
        Py_RETURN_NONE;
    }
    Py_INCREF(e);
    return e;
}

/*
 * A compiled `except` handler publishes the exception it handles, as a
 * CPython handler does: `sys.exception()`, `sys.exc_info()` and
 * `traceback.format_exc()` see it while the handler runs, and an exception
 * raised there gets it as its `__context__`.
 *
 * It does so the first time something could see it. A handler's entry stays
 * what it was (the pending pycc exception taken into the handler's record),
 * and the generated code calls `pycc_ext_handler_publish` before every
 * operation of the handler's body that can run other code or raise: a call,
 * a release that frees its object (`pycc_ext_handler_release`), a lookup
 * that is not provably silent (`pycc_ext_obj_getitem_lazy_in`). A
 * handler that only returns a default, or whose dict miss finds its key in a
 * second dict, never makes a CPython exception or a traceback.
 *
 * `PyccExtHandler` is the handler's record, an entry-block array of the
 * compiled function (`crates/pycc_codegen/src/exception/handled.rs` mirrors
 * the layout). `exc`, `key` and `prev` are released wherever the handler is
 * left, `prev` by `pycc_ext_handler_leave`.
 */
typedef struct PyccExtHandler {
    /* The caught pycc exception (owned). NULL in a handler a dict miss
     * entered straight away (`pycc_ext_obj_getitem_lazy`), until its
     * `KeyError` is made, and once the handler is left. */
    void *exc;
    /* Set once published: the exception that was being handled before
     * (owned; `None` for none). */
    PyObject *prev;
    /* The key of the dict miss that entered the handler (owned), until its
     * `KeyError`, which then holds it, is made. */
    PyObject *key;
    /* The handler of the same function this one's `try` is inside, or NULL:
     * published first, so each is the next one's previous exception. */
    struct PyccExtHandler *parent;
    /* The function's frame description and live frame (NULL without one),
     * and the position of the subscript that missed: the traceback entry of
     * the `KeyError`. */
    PyccFnInfo *info;
    PyccIFrame *frame;
    long long pos;
} PyccExtHandler;

extern void pycc_rt_tb_here(const PyccFnInfo *info, uint32_t pos, const void *activation);

/* Makes the `KeyError` of the dict miss that entered `h`, as the lookup
 * would have raised it: its only argument the key (`_PyErr_SetKeyError`),
 * its context the exception handled now, its traceback entry the missed
 * subscript's. Called with no pycc exception pending. */
static void pycc_ext_handler_make_key_error(PyccExtHandler *h)
{
    PyObject *key = h->key;
    PyObject *args = PyTuple_Pack(1, key);

    h->key = NULL;
    if (args != NULL) {
        PyErr_SetObject(PyExc_KeyError, args);
        Py_DECREF(args);
    }
    Py_DECREF(key);
    /* Total: a `MemoryError` of the tuple is what the handler handles. */
    (void)pycc_ext_obj_error_bridge();
    if (h->info != NULL && h->frame != NULL) {
        pycc_ext_tb_here_live(h->info, (uint32_t)h->pos, h->frame);
    } else if (h->info != NULL) {
        pycc_rt_tb_here(h->info, (uint32_t)h->pos, NULL);
    }
    h->exc = pycc_rt_exception_take();
}

void pycc_ext_handler_publish(PyccExtHandler *h)
{
    void *aside;
    PyObject *raised;
    PyObject *e;
    PyObject *prev;

    if (h->prev != NULL || (h->exc == NULL && h->key == NULL)) {
        /* Published already, or left: the calls on the way out of a
         * handler come after its record is released. */
        return;
    }
    if (h->parent != NULL) {
        pycc_ext_handler_publish(h->parent);
    }
    /* On an exception edge the exception leaving the handler is pending,
     * as pycc's or still as CPython's. */
    aside = pycc_rt_exception_take();
    raised = PyErr_GetRaisedException();
    if (h->exc == NULL) {
        pycc_ext_handler_make_key_error(h);
    }
    e = pycc_ext_exception_cpython(h->exc);
    if (e == NULL) {
        PyErr_Clear();
    } else {
        prev = PyErr_GetHandledException();
        PyErr_SetHandledException(e);
        h->prev = prev != NULL ? prev : Py_NewRef(Py_None);
    }
    PyErr_SetRaisedException(raised);
    pycc_rt_exception_restore(aside);
}

/* Leaving a published handler (`prev` owned, NULL when it never
 * published): the exception handled before is the handled one again. The
 * handler's own is released by this function, not inside CPython, so a
 * finalizer it runs sees no pending pycc exception
 * (`pycc_ext_obj_dealloc`). */
void pycc_ext_handler_leave(PyObject *prev)
{
    PyObject *handled;

    if (prev == NULL) {
        return;
    }
    handled = PyErr_GetHandledException();
    PyErr_SetHandledException(prev == Py_None ? NULL : prev);
    Py_XDECREF(handled);
    Py_DECREF(prev);
}

/*
 * A compiled `raise e`: CPython gives the exception it raises the one being
 * handled as its `__context__` at every raise. An `e` that already is a
 * CPython object (an `except ... as` bound it, or it came from CPython) is
 * chained here; one that is not gets its context where its object is made
 * (`pycc_ext_handler_unwind`, `pycc_ext_raise_pending`). A handler around
 * the `raise` has published its exception first.
 */
void pycc_ext_raise_context(void *exc)
{
    PyObject *original = exc == NULL ? NULL : pycc_rt_exception_foreign(exc);

    if (original != NULL) {
        pycc_ext_chain_context(original);
    }
}

/* An exception is leaving `h`'s body (it is pending). One compiled code
 * raised itself has no CPython object yet: it is made now, while `h`'s
 * exception is still the handled one, so that becomes its `__context__`
 * as it does where CPython raises. One that already had an object got its
 * context where it was raised (in CPython, or `pycc_ext_raise_context`);
 * the handler's own exception, raised again, gets none. */
void pycc_ext_handler_unwind(PyccExtHandler *h)
{
    void *raised = pycc_rt_exception_take();

    if (raised != NULL && raised != h->exc && pycc_rt_exception_foreign(raised) == NULL) {
        pycc_ext_handler_publish(h);
        if (pycc_ext_exception_cpython(raised) == NULL) {
            PyErr_Clear();
        }
    }
    pycc_rt_exception_restore(raised);
}

/*
 * Whether a lookup of an exact `str` in the exact dict `d` runs no code and
 * raises nothing: its keys are all `str`s, so no key's `__eq__` is called
 * (and an exact `str` hashes and compares without any).
 *
 * `PyDictObject.ma_keys` and `PyDictKeysObject.dk_kind` are not part of the
 * stable ABI. They are read at the offsets CPython 3.13 to 3.15 share for a
 * 64-bit GIL build (32, and 10: `DICT_KEYS_GENERIC` is 0, a table of `str`
 * keys 1 or 2), only after a probe has seen a dict of one `str` key report
 * a `str` table and the same dict with an `int` key added a general one;
 * on any other version, or a failed probe, no lookup counts as silent.
 */
static int pycc_ext_dict_kind_state = 0;

static unsigned char pycc_ext_dict_kind(PyObject *d)
{
    return (*(unsigned char **)((char *)d + 32))[10];
}

static void pycc_ext_dict_kind_probe(void)
{
    PyObject *d = NULL;
    PyObject *text = NULL;
    PyObject *number = NULL;
    int ok = 0;

    pycc_ext_dict_kind_state = -1;
    if (sizeof(void *) != 8 || (Py_Version >> 16) < 0x030D || (Py_Version >> 16) > 0x030F) {
        return;
    }
    d = PyDict_New();
    text = PyUnicode_FromString("pycc");
    number = PyLong_FromLong(1);
    if (d != NULL && text != NULL && number != NULL && pycc_ext_dict_kind(d) != 0
        && PyDict_SetItem(d, text, Py_None) == 0 && pycc_ext_dict_kind(d) == 1
        && PyDict_SetItem(d, number, Py_None) == 0 && pycc_ext_dict_kind(d) == 0) {
        ok = 1;
    }
    Py_XDECREF(d);
    Py_XDECREF(text);
    Py_XDECREF(number);
    PyErr_Clear();
    if (ok) {
        pycc_ext_dict_kind_state = 1;
    }
}

/* `pycc_ext_obj_getitem_lazy` inside the body of the handler `h`: the
 * handled exception is published first unless the lookup is one nothing
 * can see it from -- an exact `str` in an exact dict of `str` keys, with
 * the fast paths on (off, a miss raises). That lookup is made here: it
 * runs no code and raises nothing, and releasing an exact `str` key frees
 * at most a `str`. */
PyObject *pycc_ext_obj_getitem_lazy_in(PyObject *o, PyObject *k, PyObject **missed,
                                       PyccExtHandler *h)
{
    PyObject *result;

    if (pycc_ext_dict_kind_state == 0) {
        pycc_ext_dict_kind_probe();
    }
    if (o == NULL || k == NULL || pycc_ext_dict_kind_state != 1 || !PyDict_CheckExact(o)
        || !PyUnicode_CheckExact(k) || pycc_ext_dict_kind(o) == 0
        || !pycc_ext_fast_ops_enabled()) {
        pycc_ext_handler_publish(h);
        return pycc_ext_obj_getitem_lazy(o, k, missed);
    }
    if (PyDict_GetItemRef(o, k, &result) == 0) {
        pycc_ext_lazy_miss_count++;
        *missed = k;
        return PYCC_EXT_LAZY_MISS;
    }
    Py_DECREF(k);
    return result;
}

/* A release of `o` in `h`'s body (`pycc_ext_obj_release`, which the
 * generated code calls outside one): when it frees `o`, whose finalizer
 * may look at what is being handled, the handler publishes first. */
void pycc_ext_handler_release(PyccExtHandler *h, PyObject *o)
{
    if (o == NULL) {
        return;
    }
    if (h->prev == NULL && Py_REFCNT(o) == 1) {
        pycc_ext_handler_publish(h);
    }
    Py_DECREF(o);
}
