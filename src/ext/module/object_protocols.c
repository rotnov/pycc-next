/*
 * Non-strict mode's object operations, continued from `object_ops.c` (the
 * next fragment of the one shim translation unit, `src/ext_build.rs`): the
 * `None`/`NotImplemented`/`Ellipsis` singletons, `hash()`, `isinstance()`, list
 * displays, attribute stores and deletes, `raise`, and iteration over CPython
 * objects.
 */

/*
 * Part 1 of #1371: a *borrowed* pointer to CPython's `None` singleton, the
 * operand compiled code compares an object against for `o is None`
 * (`EXT_OBJ_NONE_SYMBOL`). `None` is immortal, so the borrow never dangles.
 */
PyObject *pycc_ext_obj_none(void)
{
    return Py_None;
}

/*
 * #1418: a *borrowed* pointer to CPython's `NotImplemented` singleton, the
 * value of an admitted `return NotImplemented` in a comparison method
 * (`EXT_OBJ_NOT_IMPLEMENTED_SYMBOL`). It is immortal on CPython 3.13+, so a
 * compiled return may hand the borrow back as is; when the result crosses
 * the export boundary, `pycc_ext_pack_object` takes the strong reference
 * (`Py_NewRef`) the host caller owns.
 */
PyObject *pycc_ext_obj_not_implemented(void)
{
    return Py_NotImplemented;
}

/*
 * The value of a `...` literal (`EXT_OBJ_ELLIPSIS_SYMBOL`): a borrowed
 * pointer to CPython's immortal `Ellipsis`, handed out exactly like
 * `NotImplemented` above.
 */
PyObject *pycc_ext_obj_ellipsis(void)
{
    return Py_Ellipsis;
}

/*
 * `hash(o)` of a CPython object (`EXT_OBJ_HASH_SYMBOL`): writes
 * `PyObject_Hash(o)` through `*out` and answers 0, or answers -1 with the
 * exception CPython raised (an unhashable object's `TypeError`, a raising
 * `__hash__`) set. `o` is borrowed.
 */
int pycc_ext_obj_hash(PyObject *o, long long *out)
{
    Py_hash_t hash;

    if (o == NULL || out == NULL) {
        return -1;
    }
    hash = PyObject_Hash(o);
    if (hash == -1 && PyErr_Occurred()) {
        return -1;
    }
    *out = (long long)hash;
    return 0;
}

/*
 * Part 1 of #1371: `isinstance(o, cls)` with an object `o`
 * (`EXT_OBJ_ISINSTANCE_SYMBOL`). Both operands are borrowed. A `NULL`
 * `cls` selects a builtin class by `builtin`: 0 `int`, 1 `str`, 2 `float`,
 * 3 `bool`, and since Part 7 of #1371 4 `list`, 5 `dict`, 6 `tuple`
 * (`pycc_mir::ObjBuiltinClass::shim_code`). Returns 1, 0, or -1
 * with the exception set (`PyObject_IsInstance` raises `TypeError` for a
 * `cls` that is not a class, and propagates a raising
 * `__instancecheck__`).
 */
/*
 * Whether `type` is one of the built-in types whose instances' `__class__`
 * is their type and cannot be overridden: an exact `dict`, `list`, `tuple`,
 * `str`, `int`, `float`, `bool`, `set`, `frozenset` or `None`.
 */
static int pycc_ext_plain_builtin(PyTypeObject *type)
{
    return type == &PyDict_Type || type == &PyList_Type || type == &PyTuple_Type
           || type == &PyUnicode_Type || type == &PyLong_Type || type == &PyFloat_Type
           || type == &PyBool_Type || type == &PySet_Type || type == &PyFrozenSet_Type
           || (PyObject *)type == (PyObject *)Py_TYPE(Py_None);
}

/*
 * `isinstance(o, cls)`: `PyObject_IsInstance`, answered directly when `o` is
 * an instance of a plain built-in type and `cls` is a class whose own type
 * is `type` (no `__instancecheck__` of a metaclass), or a tuple of such
 * classes. That is the answer `PyObject_IsInstance` reaches too: the type
 * test, and for a miss a `__class__` lookup that for these objects names
 * the same type again.
 */
static int pycc_ext_isinstance(PyObject *o, PyObject *cls)
{
    Py_ssize_t i;
    Py_ssize_t n;
    PyObject *item;

    /* `PyObject_IsInstance`'s own first test: an object of exactly that
     * class, which no `__instancecheck__` can overrule. */
    if ((PyObject *)Py_TYPE(o) == cls) {
        return 1;
    }
    if (pycc_ext_plain_builtin(Py_TYPE(o))) {
        if (PyType_CheckExact(cls)) {
            return PyType_IsSubtype(Py_TYPE(o), (PyTypeObject *)cls);
        }
        if (PyTuple_CheckExact(cls)) {
            n = PyTuple_Size(cls);
            for (i = 0; i < n; i++) {
                item = PyTuple_GetItem(cls, i);
                if (item == NULL || !PyType_CheckExact(item)) {
                    break;
                }
                if (PyType_IsSubtype(Py_TYPE(o), (PyTypeObject *)item)) {
                    return 1;
                }
            }
            if (i == n) {
                return 0;
            }
        }
    }
    return PyObject_IsInstance(o, cls);
}

int pycc_ext_obj_isinstance(PyObject *o, PyObject *cls, int builtin)
{
    if (cls == NULL) {
        switch (builtin) {
        case 0:
            cls = (PyObject *)&PyLong_Type;
            break;
        case 1:
            cls = (PyObject *)&PyUnicode_Type;
            break;
        case 2:
            cls = (PyObject *)&PyFloat_Type;
            break;
        case 3:
            cls = (PyObject *)&PyBool_Type;
            break;
        case 4:
            cls = (PyObject *)&PyList_Type;
            break;
        case 5:
            cls = (PyObject *)&PyDict_Type;
            break;
        case 6:
            cls = (PyObject *)&PyTuple_Type;
            break;
        default:
            PyErr_SetString(PyExc_SystemError,
                            "pycc_ext_obj_isinstance: unrecognized builtin class selector");
            return -1;
        }
    }
    if (o == NULL) {
        PyErr_SetString(PyExc_SystemError, "pycc_ext_obj_isinstance: NULL operand");
        return -1;
    }
    return pycc_ext_isinstance(o, cls);
}

/*
 * The answer for a compiled class no published type descends from (a
 * private class, one exporting no method that the host cannot construct
 * either, every class of an embedded build), asked of an object that carries no compiled instance (a carrier,
 * #1435, is answered before this is reached): no such object is an
 * instance of the class, but CPython's own
 * `isinstance` does not answer False before it has looked up the object's
 * `__class__` (`object_isinstance` in `Objects/abstract.c`), so an error
 * raised there propagates as -1 here too. A missing `__class__` is 0, and
 * whatever it names cannot be a subclass of a class with no type object.
 * The generated `pycc_ext_compiled_class_isinstance` calls it by the name
 * `UNPUBLISHED_CLASS_ISINSTANCE` in `src/ext_build/method_types.rs` spells.
 */
static int pycc_ext_unpublished_class_isinstance(PyObject *o)
{
    PyObject *cls = NULL;
    if (PyObject_GetOptionalAttrString(o, "__class__", &cls) < 0) {
        return -1;
    }
    Py_XDECREF(cls);
    return 0;
}

/*
 * Part 2d of #1371: a list display bound to an object slot, `x: object =
 * [a, b]` (`EXT_OBJ_BUILD_LIST_SYMBOL` in `crates/pycc_codegen/src/ext.rs`).
 *
 * `items` holds `n` *packed* elements, in source order, each a new
 * reference a `pycc_ext_obj_pack_*` helper produced. Every one is consumed
 * on every path, exactly as `pycc_ext_obj_call` consumes its arguments: a
 * `NULL` element is a packer that already set the exception, so the list is
 * never built and the rest are released; a failed `PyList_New` releases
 * them all. Otherwise each reference moves into the fresh list through
 * `PyList_SetItem`, which steals it -- the limited API (abi3) has no
 * `PyList_SET_ITEM`. The index is always in range of a list just built
 * with `n` slots, so it cannot fail; the check is defence in depth and
 * releases what has not moved yet. The result is a *new* reference, owned
 * as `pycc_ext_obj_call`'s result is (released when an unbound temporary,
 * Part 1 of #1092), or `NULL` with the exception set.
 */
PyObject *pycc_ext_obj_build_list(PyObject **items, long long n)
{
    PyObject *list = NULL;
    int failed = 0;
    long long i;

    for (i = 0; i < n; i++) {
        if (items[i] == NULL) {
            failed = 1;
        }
    }
    if (!failed) {
        list = PyList_New((Py_ssize_t)n);
    }
    if (list == NULL) {
        for (i = 0; i < n; i++) {
            Py_XDECREF(items[i]);
        }
        return NULL;
    }
    for (i = 0; i < n; i++) {
        if (PyList_SetItem(list, (Py_ssize_t)i, items[i]) < 0) {
            for (i = i + 1; i < n; i++) {
                Py_DECREF(items[i]);
            }
            Py_DECREF(list);
            return NULL;
        }
    }
    return list;
}

/*
 * #1457 (Part 2 of #1443): `o.name = value` with a CPython object `o`
 * (`EXT_OBJ_SETATTR_SYMBOL` in `crates/pycc_codegen/src/ext.rs`).
 *
 * `o` is borrowed; `value` is a packer's new reference, consumed on every
 * path. A NULL `value` is a failed packer (an out-of-range bigint's
 * `OverflowError`, say) whose exception is already set, and it must not
 * reach `PyObject_SetAttr`: a NULL value there *deletes* the
 * attribute. A NULL `o` is guarded as `pycc_ext_obj_getattr` guards it.
 * Returns `0`, or `-1` with the exception set -- whatever the object's own
 * `__setattr__`, a read-only property or a `__slots__` layout raises.
 */
int pycc_ext_obj_setattr(PyObject *o, const char *name, PyObject *value)
{
    PyObject *attr_name;
    int status;

    if (value == NULL) {
        return -1;
    }
    if (o == NULL) {
        Py_DECREF(value);
        return -1;
    }
    attr_name = pycc_ext_attr_name(name);
    status = attr_name == NULL ? -1 : PyObject_SetAttr(o, attr_name, value);
    Py_DECREF(value);
    return status;
}

/*
 * #1457: `del o.name` with a CPython object `o`
 * (`EXT_OBJ_DELATTR_SYMBOL` in `crates/pycc_codegen/src/ext.rs`), `o`
 * borrowed. Returns `0`, or `-1` with the exception set: a missing
 * attribute raises `AttributeError`, exactly as CPython's own `del` does.
 */
int pycc_ext_obj_delattr(PyObject *o, const char *name)
{
    PyObject *attr_name;

    if (o == NULL) {
        return -1;
    }
    attr_name = pycc_ext_attr_name(name);
    return attr_name == NULL ? -1 : PyObject_DelAttr(o, attr_name);
}

/*
 * Part 9 of #1371: `raise o` with a CPython object `o`
 * (`EXT_OBJ_RAISE_SYMBOL` in `crates/pycc_codegen/src/ext.rs`).
 *
 * Decides what is raised in the order CPython's own `raise` does:
 *
 *  - an exception class is called with no arguments, and its result must
 *    be an exception instance: a raising constructor's own exception is the
 *    one raised, and any other result raises `TypeError` with CPython's
 *    own message;
 *  - an exception instance is raised as it is;
 *  - anything else raises `TypeError: exceptions must derive from
 *    BaseException`.
 *
 * The instance is set with `PyErr_SetObject`, as CPython's `raise` does,
 * not `PyErr_SetRaisedException`: it keeps the instance's identity and also
 * sets its implicit `__context__` from the exception CPython is handling,
 * such as the host's own `except` block around the call. A pycc `except`
 * handler is not a CPython handler, so an object raised inside one gets no
 * `__context__` from the pycc exception it handles.
 *
 * It then hands the CPython exception to `pycc_ext_obj_error_bridge`, which
 * makes it a pending pycc exception and keeps the original in the bridge
 * table. The caller branches to its innermost exception target, so an
 * enclosing `try` runs in a function body and in the module body alike,
 * and an exception that escapes reaches the host as the original object.
 *
 * `o` is borrowed. Total: on return a pycc exception is always pending and
 * CPython's error indicator is clear. A NULL `o` is the same defence in
 * depth `pycc_ext_obj_get_iter` documents, and bridges a `SystemError`.
 */
void pycc_ext_obj_raise(PyObject *o)
{
    PyObject *instance;

    if (o == NULL) {
        PyErr_SetString(PyExc_SystemError, "pycc: raise of a missing CPython object");
    } else if (PyExceptionClass_Check(o)) {
        instance = PyObject_CallNoArgs(o);
        if (instance == NULL) {
            /* The constructor's own exception is the one raised. */
        } else if (!PyExceptionInstance_Check(instance)) {
            PyErr_Format(PyExc_TypeError,
                         "calling %R should have returned an instance of BaseException, not %R",
                         o, (PyObject *)Py_TYPE(instance));
            Py_DECREF(instance);
        } else {
            PyErr_SetObject((PyObject *)Py_TYPE(instance), instance);
            Py_DECREF(instance);
        }
    } else if (PyExceptionInstance_Check(o)) {
        PyErr_SetObject((PyObject *)Py_TYPE(o), o);
    } else {
        PyErr_SetString(PyExc_TypeError, "exceptions must derive from BaseException");
    }
    (void)pycc_ext_obj_error_bridge();
}

/*
 * `raise o from cause` with a CPython object `o`
 * (`EXT_OBJ_RAISE_FROM_SYMBOL` in `crates/pycc_codegen/src/ext.rs`): what
 * `pycc_ext_obj_raise` raises, with `__cause__` set as CPython's own
 * `raise ... from ...` sets it. The cause is either a CPython object
 * (`object_cause`) or a pycc exception (`native_cause`), which crosses as
 * the CPython exception it was bridged from, or else as the one the bridge
 * builds for it when it escapes. Both are borrowed.
 */
extern void pycc_rt_exception_incref(void *obj);

void pycc_ext_obj_raise_from(PyObject *o, void *native_cause, PyObject *object_cause)
{
    PyObject *cause = NULL;
    PyObject *instance = NULL;
    PyObject *foreign;

    if (object_cause != NULL) {
        cause = Py_NewRef(object_cause);
    } else if (native_cause != NULL) {
        foreign = pycc_rt_exception_foreign(native_cause);
        if (foreign != NULL) {
            cause = Py_NewRef(foreign);
        } else {
            /* The pending state takes its own reference, which raising it
             * through the bridge then releases. */
            pycc_rt_exception_incref(native_cause);
            pycc_rt_exception_raise(native_cause);
            if (pycc_ext_raise_pending()) {
                cause = PyErr_GetRaisedException();
            }
        }
    }
    if (o == NULL) {
        PyErr_SetString(PyExc_SystemError, "pycc: raise of a missing CPython object");
    } else if (PyExceptionClass_Check(o)) {
        instance = PyObject_CallNoArgs(o);
        if (instance != NULL && !PyExceptionInstance_Check(instance)) {
            PyErr_Format(PyExc_TypeError,
                         "calling %R should have returned an instance of BaseException, not %R",
                         o, (PyObject *)Py_TYPE(instance));
            Py_CLEAR(instance);
        }
    } else if (PyExceptionInstance_Check(o)) {
        instance = Py_NewRef(o);
    } else {
        PyErr_SetString(PyExc_TypeError, "exceptions must derive from BaseException");
    }
    if (instance != NULL) {
        if (cause != NULL && cause != Py_None && !PyExceptionInstance_Check(cause)) {
            if (PyExceptionClass_Check(cause)) {
                PyObject *made = PyObject_CallNoArgs(cause);
                Py_DECREF(cause);
                cause = made;
                if (cause == NULL) {
                    Py_CLEAR(instance);
                }
            } else {
                PyErr_SetString(PyExc_TypeError,
                                "exception causes must derive from BaseException");
                Py_CLEAR(cause);
                Py_CLEAR(instance);
            }
        }
    }
    if (instance != NULL) {
        /* Steals `cause`; a NULL cause still suppresses the context. */
        PyException_SetCause(instance, cause);
        cause = NULL;
        PyErr_SetObject((PyObject *)Py_TYPE(instance), instance);
        Py_DECREF(instance);
    }
    Py_XDECREF(cause);
    (void)pycc_ext_obj_error_bridge();
}

/*
 * Part 3 of #1026 (PR 3c of #1082): `iter(o)` for a `for x in <object>:`
 * loop (`EXT_OBJ_GET_ITER_SYMBOL` in `crates/pycc_codegen/src/ext.rs`).
 *
 * `o` is borrowed. The result is a *new* reference the caller owns: since
 * Part 3 of #1092 the compiled `for` loop or comprehension releases it on
 * its normal exit and on every failure edge that leaves it, as
 * `docs/RUNTIME.md` records.
 *
 * A non-iterable operand makes `PyObject_GetIter` set `TypeError` and
 * return NULL, which the caller routes to the foreign failure edge, so
 * "this object cannot be iterated" needs no compile-time test: pycc knows
 * nothing about the pointee and could not perform one.
 *
 * The NULL guard is the same defence in depth `pycc_ext_obj_len` documents.
 */
PyObject *pycc_ext_obj_get_iter(PyObject *o)
{
    if (o == NULL) {
        return NULL;
    }
    return PyObject_GetIter(o);
}

/*
 * Part 3 of #1026 (PR 3c of #1082): one step of a `for x in <object>:` loop
 * (`EXT_OBJ_ITER_NEXT_SYMBOL` in `crates/pycc_codegen/src/ext.rs`).
 *
 * Three-valued, unlike every other helper here:
 *
 *   1  -- a *new* reference to the next item has been written to `*out`;
 *   0  -- the iterator is cleanly exhausted, `*out` untouched;
 *  -1  -- iteration failed with a CPython exception already set.
 *
 * `PyIter_Next` collapses the last two into NULL and only `PyErr_Occurred()`
 * tells them apart. That discrimination lives here rather than in emitted
 * LLVM IR on purpose: it is a CPython calling convention, and open-coding it
 * in the code generator would put a second, independently maintained copy of
 * that convention in a place where it could silently drift. It is also what
 * keeps *exhaustion off the failure edge* -- a loop that simply ends is not
 * a foreign failure, and fusing the two would have made every `for` loop
 * over a foreign object terminate the module body.
 *
 * Each item written through `*out` is a new reference, which the `for`
 * target or comprehension variable owns and releases on the next trip.
 *
 * `it` and `out` are NULL-guarded as defence in depth, exactly like
 * `pycc_ext_obj_len`'s own operand: returning -1 without setting an
 * exception would be wrong, so this path sets one itself -- unlike the
 * NULL-returning helpers above, whose caller has already seen a real
 * CPython exception from the producer that returned NULL.
 */
long long pycc_ext_obj_iter_next(PyObject *it, PyObject **out)
{
    PyObject *item;

    if (it == NULL || out == NULL) {
        PyErr_SetString(PyExc_SystemError,
                        "pycc_ext_obj_iter_next called with a NULL argument");
        return -1;
    }
    item = PyIter_Next(it);
    if (item != NULL) {
        *out = item;
        return 1;
    }
    return PyErr_Occurred() == NULL ? 0 : -1;
}

