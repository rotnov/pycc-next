/*
 * A patched forwarder's calls (`forward.c`): its trampoline's, and a call
 * site's that calls the forwarder's compiled body through its slot
 * (`call_site.c`, `object_ops.c`). A call that passes the parameters as
 * they stand, in order -- or leaves the rest to their defaults -- calls the
 * compiled body directly; any other is bound by `pycc_fwd_bind`, or by
 * CPython when the trampoline does not bind it.
 */

/* Drops the forwarder's own traceback entry from the exception raised now,
 * when it is the newest one. */
static void pycc_fwd_drop_entry(PycFwdSlot *slot)
{
    PyObject *exc = PyErr_GetRaisedException();
    PyObject *tb, *frame, *code, *next;

    if (exc == NULL) {
        return;
    }
    if (slot->own_body) {
        PyErr_SetRaisedException(exc);
        return;
    }
    tb = PyException_GetTraceback(exc);
    if (tb != NULL) {
        frame = PyObject_GetAttrString(tb, "tb_frame");
        code = frame != NULL ? PyObject_GetAttrString(frame, "f_code") : NULL;
        if (code != NULL && code == slot->code) {
            next = PyObject_GetAttrString(tb, "tb_next");
            if (next != NULL) {
                (void)PyException_SetTraceback(exc, next);
                Py_DECREF(next);
            }
        }
        Py_XDECREF(code);
        Py_XDECREF(frame);
        Py_DECREF(tb);
    }
    PyErr_Clear();
    PyErr_SetRaisedException(exc);
}

/* Calls the compiled body with `n` arguments, its parameters in order. */
static PyObject *pycc_fwd_target(PycFwdSlot *slot, PyObject *const *args, Py_ssize_t n)
{
    PyObject *result;

    if (slot->entry == NULL && slot->entry_kw == NULL) {
        return PyObject_Vectorcall(slot->target, args, (size_t)n, NULL);
    }
    /* What calling the builtin would check. */
    if (Py_EnterRecursiveCall(" while calling a Python object")) {
        return NULL;
    }
    result = slot->entry != NULL ? slot->entry(slot->self, args, n)
                                 : slot->entry_kw(slot->self, args, n, NULL);
    Py_LeaveRecursiveCall();
    return result;
}

/* The call as the forwarder's own vectorcall makes it. */
static PyObject *pycc_fwd_slow(PycFwdSlot *slot, PyObject *func, PyObject *const *args,
                               size_t nargsf, PyObject *kwnames)
{
    PyObject *result = pycc_fwd_original(func, args, nargsf, kwnames);

    if (result == NULL) {
        pycc_fwd_drop_entry(slot);
    }
    return result;
}

/* The index of keyword `name` among the parameters a keyword can bind
 * (`co_varnames[posonly:argcount + kwonly]`), or -1. */
static int pycc_fwd_keyword(PycFwdSlot *slot, PyObject *name)
{
    int i, end = slot->argcount + slot->kwonly;

    for (i = slot->posonly; i < end; i++) {
        if (PyTuple_GetItem(slot->names, i) == name) {
            return i;
        }
    }
    for (i = slot->posonly; i < end; i++) {
        int equal = PyObject_RichCompareBool(PyTuple_GetItem(slot->names, i), name, Py_EQ);

        if (equal < 0) {
            PyErr_Clear();
            return -1;
        }
        if (equal) {
            return i;
        }
    }
    return -1;
}

/* Drops what `pycc_fwd_bind` held for a call: the keyword defaults among
 * the `n` `bound` that `owned` marks, and the defaults themselves. */
static void pycc_fwd_release(PyObject **bound, const char *owned, int n, PyObject *defaults,
                             PyObject *kwdefaults)
{
    int i;

    for (i = 0; i < n; i++) {
        if (owned[i]) {
            Py_DECREF(bound[i]);
        }
    }
    Py_XDECREF(kwdefaults);
    Py_XDECREF(defaults);
}

/*
 * A call the fast path does not take: binds the arguments to the
 * forwarder's parameters and calls the compiled body with them in its
 * order (the class first, when the forwarder passes it, then the
 * parameters in source order: positional, `*args`, keyword-only,
 * `**kwargs`). Kept out of line, so the fast path stays a few cache lines.
 */
static PyObject *pycc_fwd_bind(PycFwdSlot *slot, PyObject *func, PyObject *const *args,
                               size_t nargsf, PyObject *kwnames) __attribute__((noinline));
static PyObject *pycc_fwd_bind(PycFwdSlot *slot, PyObject *func, PyObject *const *args,
                               size_t nargsf, PyObject *kwnames)
{
    PyObject *bound[PYCC_FWD_MAX_PARAMS];
    PyObject *call[PYCC_FWD_MAX_PARAMS + 1];
    /* The keyword defaults `bound` takes, held until the body returns. */
    char owned[PYCC_FWD_MAX_PARAMS];
    PyObject *star = NULL, *starstar = NULL, *result, *defaults = NULL, *kwdefaults = NULL;
    Py_ssize_t nargs = PyVectorcall_NARGS(nargsf);
    Py_ssize_t nkw;
    int nparams = slot->argcount + slot->kwonly;
    int i, n = 0;

    if (PYCC_FUNC_FIELD(func, PYCC_FUNC_CODE) != slot->code) {
        return pycc_fwd_original(func, args, nargsf, kwnames);
    }
    nkw = kwnames != NULL ? PyTuple_Size(kwnames) : 0;
    if (slot->cell != NULL && slot->klass == NULL) {
        slot->klass = PyObject_GetAttrString(slot->cell, "cell_contents");
        if (slot->klass == NULL) {
            PyErr_Clear();
            return pycc_fwd_slow(slot, func, args, nargsf, kwnames);
        }
    }
    /* Exactly the positional parameters, after the class. */
    if (nkw == 0 && nargs == slot->argcount && slot->kwonly == 0 && !slot->varargs
        && !slot->varkw) {
        call[0] = slot->klass;
        for (i = 0; i < nargs; i++) {
            call[i + 1] = args[i];
        }
        return pycc_fwd_target(slot, call, nargs + 1);
    }
    if (nargs > slot->argcount && !slot->varargs) {
        return pycc_fwd_slow(slot, func, args, nargsf, kwnames);
    }
    for (i = 0; i < nparams; i++) {
        bound[i] = i < nargs && i < slot->argcount ? args[i] : NULL;
        owned[i] = 0;
    }
    for (i = 0; i < nkw; i++) {
        PyObject *name = PyTuple_GetItem(kwnames, i);
        int at = pycc_fwd_keyword(slot, name);

        if (at >= 0 && bound[at] == NULL) {
            bound[at] = args[nargs + i];
            continue;
        }
        if (at >= 0 || !slot->varkw) {
            Py_XDECREF(starstar);
            return pycc_fwd_slow(slot, func, args, nargsf, kwnames);
        }
        if (starstar == NULL && (starstar = PyDict_New()) == NULL) {
            return NULL;
        }
        if (PyDict_GetItemWithError(starstar, name) != NULL || PyErr_Occurred()
            || PyDict_SetItem(starstar, name, args[nargs + i]) != 0) {
            PyErr_Clear();
            Py_DECREF(starstar);
            return pycc_fwd_slow(slot, func, args, nargsf, kwnames);
        }
    }
    {
        PyObject *now = PYCC_FUNC_FIELD(func, PYCC_FUNC_DEFAULTS);

        if (now != slot->defaults) {
            /* `__defaults__` changed (or this is the first call): take the
             * new items. The old tuple is kept until then, so its address
             * cannot come back as another tuple's. */
            Py_ssize_t count = now != NULL ? PyTuple_Size(now) : 0;

            if (count > slot->argcount) {
                Py_XDECREF(starstar);
                return pycc_fwd_slow(slot, func, args, nargsf, kwnames);
            }
            {
                PyObject *old = slot->defaults;

                slot->defaults = Py_XNewRef(now);
                Py_XDECREF(old);
            }
            slot->ndefaults = count;
        }
        /* Held, not borrowed from the slot or the function, until the body
         * returns: a keyword default's key compares with `__eq__`, and
         * unpacking an argument can run code too, and either can rebind
         * `__defaults__` (a nested call then replaces `slot->defaults`) or
         * `__kwdefaults__`, which would free a default the call passes. */
        defaults = Py_XNewRef(slot->defaults);
        kwdefaults = Py_XNewRef(PYCC_FUNC_FIELD(func, PYCC_FUNC_KWDEFAULTS));
        for (i = 0; i < nparams; i++) {
            if (bound[i] != NULL) {
                continue;
            }
            if (i < slot->argcount && i >= slot->argcount - slot->ndefaults) {
                bound[i] = PyTuple_GetItem(defaults, i - (slot->argcount - slot->ndefaults));
            } else if (i >= slot->argcount && kwdefaults != NULL) {
                bound[i] = PyDict_GetItemWithError(kwdefaults, PyTuple_GetItem(slot->names, i));
                PyErr_Clear();
                if (bound[i] != NULL) {
                    Py_INCREF(bound[i]);
                    owned[i] = 1;
                }
            }
            if (bound[i] == NULL) {
                pycc_fwd_release(bound, owned, nparams, defaults, kwdefaults);
                Py_XDECREF(starstar);
                return pycc_fwd_slow(slot, func, args, nargsf, kwnames);
            }
        }
    }
    if (slot->varargs) {
        Py_ssize_t extra = nargs > slot->argcount ? nargs - slot->argcount : 0;

        star = PyTuple_New(extra);
        if (star == NULL) {
            pycc_fwd_release(bound, owned, nparams, defaults, kwdefaults);
            Py_XDECREF(starstar);
            return NULL;
        }
        for (i = 0; i < extra; i++) {
            Py_INCREF(args[slot->argcount + i]);
            PyTuple_SetItem(star, i, args[slot->argcount + i]);
        }
    }
    if (slot->varkw && starstar == NULL && (starstar = PyDict_New()) == NULL) {
        pycc_fwd_release(bound, owned, nparams, defaults, kwdefaults);
        Py_XDECREF(star);
        return NULL;
    }
    if (slot->cell != NULL) {
        call[n++] = slot->klass;
    }
    for (i = 0; i < slot->argcount; i++) {
        call[n++] = bound[i];
    }
    if (star != NULL) {
        call[n++] = star;
    }
    for (i = slot->argcount; i < nparams; i++) {
        call[n++] = bound[i];
    }
    if (starstar != NULL) {
        call[n++] = starstar;
    }
    result = pycc_fwd_target(slot, call, n);
    pycc_fwd_release(bound, owned, nparams, defaults, kwdefaults);
    Py_XDECREF(star);
    Py_XDECREF(starstar);
    return result;
}

/* How many calls `pycc_fwd_direct` made straight to a compiled body, which
 * the tests read with `ctypes` (`RUNTIME_COUNTERS` in
 * `src/ext_build/link.rs`). */
static long long pycc_fwd_direct_calls = 0;

long long pycc_ext_site_forward_calls(void)
{
    return pycc_fwd_direct_calls;
}

/* How many keyword names `kwnames` (or none) holds. */
static Py_ssize_t pycc_fwd_nkw(PyObject *kwnames)
{
    if (kwnames == NULL) {
        return 0;
    }
    return pycc_ext_seq_layout > 0 ? PYCC_SEQ_SIZE(kwnames) : PyTuple_Size(kwnames);
}

/* `slot`'s parameter name `i`, borrowed. */
static PyObject *pycc_fwd_name(const PycFwdSlot *slot, Py_ssize_t i)
{
    return slot->name_items != NULL ? slot->name_items[i] : PyTuple_GetItem(slot->names, i);
}

/* Whether the `nkw` keywords `kwnames` name, as the same interned strings,
 * the parameters after the `nargs` passed positionally, in order. */
static int pycc_fwd_keywords_follow(const PycFwdSlot *slot, Py_ssize_t nargs, PyObject *kwnames,
                                    Py_ssize_t nkw)
{
    Py_ssize_t i;

    for (i = 0; i < nkw; i++) {
        PyObject *given = pycc_ext_seq_layout > 0 ? PYCC_TUPLE_ITEMS(kwnames)[i]
                                                  : PyTuple_GetItem(kwnames, i);

        if (given != pycc_fwd_name(slot, nargs + i)) {
            return 0;
        }
    }
    return 1;
}

/* Whether a call of `slot`'s function passing `nargs` arguments
 * positionally, then the keyword arguments `kwnames` (or none), passes
 * exactly its parameters in their order: each keyword names, as the same
 * interned string, the parameter after the ones before it (`f(x, k=v)` for
 * `def f(x, k)`), as CPython's own calls name them. The arguments then are
 * the parameters as they stand, and the compiled body takes them as they
 * are: nothing to bind. A keyword that names its parameter by an equal but
 * other string, or out of order, is bound by `pycc_fwd_bind`. Only for a
 * `simple` slot, which its callers test first: the others never pass, and
 * do not pay for the call. */
static int pycc_fwd_in_order(const PycFwdSlot *slot, Py_ssize_t nargs, PyObject *kwnames)
{
    Py_ssize_t nkw = pycc_fwd_nkw(kwnames);

    if (nargs + nkw != slot->argcount || (nkw > 0 && nargs < slot->posonly)) {
        return 0;
    }
    return nkw == 0 || pycc_fwd_keywords_follow(slot, nargs, kwnames, nkw);
}

/*
 * A call of `slot`'s function `func` that passes its leading parameters in
 * order -- `nargs` positionally, then by keyword as `pycc_fwd_in_order`
 * names them -- and leaves every later one to its default
 * (`get(key)` for `def get(self, key, *, lists=True)`): the compiled body
 * called with the arguments and the defaults as the function holds them
 * now, its result in `*result`. Returns 0, calling nothing, for a call of
 * any other shape, a function with `*args`, `**kwargs` or `__class__`, or a
 * parameter left without a default; `pycc_fwd_bind` or CPython then binds
 * it. Out of line, so a call that passes every parameter does not pay for
 * the array.
 */
static int pycc_fwd_defaulted(PycFwdSlot *slot, PyObject *func, PyObject *const *args,
                              Py_ssize_t nargs, PyObject *kwnames, PyObject **result)
    __attribute__((noinline));
static int pycc_fwd_defaulted(PycFwdSlot *slot, PyObject *func, PyObject *const *args,
                              Py_ssize_t nargs, PyObject *kwnames, PyObject **result)
{
    PyObject *call[PYCC_FWD_MAX_PARAMS];
    PyObject *defaults, *kwdefaults;
    Py_ssize_t i, kwstart, nkw = pycc_fwd_nkw(kwnames);
    Py_ssize_t given = nargs + nkw, total = slot->argcount + slot->kwonly, ndefaults;

    if (slot->varargs || slot->varkw || slot->cell != NULL || nargs > slot->argcount
        || given > total || (nkw > 0 && nargs < slot->posonly)
        || (nkw > 0 && !pycc_fwd_keywords_follow(slot, nargs, kwnames, nkw))) {
        return 0;
    }
    for (i = 0; i < given; i++) {
        call[i] = args[i];
    }
    /* The defaults are held, not borrowed, until the body returns: a
     * keyword default's key compares with `__eq__`, and unpacking an
     * argument can run code too, and either can rebind `__defaults__` or
     * `__kwdefaults__` (or empty the dict), which would free a default the
     * call still passes. */
    defaults = PYCC_FUNC_FIELD(func, PYCC_FUNC_DEFAULTS);
    ndefaults = defaults != NULL ? PyTuple_Size(defaults) : 0;
    if (ndefaults < 0 || ndefaults > slot->argcount) {
        PyErr_Clear();
        return 0;
    }
    for (; i < slot->argcount; i++) {
        if (i < slot->argcount - ndefaults) {
            return 0;
        }
        call[i] = PyTuple_GetItem(defaults, i - (slot->argcount - ndefaults));
    }
    Py_XINCREF(defaults);
    kwdefaults = PYCC_FUNC_FIELD(func, PYCC_FUNC_KWDEFAULTS);
    kwstart = i;
    if (i < total) {
        if (kwdefaults == NULL
            || !(Py_IS_TYPE(kwdefaults, &PyDict_Type) || PyDict_Check(kwdefaults))) {
            Py_XDECREF(defaults);
            return 0;
        }
        Py_INCREF(kwdefaults);
        for (; i < total; i++) {
            PyObject *value = PyDict_GetItemWithError(kwdefaults, pycc_fwd_name(slot, i));

            if (value == NULL) {
                PyErr_Clear();
                while (--i >= kwstart) {
                    Py_DECREF(call[i]);
                }
                Py_DECREF(kwdefaults);
                Py_XDECREF(defaults);
                return 0;
            }
            call[i] = Py_NewRef(value);
        }
    }
    *result = pycc_fwd_target(slot, call, total);
    for (i = kwstart; i < total; i++) {
        Py_DECREF(call[i]);
    }
    if (kwstart < total) {
        Py_DECREF(kwdefaults);
    }
    Py_XDECREF(defaults);
    return 1;
}

/* `func(*args, **kw)` for the forwarder of `slot` (`pycc_fwd_slot_of(func)`),
 * `args[0]` its receiver, the last of the `n` arguments the values of the
 * keywords `kwnames` (or none): what its trampoline does for a call that
 * passes exactly its parameters in order, or leaves the rest to their
 * defaults (`pycc_fwd_call_in`), else the vectorcall itself. */
static PyObject *pycc_fwd_direct(struct PycFwdSlot *slot, PyObject *func, PyObject *const *args,
                                 Py_ssize_t n, PyObject *kwnames)
{
    Py_ssize_t nkw = pycc_fwd_nkw(kwnames);

    if (func == slot->func && PYCC_FUNC_FIELD(func, PYCC_FUNC_CODE) == slot->code) {
        PyObject *result;

        if (slot->simple && pycc_fwd_in_order(slot, n - nkw, kwnames)) {
            pycc_fwd_direct_calls++;
            return pycc_fwd_target(slot, args, n);
        }
        if (pycc_fwd_defaulted(slot, func, args, n - nkw, kwnames, &result)) {
            pycc_fwd_direct_calls++;
            return result;
        }
    }
    return PyObject_Vectorcall(func, args, (size_t)(n - nkw), kwnames);
}

/* Trampoline `index`'s call: straight to the compiled body when the call
 * passes exactly the parameters in order (`pycc_fwd_in_order`) or leaves
 * the rest to their defaults (`pycc_fwd_defaulted`), else `pycc_fwd_bind`. */
static PyObject *pycc_fwd_call_in(PyccExtModule *m, int index, PyObject *func,
                                  PyObject *const *args, size_t nargsf, PyObject *kwnames)
{
    PycFwdSlot *slot = &m->fwd_slots[index];
    Py_ssize_t nargs = PyVectorcall_NARGS(nargsf);

    if (PYCC_FUNC_FIELD(func, PYCC_FUNC_CODE) == slot->code) {
        PyObject *result;

        if (slot->simple && pycc_fwd_in_order(slot, nargs, kwnames)) {
            return pycc_fwd_target(slot, args, slot->argcount);
        }
        if (pycc_fwd_defaulted(slot, func, args, nargs, kwnames, &result)) {
            return result;
        }
    }
    return pycc_fwd_bind(slot, func, args, nargsf, kwnames);
}
