/*
 * Frameless forwarders (v0.8 accept 2's call boundary, accept 3).
 *
 * A method of a class CPython creates keeps its `def`, whose body is a
 * forwarder calling the compiled body `__pycc_liftN_m__` with every
 * parameter positionally (`src/fallback/lift.rs`). Called as written, the
 * forwarder runs as a Python frame: a bytecode frame per call, and one
 * frame more than CPython's own method on the stack `sys._getframe` and
 * tracebacks see.
 *
 * Once its statement has run, the fallback rewrite calls
 * `__pycc_forward__(qualname, ...)` with the qualified name of each method
 * or function of it whose body compiled. Each forwarder found there -- the
 * function itself, a `property`'s accessor, a `classmethod`/`staticmethod`'s
 * function, a decorator's `__wrapped__`/`wrapped`/`func` -- is patched in
 * place: its `vectorcall` becomes one of the generated trampolines
 * (`pycc_fwd_trampolines`, one per lift of the module), which binds the
 * call's arguments to the forwarder's parameters as CPython does --
 * positional, keyword, defaults, `*args`, `**kwargs` and the `__class__`
 * cell -- and calls the compiled body directly. The forwarder's code is
 * also flagged (`PYCC_CO_SIGNATURE`), so a compiled frame called through
 * the forwarder as written links past it.
 *
 * Anything the trampoline does not bind (a missing, duplicate or unexpected
 * argument, a `__code__` replaced since) goes to the function's own
 * vectorcall: CPython binds it and raises its own `TypeError`, before any
 * frame exists, or runs the forwarder, whose traceback entry is then
 * dropped.
 *
 * `PyFunctionObject`'s `vectorcall`, `func_version`, `func_code`,
 * `func_defaults`, `func_kwdefaults` and `func_closure` are mirrored for
 * CPython 3.13 and checked by `pycc_forward_probe`; a function's version
 * is cleared so no call site specializes the call past the vectorcall
 * (CPython 3.13 assigns a function no new version once cleared). With
 * compiled frames off, `__pycc_forward__` patches nothing.
 */

/* The trampolines, one per lift of the module, and how many there are, are
 * defined by the generated companion (`forward_trampolines_c` in
 * src/ext_build/exports_inc.rs) and reached through the module's record
 * (`PyccExtModule.fwd_trampolines`), as are its slots. */

#define PYCC_FUNC_CODE 48
#define PYCC_FUNC_DEFAULTS 56
#define PYCC_FUNC_KWDEFAULTS 64
#define PYCC_FUNC_CLOSURE 72
#define PYCC_FUNC_VECTORCALL 128
#define PYCC_FUNC_VERSION 136

/* `PyCodeObject`'s `co_names`, `co_argcount`, `co_posonlyargcount`,
 * `co_kwonlyargcount`, `co_nfreevars` and `co_localsplusnames` in CPython
 * 3.13 (`co_flags` is frames.c's `PYCC_CODE_FLAGS`): read directly once the
 * first forwarder's code has matched what its attributes say
 * (`pycc_fwd_code_checked`), so patching makes no attribute objects. The
 * parameters lead `co_localsplusnames` (`co_varnames`' first names) and the
 * free variables end it. */
#define PYCC_CODE_NAMES 32
#define PYCC_CODE_ARGCOUNT 52
#define PYCC_CODE_POSONLY 56
#define PYCC_CODE_KWONLY 60
#define PYCC_CODE_NLOCALSPLUS 72
#define PYCC_CODE_NFREEVARS 88
#define PYCC_CODE_LOCALSPLUSNAMES 96
#define PYCC_CODE_INT(code, offset) (*(const int *)((const char *)(code) + (offset)))
#define PYCC_CODE_OBJ(code, offset) (*(PyObject *const *)((const char *)(code) + (offset)))

#define PYCC_CO_VARARGS 0x0004
#define PYCC_CO_VARKEYWORDS 0x0008

/* The most parameters a trampoline binds; a forwarder with more runs as
 * written. */
#define PYCC_FWD_MAX_PARAMS 24

typedef struct PycFwdSlot {
    PyObject *func;    /* the patched forwarder (a strong reference) */
    PyObject *code;    /* its code when patched */
    PyObject *target;  /* the compiled body's published function */
    PyObject *self;    /* its `self` (the module), borrowed */
    /* Its C function when it is `METH_FASTCALL`, which the trampoline
     * calls directly. */
    PyObject *(*entry)(PyObject *, PyObject *const *, Py_ssize_t);
    PyObject *names;   /* `co_varnames` */
    PyObject *klass;   /* the `__class__` cell's value, once read */
    PyObject *cell;    /* the `__class__` cell, or NULL */
    int argcount;
    int posonly;
    int kwonly;
    int varargs;
    int varkw;
    int simple; /* positional parameters only, no `__class__` */
    /* The positional defaults last seen (owned) and their count. A
     * parameter's name and default are read from `names` and `defaults`
     * when a call binds them, rather than copied into the slot: a module's
     * slots are allocated together when its body starts, one per
     * forwarder, and copies of 24 of each made them 488 bytes. */
    PyObject *defaults;
    Py_ssize_t ndefaults;
} PycFwdSlot;

static vectorcallfunc pycc_fwd_original = NULL;
/* `"__pycc_lift"`, the prefix of a lift's name (`src/fallback/lift.rs`). */
static PyObject *pycc_fwd_prefix = NULL;
/* `"."`, which separates a qualified name's parts. */
static PyObject *pycc_fwd_dot = NULL;

/* `property`, `classmethod` and `staticmethod`, and what each wraps. */
static PyObject *pycc_fwd_wrapper_types[3] = {NULL, NULL, NULL};
static const char *const pycc_fwd_wrapper_attrs[3][4] = {
    {"fget", "fset", "fdel", NULL},
    {"__func__", NULL, NULL, NULL},
    {"__func__", NULL, NULL, NULL},
};

#define PYCC_FUNC_FIELD(func, offset) (*(PyObject **)((char *)(func) + (offset)))

/* `PyFunctionObject.func_dict` in CPython 3.13 (checked by the probe). */
#define PYCC_FUNC_DICT 88

/* `obj.name` for a `name` spelled by a constant of this unit, with the
 * name interned once (`pycc_ext_attr_name`): patching reads a handful of
 * attributes of every compiled method's forwarder. A new reference, or
 * NULL with an exception set. */
static PyObject *pycc_fwd_getattr(PyObject *obj, const char *name)
{
    PyObject *attr_name = pycc_ext_attr_name(name);

    return attr_name == NULL ? NULL : PyObject_GetAttr(obj, attr_name);
}

/*
 * Checks the `PyFunctionObject` mirror against a function made now, and
 * keeps the interpreter's own vectorcall of Python functions. Leaves
 * `pycc_fwd_original` NULL (no forwarder is patched) on any mismatch.
 */
static void pycc_forward_probe(void)
{
    PyObject *ns, *f = NULL, *g = NULL, *attr, *defaults = NULL, *kwdefaults = NULL;
    int ok = 0;

    if (pycc_frames_state != 1 || pycc_fwd_original != NULL) {
        return;
    }
    /* Two functions over the frames probe's template, one given defaults. */
    ns = PyDict_New();
    if (ns == NULL || PyDict_SetItemString(ns, "__builtins__", PyEval_GetBuiltins()) != 0) {
        goto done;
    }
    f = PyObject_CallFunctionObjArgs(pycc_frames_function_type, pycc_frames_template, ns, NULL);
    g = PyObject_CallFunctionObjArgs(pycc_frames_function_type, pycc_frames_template, ns, NULL);
    defaults = Py_BuildValue("(i)", 1);
    kwdefaults = Py_BuildValue("{s:i}", "c", 2);
    if (f == NULL || g == NULL || defaults == NULL || kwdefaults == NULL
        || Py_TYPE(f) != (PyTypeObject *)pycc_frames_function_type
        || PyObject_SetAttrString(f, "__defaults__", defaults) != 0
        || PyObject_SetAttrString(f, "__kwdefaults__", kwdefaults) != 0) {
        goto done;
    }
    ok = 1;
    attr = PyObject_GetAttrString(f, "__code__");
    ok &= attr != NULL && attr == PYCC_FUNC_FIELD(f, PYCC_FUNC_CODE);
    Py_XDECREF(attr);
    attr = PyObject_GetAttrString(f, "__defaults__");
    ok &= attr != NULL && attr == PYCC_FUNC_FIELD(f, PYCC_FUNC_DEFAULTS);
    Py_XDECREF(attr);
    attr = PyObject_GetAttrString(f, "__kwdefaults__");
    ok &= attr != NULL && attr == PYCC_FUNC_FIELD(f, PYCC_FUNC_KWDEFAULTS);
    Py_XDECREF(attr);
    /* No `__dict__` until one is asked for; then the field holds it. */
    ok &= PYCC_FUNC_FIELD(f, PYCC_FUNC_DICT) == NULL;
    attr = PyObject_GetAttrString(f, "__dict__");
    ok &= attr != NULL && attr == PYCC_FUNC_FIELD(f, PYCC_FUNC_DICT);
    Py_XDECREF(attr);
    ok &= PYCC_FUNC_FIELD(f, PYCC_FUNC_CLOSURE) == NULL;
    ok &= PYCC_FUNC_FIELD(f, PYCC_FUNC_VECTORCALL) != NULL
          && PYCC_FUNC_FIELD(f, PYCC_FUNC_VECTORCALL) == PYCC_FUNC_FIELD(g, PYCC_FUNC_VECTORCALL);
    if (ok) {
        static const char *const wrappers[3] = {"property", "classmethod", "staticmethod"};
        PyObject *builtins = PyEval_GetBuiltins();
        int i;

        for (i = 0; i < 3; i++) {
            PyObject *type = PyDict_GetItemString(builtins, wrappers[i]);

            ok &= type != NULL && PyType_Check(type);
            pycc_fwd_wrapper_types[i] = Py_XNewRef(type);
        }
        pycc_fwd_prefix = PyUnicode_FromString("__pycc_lift");
        pycc_fwd_dot = PyUnicode_FromString(".");
        ok &= pycc_fwd_prefix != NULL && pycc_fwd_dot != NULL;
    }
    if (ok) {
        pycc_fwd_original = *(vectorcallfunc *)((char *)f + PYCC_FUNC_VECTORCALL);
    }
done:
    PyErr_Clear();
    Py_XDECREF(f);
    Py_XDECREF(g);
    Py_XDECREF(defaults);
    Py_XDECREF(kwdefaults);
    Py_XDECREF(ns);
}

/* Drops the forwarder's own traceback entry from the exception raised now,
 * when it is the newest one. */
static void pycc_fwd_drop_entry(PycFwdSlot *slot)
{
    PyObject *exc = PyErr_GetRaisedException();
    PyObject *tb, *frame, *code, *next;

    if (exc == NULL) {
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

    if (slot->entry == NULL) {
        return PyObject_Vectorcall(slot->target, args, (size_t)n, NULL);
    }
    /* What calling the builtin would check. */
    if (Py_EnterRecursiveCall(" while calling a Python object")) {
        return NULL;
    }
    result = slot->entry(slot->self, args, n);
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
    PyObject *star = NULL, *starstar = NULL, *result;
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
        PyObject *defaults = PYCC_FUNC_FIELD(func, PYCC_FUNC_DEFAULTS);
        PyObject *kwdefaults = PYCC_FUNC_FIELD(func, PYCC_FUNC_KWDEFAULTS);

        if (defaults != slot->defaults) {
            /* `__defaults__` changed (or this is the first call): take the
             * new items. The old tuple is kept until then, so its address
             * cannot come back as another tuple's. */
            Py_ssize_t count = defaults != NULL ? PyTuple_Size(defaults) : 0;

            if (count > slot->argcount) {
                return pycc_fwd_slow(slot, func, args, nargsf, kwnames);
            }
            {
                PyObject *old = slot->defaults;

                slot->defaults = Py_XNewRef(defaults);
                Py_XDECREF(old);
            }
            slot->ndefaults = count;
        }
        for (i = 0; i < nparams; i++) {
            if (bound[i] != NULL) {
                continue;
            }
            if (i < slot->argcount && i >= slot->argcount - slot->ndefaults) {
                bound[i] = PyTuple_GetItem(slot->defaults, i - (slot->argcount - slot->ndefaults));
            } else if (i >= slot->argcount && kwdefaults != NULL) {
                bound[i] = PyDict_GetItemWithError(kwdefaults, PyTuple_GetItem(slot->names, i));
                PyErr_Clear();
            }
            if (bound[i] == NULL) {
                Py_XDECREF(starstar);
                return pycc_fwd_slow(slot, func, args, nargsf, kwnames);
            }
        }
    }
    if (slot->varargs) {
        Py_ssize_t extra = nargs > slot->argcount ? nargs - slot->argcount : 0;

        star = PyTuple_New(extra);
        if (star == NULL) {
            Py_XDECREF(starstar);
            return NULL;
        }
        for (i = 0; i < extra; i++) {
            Py_INCREF(args[slot->argcount + i]);
            PyTuple_SetItem(star, i, args[slot->argcount + i]);
        }
    }
    if (slot->varkw && starstar == NULL && (starstar = PyDict_New()) == NULL) {
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
    Py_XDECREF(star);
    Py_XDECREF(starstar);
    return result;
}

/* Trampoline `index`'s call: straight to the compiled body when the call
 * passes exactly the positional parameters, else `pycc_fwd_bind`. */
static PyObject *pycc_fwd_call_in(PyccExtModule *m, int index, PyObject *func,
                                  PyObject *const *args, size_t nargsf, PyObject *kwnames)
{
    PycFwdSlot *slot = &m->fwd_slots[index];
    Py_ssize_t nargs = PyVectorcall_NARGS(nargsf);

    if (kwnames == NULL && nargs == slot->argcount && slot->simple
        && PYCC_FUNC_FIELD(func, PYCC_FUNC_CODE) == slot->code) {
        return pycc_fwd_target(slot, args, nargs);
    }
    return pycc_fwd_bind(slot, func, args, nargsf, kwnames);
}

/* An int attribute of `code`, or -1. */
static int pycc_fwd_code_int(PyObject *code, const char *name)
{
    PyObject *value = pycc_fwd_getattr(code, name);
    long out = value != NULL ? PyLong_AsLong(value) : -1;

    Py_XDECREF(value);
    PyErr_Clear();
    return (int)out;
}

/* 1 once a code object's fields read at the offsets above matched its
 * attributes, -1 once they did not (patching then reads attributes), 0
 * before the first forwarder. */
static int pycc_fwd_code_checked = 0;

/* What patching reads of a forwarder's code. */
typedef struct {
    PyObject *names;      /* `co_names`, borrowed */
    PyObject *varnames;   /* starts with the parameters' names; owned */
    PyObject *freevar;    /* the one free variable's name, borrowed, or NULL */
    Py_ssize_t nfreevars;
    int flags, argcount, posonly, kwonly;
} PycFwdCode;

/* Reads `code` through its attributes; 0, or -1. */
static int pycc_fwd_code_attrs(PyObject *code, PycFwdCode *out, PyObject **names_owned)
{
    PyObject *freevars = pycc_fwd_getattr(code, "co_freevars");

    *names_owned = pycc_fwd_getattr(code, "co_names");
    out->names = *names_owned;
    out->varnames = pycc_fwd_getattr(code, "co_varnames");
    out->flags = pycc_fwd_code_int(code, "co_flags");
    out->argcount = pycc_fwd_code_int(code, "co_argcount");
    out->posonly = pycc_fwd_code_int(code, "co_posonlyargcount");
    out->kwonly = pycc_fwd_code_int(code, "co_kwonlyargcount");
    out->nfreevars = freevars != NULL && PyTuple_Check(freevars) ? PyTuple_Size(freevars) : -1;
    out->freevar = out->nfreevars == 1 ? PyTuple_GetItem(freevars, 0) : NULL;
    /* The name lives on in the code's `co_localsplusnames`. */
    Py_XDECREF(freevars);
    PyErr_Clear();
    return out->names != NULL && PyTuple_Check(out->names) && out->varnames != NULL
                   && PyTuple_Check(out->varnames) && out->nfreevars >= 0
               ? 0
               : -1;
}

/* Reads `code` at the offsets. */
static void pycc_fwd_code_fields(PyObject *code, PycFwdCode *out)
{
    PyObject *plus = PYCC_CODE_OBJ(code, PYCC_CODE_LOCALSPLUSNAMES);
    int nplus = PYCC_CODE_INT(code, PYCC_CODE_NLOCALSPLUS);

    out->names = PYCC_CODE_OBJ(code, PYCC_CODE_NAMES);
    out->varnames = Py_NewRef(plus);
    out->flags = PYCC_CODE_INT(code, PYCC_CODE_FLAGS);
    out->argcount = PYCC_CODE_INT(code, PYCC_CODE_ARGCOUNT);
    out->posonly = PYCC_CODE_INT(code, PYCC_CODE_POSONLY);
    out->kwonly = PYCC_CODE_INT(code, PYCC_CODE_KWONLY);
    out->nfreevars = PYCC_CODE_INT(code, PYCC_CODE_NFREEVARS);
    out->freevar = out->nfreevars == 1 ? PyTuple_GetItem(plus, nplus - 1) : NULL;
}

/* Whether `fields` (read at the offsets) say what `attrs` (read through the
 * attributes) do. */
static int pycc_fwd_code_same(PyObject *code, const PycFwdCode *fields, const PycFwdCode *attrs)
{
    Py_ssize_t i, n = PyTuple_Size(attrs->varnames);

    if (fields->names != attrs->names || fields->flags != attrs->flags
        || fields->argcount != attrs->argcount || fields->posonly != attrs->posonly
        || fields->kwonly != attrs->kwonly || fields->nfreevars != attrs->nfreevars
        || fields->freevar != attrs->freevar || !PyTuple_Check(fields->varnames)
        || PYCC_CODE_INT(code, PYCC_CODE_NLOCALSPLUS) != PyTuple_Size(fields->varnames)
        || PyTuple_Size(fields->varnames) < n) {
        return 0;
    }
    for (i = 0; i < n; i++) {
        if (PyTuple_GetItem(fields->varnames, i) != PyTuple_GetItem(attrs->varnames, i)) {
            return 0;
        }
    }
    return 1;
}

/* Reads `code` into `out` (`*owned`: a reference to drop after), at the
 * offsets once checked; 0, or -1. */
static int pycc_fwd_code_read(PyObject *code, PycFwdCode *out, PyObject **owned)
{
    PycFwdCode fields;

    *owned = NULL;
    if (pycc_fwd_code_checked == 1) {
        pycc_fwd_code_fields(code, out);
        return 0;
    }
    if (pycc_fwd_code_attrs(code, out, owned) != 0) {
        Py_XDECREF(out->varnames);
        Py_CLEAR(*owned);
        return -1;
    }
    if (pycc_fwd_code_checked == 0) {
        pycc_fwd_code_fields(code, &fields);
        pycc_fwd_code_checked = pycc_fwd_code_same(code, &fields, out) ? 1 : -1;
        Py_DECREF(fields.varnames);
    }
    return 0;
}

/*
 * Patches `func` when it is an unpatched forwarder of this module: a
 * Python function whose code names only a lift (`__pycc_lift...`) bound in
 * `ns` and has at most the `__class__` cell.
 */
static void pycc_fwd_patch(PyccExtModule *m, PyObject *ns, PyObject *func)
{
    PyObject *code, *owned, *lift, *target;
    PycFwdCode info;
    PycFwdSlot *slot;

    if (Py_TYPE(func) != (PyTypeObject *)pycc_frames_function_type
        || *(vectorcallfunc *)((char *)func + PYCC_FUNC_VECTORCALL) != pycc_fwd_original
        || m->fwd_used >= *m->fwd_slot_count) {
        return;
    }
    code = PYCC_FUNC_FIELD(func, PYCC_FUNC_CODE);
    if (pycc_fwd_code_read(code, &info, &owned) != 0) {
        PyErr_Clear();
        return;
    }
    if (PyTuple_Size(info.names) != 1 || info.nfreevars > 1) {
        goto done;
    }
    lift = PyTuple_GetItem(info.names, 0);
    if (!PyUnicode_Check(lift) || PyUnicode_Tailmatch(lift, pycc_fwd_prefix, 0, PY_SSIZE_T_MAX, -1) != 1) {
        goto done;
    }
    target = PyDict_GetItemWithError(ns, lift);
    if (target == NULL || !PyCFunction_Check(target)) {
        goto done;
    }
    slot = &m->fwd_slots[m->fwd_used];
    memset(slot, 0, sizeof(*slot));
    slot->argcount = info.argcount;
    slot->posonly = info.posonly;
    slot->kwonly = info.kwonly;
    slot->varargs = (info.flags & PYCC_CO_VARARGS) != 0;
    slot->varkw = (info.flags & PYCC_CO_VARKEYWORDS) != 0;
    if (info.flags < 0 || slot->argcount < 0 || slot->posonly < 0 || slot->kwonly < 0
        || slot->argcount + slot->kwonly > PYCC_FWD_MAX_PARAMS - 2
        || PyTuple_Size(info.varnames) < slot->argcount + slot->kwonly) {
        goto done;
    }
    if (info.nfreevars == 1) {
        PyObject *closure = PYCC_FUNC_FIELD(func, PYCC_FUNC_CLOSURE);

        if (PyUnicode_CompareWithASCIIString(info.freevar, "__class__") != 0
            || closure == NULL || PyTuple_Size(closure) != 1) {
            goto done;
        }
        slot->cell = Py_NewRef(PyTuple_GetItem(closure, 0));
    }
    slot->names = Py_NewRef(info.varnames);
    slot->func = Py_NewRef(func);
    slot->code = Py_NewRef(code);
    slot->target = Py_NewRef(target);
    slot->simple = slot->kwonly == 0 && !slot->varargs && !slot->varkw && slot->cell == NULL;
    if (PyCFunction_GetFlags(target) == METH_FASTCALL) {
        slot->self = PyCFunction_GetSelf(target);
        slot->entry = (PyObject * (*)(PyObject *, PyObject *const *, Py_ssize_t))
            PyCFunction_GetFunction(target);
    }
    *(int *)((char *)code + PYCC_CODE_FLAGS) |= PYCC_CO_SIGNATURE;
    *(uint32_t *)((char *)func + PYCC_FUNC_VERSION) = 0;
    *(vectorcallfunc *)((char *)func + PYCC_FUNC_VECTORCALL) = m->fwd_trampolines[m->fwd_used];
    m->fwd_used++;
done:
    PyErr_Clear();
    Py_DECREF(info.varnames);
    Py_XDECREF(owned);
}

static void pycc_fwd_walk(PyccExtModule *m, PyObject *ns, PyObject *obj, int depth);

/* Walks `obj.<name>` when `obj` has it. */
static void pycc_fwd_walk_attr(PyccExtModule *m, PyObject *ns, PyObject *obj, const char *name,
                               int depth)
{
    PyObject *inner = pycc_fwd_getattr(obj, name);

    if (inner != NULL && inner != obj && inner != Py_None) {
        pycc_fwd_walk(m, ns, inner, depth + 1);
    }
    Py_XDECREF(inner);
    PyErr_Clear();
}

/* Walks the functions other decorators keep in the instance dict of `obj`
 * (`functools.wraps`'s `__wrapped__`, `functools.cached_property`'s and
 * `partialmethod`'s `func`, a `wrapped` field), without running any of
 * its attribute hooks. */
static void pycc_fwd_walk_dict(PyccExtModule *m, PyObject *ns, PyObject *obj, int depth)
{
    static const char *const keys[] = {"__wrapped__", "wrapped", "func", NULL};
    PyObject *dict;
    int i;

    /* A function without a `__dict__` yet has none of them, and asking for
     * it would make one. */
    if (Py_TYPE(obj) == (PyTypeObject *)pycc_frames_function_type) {
        dict = PYCC_FUNC_FIELD(obj, PYCC_FUNC_DICT);
        if (dict == NULL) {
            return;
        }
        Py_INCREF(dict);
    } else {
        dict = PyObject_GenericGetDict(obj, NULL);
    }
    if (dict == NULL || !PyDict_Check(dict)) {
        Py_XDECREF(dict);
        PyErr_Clear();
        return;
    }
    for (i = 0; keys[i] != NULL; i++) {
        PyObject *key = pycc_ext_attr_name(keys[i]);
        PyObject *inner = key == NULL ? NULL : PyDict_GetItemWithError(dict, key);

        PyErr_Clear();

        if (inner != NULL && inner != obj) {
            Py_INCREF(inner);
            pycc_fwd_walk(m, ns, inner, depth + 1);
            Py_DECREF(inner);
        }
    }
    Py_DECREF(dict);
    PyErr_Clear();
}

/* Patches the forwarders reachable from `obj`: itself, or the function a
 * decorator wraps (`depth` bounds the walk). */
static void pycc_fwd_walk(PyccExtModule *m, PyObject *ns, PyObject *obj, int depth)
{
    int i, j;

    if (depth > 8) {
        return;
    }
    if (Py_TYPE(obj) == (PyTypeObject *)pycc_frames_function_type) {
        pycc_fwd_patch(m, ns, obj);
        pycc_fwd_walk_dict(m, ns, obj, depth);
        return;
    }
    if (PyType_Check(obj)) {
        return;
    }
    for (i = 0; i < 3; i++) {
        if (pycc_fwd_wrapper_types[i] != NULL
            && Py_TYPE(obj) == (PyTypeObject *)pycc_fwd_wrapper_types[i]) {
            for (j = 0; pycc_fwd_wrapper_attrs[i][j] != NULL; j++) {
                pycc_fwd_walk_attr(m, ns, obj, pycc_fwd_wrapper_attrs[i][j], depth);
            }
            return;
        }
    }
    pycc_fwd_walk_dict(m, ns, obj, depth);
}

/*
 * The member `qualname` (`C.m`, `Outer.Inner.m`, `f`) names in `ns`: each
 * class along the path read from its owner's `__dict__` (so no descriptor
 * runs), the first from `ns`. A new reference, or NULL.
 */
static PyObject *pycc_fwd_resolve(PyObject *ns, PyObject *qualname)
{
    PyObject *parts = PyUnicode_Split(qualname, pycc_fwd_dot, -1);
    PyObject *obj = NULL;
    Py_ssize_t i;

    if (parts == NULL) {
        return NULL;
    }
    for (i = 0; i < PyList_Size(parts); i++) {
        PyObject *part = PyList_GetItem(parts, i);
        PyObject *next;

        if (i == 0) {
            next = Py_XNewRef(PyDict_GetItemWithError(ns, part));
        } else {
            PyObject *dict = PyType_Check(obj) ? pycc_fwd_getattr(obj, "__dict__") : NULL;

            next = NULL;
            if (dict != NULL && PyMapping_GetOptionalItem(dict, part, &next) < 0) {
                next = NULL;
            }
            Py_XDECREF(dict);
        }
        Py_XDECREF(obj);
        obj = next;
        if (obj == NULL) {
            break;
        }
    }
    Py_DECREF(parts);
    return obj;
}

/* The capsule name of a module's record, the `self` of its
 * `__pycc_forward__`. */
#define PYCC_FWD_CAPSULE "pycc.module"

/* `__pycc_forward__(qualname, ...)`: patches the forwarder each lift's
 * qualified name names in the running statement's namespace, into the slots
 * of the module whose record `self` holds. */
static PyObject *pycc_ext_forward(PyObject *self, PyObject *const *args, Py_ssize_t nargs)
{
    PyObject *ns = PyEval_GetGlobals();
    PyccExtModule *m = (PyccExtModule *)PyCapsule_GetPointer(self, PYCC_FWD_CAPSULE);
    /* The last owner (`C` of `C.m`), and its `__dict__` or NULL: the names of
     * one statement are usually all one class's. */
    PyObject *owner_name = NULL, *owner_dict = NULL;
    Py_ssize_t i;

    if (m == NULL) {
        return NULL;
    }
    if (pycc_fwd_original == NULL || ns == NULL || !PyDict_Check(ns)) {
        Py_RETURN_NONE;
    }
    for (i = 0; i < nargs; i++) {
        PyObject *qualname = args[i], *obj = NULL;
        Py_ssize_t len = PyUnicode_Check(qualname) ? PyUnicode_GetLength(qualname) : -1;
        Py_ssize_t dot = len < 0 ? -2 : PyUnicode_FindChar(qualname, '.', 0, len, -1);

        if (dot >= 0) {
            PyObject *member;

            if (owner_name == NULL || PyUnicode_GetLength(owner_name) != dot
                || PyUnicode_Tailmatch(qualname, owner_name, 0, dot, -1) != 1) {
                PyObject *owner;

                Py_CLEAR(owner_name);
                Py_CLEAR(owner_dict);
                owner_name = PyUnicode_Substring(qualname, 0, dot);
                owner = owner_name == NULL ? NULL : pycc_fwd_resolve(ns, owner_name);
                owner_dict = owner != NULL && PyType_Check(owner)
                                 ? pycc_fwd_getattr(owner, "__dict__")
                                 : NULL;
                Py_XDECREF(owner);
                PyErr_Clear();
            }
            member = owner_dict == NULL ? NULL : PyUnicode_Substring(qualname, dot + 1, len);
            if (member != NULL && PyMapping_GetOptionalItem(owner_dict, member, &obj) < 0) {
                obj = NULL;
            }
            Py_XDECREF(member);
        } else if (dot == -1) {
            obj = Py_XNewRef(PyDict_GetItemWithError(ns, qualname));
        }
        if (obj != NULL) {
            pycc_fwd_walk(m, ns, obj, 0);
            Py_DECREF(obj);
        }
        PyErr_Clear();
    }
    Py_XDECREF(owner_name);
    Py_XDECREF(owner_dict);
    Py_RETURN_NONE;
}

static PyMethodDef pycc_ext_forward_def = {
    "__pycc_forward__", (PyCFunction)(void (*)(void))pycc_ext_forward, METH_FASTCALL, NULL,
};

/* Binds `__pycc_forward__` in the namespace of `module`, of record `m`, for
 * the body; 0, or -1 with an exception set. */
static int pycc_ext_forward_install(PyccExtModule *m, PyObject *module)
{
    PyObject *hook;
    PyObject *capsule;
    int status;

    if (*m->fwd_slot_count > 0 && m->fwd_slots == NULL) {
        /* Each slot is cleared when a forwarder takes it. */
        m->fwd_slots = (PycFwdSlot *)PyMem_Malloc((size_t)*m->fwd_slot_count * sizeof(PycFwdSlot));
        if (m->fwd_slots == NULL) {
            PyErr_NoMemory();
            return -1;
        }
    }
    pycc_forward_probe();
    capsule = PyCapsule_New(m, PYCC_FWD_CAPSULE, NULL);
    if (capsule == NULL) {
        return -1;
    }
    hook = PyCFunction_NewEx(&pycc_ext_forward_def, capsule, NULL);
    Py_DECREF(capsule);
    if (hook == NULL) {
        return -1;
    }
    status = PyDict_SetItemString(PyModule_GetDict(module), "__pycc_forward__", hook);
    Py_DECREF(hook);
    return status;
}

/* Removes the hook once the body has run, keeping any exception. */
static void pycc_ext_forward_remove(PyObject *module)
{
    PyObject *raised = PyErr_GetRaisedException();

    if (PyDict_DelItemString(PyModule_GetDict(module), "__pycc_forward__") != 0) {
        PyErr_Clear();
    }
    PyErr_SetRaisedException(raised);
}

