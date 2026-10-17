
/*
 * Non-strict mode (#3): CPython's own operators on a CPython object, for
 * `crates/pycc_codegen/src/foreign_arith.rs`. Appended to
 * `pycc_ext_module.c` (`SHIM_C` in `src/ext_build.rs`), so it shares that
 * file's includes and conventions.
 *
 * The packer contract is `pycc_ext_obj_richcompare`'s: an operand produced
 * by a `pycc_ext_obj_pack_*` helper is consumed on every path, including a
 * NULL one (a failed packer, whose exception is already set); every other
 * operand is borrowed. A result is a new reference, or NULL / -1 with the
 * CPython exception set, so `1 + "a"` raises CPython's exact `TypeError`.
 */

/* `op` follows `binop_selector` in `foreign_arith.rs`; 12 and up are the
 * in-place forms an augmented assignment runs (`o += x` extends a list). */
PyObject *pycc_ext_obj_binop(PyObject *l, PyObject *r, int op, int owned)
{
    PyObject *result = NULL;

    if (l != NULL && r != NULL) {
        switch (op) {
        case 0: result = PyNumber_Add(l, r); break;
        case 1: result = PyNumber_Subtract(l, r); break;
        case 2: result = PyNumber_Multiply(l, r); break;
        case 3: result = PyNumber_TrueDivide(l, r); break;
        case 4: result = PyNumber_FloorDivide(l, r); break;
        case 5: result = PyNumber_Remainder(l, r); break;
        case 6: result = PyNumber_Power(l, r, Py_None); break;
        case 7: result = PyNumber_Lshift(l, r); break;
        case 8: result = PyNumber_Rshift(l, r); break;
        case 9: result = PyNumber_And(l, r); break;
        case 10: result = PyNumber_Or(l, r); break;
        case 11: result = PyNumber_Xor(l, r); break;
        case 12: result = PyNumber_InPlaceAdd(l, r); break;
        case 13: result = PyNumber_InPlaceSubtract(l, r); break;
        case 14: result = PyNumber_InPlaceMultiply(l, r); break;
        case 15: result = PyNumber_InPlaceTrueDivide(l, r); break;
        case 16: result = PyNumber_InPlaceFloorDivide(l, r); break;
        case 17: result = PyNumber_InPlaceRemainder(l, r); break;
        case 18: result = PyNumber_InPlacePower(l, r, Py_None); break;
        case 19: result = PyNumber_InPlaceLshift(l, r); break;
        case 20: result = PyNumber_InPlaceRshift(l, r); break;
        case 21: result = PyNumber_InPlaceAnd(l, r); break;
        case 22: result = PyNumber_InPlaceOr(l, r); break;
        case 23: result = PyNumber_InPlaceXor(l, r); break;
        default:
            PyErr_SetString(PyExc_SystemError, "pycc: unknown binary operator selector");
            break;
        }
    }
    if (owned & 1) {
        Py_XDECREF(l);
    }
    if (owned & 2) {
        Py_XDECREF(r);
    }
    return result;
}

/* `op` follows `unary_selector` in `foreign_arith.rs`; `o` is borrowed. */
PyObject *pycc_ext_obj_unaryop(PyObject *o, int op)
{
    if (o == NULL) {
        return NULL;
    }
    switch (op) {
    case 0: return PyNumber_Negative(o);
    case 1: return PyNumber_Positive(o);
    case 2: return PyNumber_Invert(o);
    default:
        PyErr_SetString(PyExc_SystemError, "pycc: unknown unary operator selector");
        return NULL;
    }
}

/* `o[k] = v`: `o` borrowed, `k` and `v` packed and consumed. */
int pycc_ext_obj_setitem(PyObject *o, PyObject *k, PyObject *v)
{
    int status = -1;

    if (o != NULL && k != NULL && v != NULL) {
        status = PyObject_SetItem(o, k, v);
    }
    Py_XDECREF(k);
    Py_XDECREF(v);
    return status;
}

/*
 * A CPython object reaching a compiled `int`/`float`/`bool`/`str` slot in
 * non-strict mode (`crates/pycc_codegen/src/object_coerce.rs`): `kind` is
 * 0 int, 1 float, 2 bool, 3 str; `what` names the slot ("f() argument 1").
 * An object of another type raises `TypeError: <what> must be <type>, not
 * <type of o>`; an `int` is admitted where a `float` is expected, as the
 * numeric tower does. `o` is borrowed. Returns 0 with the native value
 * written to `out`, or -1 with the exception set.
 *
 * Kinds 4..7 read `o` as an index, through `__index__` as CPython does:
 * 4 a `range()` argument, 5 a list index, 6 a `str` index, 7 a tuple index,
 * each refusing a non-index object with CPython's own message for that
 * position. The result is always an inline `int` word, never a heap
 * bigint the caller would have to release: a sequence index beyond
 * `Py_ssize_t` raises CPython's own `IndexError` ("cannot fit 'int' into an
 * index-sized integer"), and one beyond 2**61 -- out of range for any
 * sequence there is -- is clamped to 2**61, so the sequence raises its own
 * `IndexError`; a `range()` argument beyond the inline range raises
 * `OverflowError` (`docs/DEVIATIONS.md`). Kind 8 reads a slice bound,
 * which clamps instead of raising, as CPython's `_PyEval_SliceIndex` does.
 */
extern void *pycc_rt_set_new(long long kind);
extern void pycc_rt_set_add(void *set, long long word);
extern void pycc_rt_set_decref(void *set);
extern void *pycc_rt_tuple_new(long long len, const unsigned char *kinds);
extern void pycc_rt_tuple_set_word(void *tuple, long long index, long long word);
extern void pycc_rt_tuple_set_foreign(void *tuple, void *object);
extern void pycc_rt_list_decref(void *list);

/*
 * Kind 11 of `pycc_ext_obj_coerce`: a `tuple[str, ...]` slot. A CPython
 * `tuple` of `str`s becomes a native tuple object of the same strings (each
 * sharing its CPython `str`), which shares the tuple itself when it crosses
 * back, so a copy nothing can tell from the value: a tuple is never
 * mutated. Returns 0 with the tuple in `*out`, or -1 with `TypeError`
 * (`what` names the slot) for anything else.
 */
int pycc_ext_coerce_str_tuple(PyObject *o, const char *what, void **out)
{
    PyObject *type_name;
    Py_ssize_t n;
    Py_ssize_t i;
    unsigned char *kinds;
    void *tuple;

    if (!PyTuple_Check(o)) {
        type_name = PyType_GetName(Py_TYPE(o));
        if (type_name != NULL) {
            PyErr_Format(PyExc_TypeError, "%s must be tuple, not %U", what, type_name);
            Py_DECREF(type_name);
        }
        return -1;
    }
    n = PyTuple_Size(o);
    kinds = (unsigned char *)PyMem_Malloc(n > 0 ? (size_t)n : 1);
    if (kinds == NULL) {
        PyErr_NoMemory();
        return -1;
    }
    memset(kinds, 3, n > 0 ? (size_t)n : 1); /* LIST_KIND_STR */
    tuple = pycc_rt_tuple_new((long long)n, kinds);
    PyMem_Free(kinds);
    for (i = 0; i < n; i++) {
        PyObject *item = PyTuple_GetItem(o, i);
        void *text = NULL;

        if (!PyUnicode_Check(item)) {
            type_name = PyType_GetName(Py_TYPE(item));
            if (type_name != NULL) {
                PyErr_Format(PyExc_TypeError,
                             "%s must be a tuple of str, not one holding %U", what, type_name);
                Py_DECREF(type_name);
            }
        } else if (pycc_ext_obj_unbox_str(item, &text) == 0) {
            pycc_rt_tuple_set_word(tuple, (long long)i, (long long)(intptr_t)text);
            continue;
        }
        /* The words not yet written are zero, which releasing skips. */
        pycc_rt_list_decref(tuple);
        return -1;
    }
    if (PyTuple_CheckExact(o)) {
        Py_INCREF(o);
        pycc_rt_tuple_set_foreign(tuple, o);
    }
    *out = tuple;
    return 0;
}

int pycc_ext_obj_coerce(PyObject *o, int kind, const char *what, void *out)
{
    static const char *const wanted_names[] = {"int", "float", "bool", "str"};
    PyObject *type_name;
    int admitted;
    double as_double;

    /*
     * Kinds 9 and 10: a `frozenset[str]` / `frozenset[int]` slot. A CPython
     * `set` or `frozenset` of such elements becomes a fresh native set (a
     * copy, which nothing can tell from the value: a frozenset is never
     * mutated).
     */
    if (o != NULL && out != NULL && (kind == 9 || kind == 10)) {
        PyObject *iter;
        PyObject *item;
        void *set;

        if (!PyAnySet_Check(o)) {
            type_name = PyType_GetName(Py_TYPE(o));
            if (type_name != NULL) {
                PyErr_Format(PyExc_TypeError, "%s must be frozenset, not %U", what, type_name);
                Py_DECREF(type_name);
            }
            return -1;
        }
        iter = PyObject_GetIter(o);
        if (iter == NULL) {
            return -1;
        }
        set = pycc_rt_set_new(kind == 9 ? 3 : 0);
        while ((item = PyIter_Next(iter)) != NULL) {
            long long word = 0;
            int status;

            if (kind == 9) {
                void *text = NULL;
                status = PyUnicode_Check(item) ? pycc_ext_obj_unbox_str(item, &text) : -2;
                word = (long long)(intptr_t)text;
            } else {
                status = PyLong_Check(item) && !PyBool_Check(item)
                             ? pycc_ext_obj_unbox_int(item, &word)
                             : -2;
            }
            if (status == -2) {
                type_name = PyType_GetName(Py_TYPE(item));
                if (type_name != NULL) {
                    PyErr_Format(PyExc_TypeError, "%s must be a frozenset of %s, not one holding %U",
                                 what, kind == 9 ? "str" : "int", type_name);
                    Py_DECREF(type_name);
                }
            }
            Py_DECREF(item);
            if (status != 0) {
                Py_DECREF(iter);
                pycc_rt_set_decref(set);
                return -1;
            }
            pycc_rt_set_add(set, word);
        }
        Py_DECREF(iter);
        if (PyErr_Occurred()) {
            pycc_rt_set_decref(set);
            return -1;
        }
        /* The slot is a frozenset; an exact CPython `frozenset` is the one
         * the copy shares when it crosses back (`pycc_ext_obj_pack_frozenset`). */
        pycc_rt_set_freeze(set);
        if (PyFrozenSet_CheckExact(o)) {
            Py_INCREF(o);
            pycc_rt_set_set_foreign(set, o);
        }
        *(void **)out = set;
        return 0;
    }

    if (o != NULL && out != NULL && kind == 11) {
        return pycc_ext_coerce_str_tuple(o, what, (void **)out);
    }

    if (o != NULL && out != NULL && kind >= 4 && kind <= 8) {
        PyObject *index;

        if (!PyIndex_Check(o)) {
            type_name = PyType_GetName(Py_TYPE(o));
            if (type_name == NULL) {
                return -1;
            }
            if (kind == 4) {
                PyErr_Format(PyExc_TypeError,
                             "'%U' object cannot be interpreted as an integer", type_name);
            } else if (kind == 5) {
                PyErr_Format(PyExc_TypeError,
                             "list indices must be integers or slices, not %U", type_name);
            } else if (kind == 6) {
                PyErr_Format(PyExc_TypeError, "string indices must be integers, not '%U'",
                             type_name);
            } else if (kind == 8) {
                PyErr_SetString(PyExc_TypeError, "slice indices must be integers or None or "
                                                 "have an __index__ method");
            } else {
                PyErr_Format(PyExc_TypeError,
                             "tuple indices must be integers or slices, not %U", type_name);
            }
            Py_DECREF(type_name);
            return -1;
        }
        index = PyNumber_Index(o);
        if (index == NULL) {
            return -1;
        }
        {
            const Py_ssize_t limit = (Py_ssize_t)1 << 61;
            /* A slice bound clamps (`_PyEval_SliceIndex`); an index raises. */
            Py_ssize_t n = PyNumber_AsSsize_t(index, kind == 8   ? NULL
                                                     : kind == 4 ? PyExc_OverflowError
                                                                 : PyExc_IndexError);
            Py_DECREF(index);
            if (n == -1 && PyErr_Occurred()) {
                return -1;
            }
            if (n > limit || n < -limit) {
                if (kind == 4) {
                    PyErr_SetString(PyExc_OverflowError,
                                    "range() argument too large for a compiled range");
                    return -1;
                }
                n = n > 0 ? limit : -limit;
            }
            *(long long *)out = pycc_rt_int_from_i64((long long)n);
        }

        return 0;
    }
    if (o == NULL || out == NULL || kind < 0 || kind > 3) {
        if (!PyErr_Occurred()) {
            PyErr_SetString(PyExc_SystemError, "pycc: invalid object conversion");
        }
        return -1;
    }
    switch (kind) {
    case 0: admitted = PyLong_Check(o); break;
    case 1: admitted = PyFloat_Check(o) || PyLong_Check(o); break;
    case 2: admitted = PyBool_Check(o); break;
    default: admitted = PyUnicode_Check(o); break;
    }
    if (!admitted) {
        type_name = PyType_GetName(Py_TYPE(o));
        if (type_name == NULL) {
            return -1;
        }
        PyErr_Format(PyExc_TypeError, "%s must be %s, not %U", what, wanted_names[kind],
                     type_name);
        Py_DECREF(type_name);
        return -1;
    }
    switch (kind) {
    case 0:
        return pycc_ext_obj_unbox_int(o, (long long *)out);
    case 1:
        if (PyFloat_Check(o)) {
            return pycc_ext_obj_unbox_float(o, (double *)out);
        }
        as_double = PyLong_AsDouble(o);
        if (as_double == -1.0 && PyErr_Occurred()) {
            return -1;
        }
        *(double *)out = as_double;
        return 0;
    case 2:
        return pycc_ext_obj_unbox_bool(o, (char *)out);
    default:
        return pycc_ext_obj_unbox_str(o, (void **)out);
    }
}

/*
 * A tuple (`kind` 0), set (1) or dict (2) display with an object element,
 * or (3) a set display of constants, in CPython's folded order.
 * `items` follows `pycc_ext_obj_build_list`'s contract (every packed element
 * consumed on every path); a dict's items alternate key and value. The
 * elements are first collected into a list, which keeps that one consuming
 * path, and then converted, so insertion order and duplicate-key handling
 * are CPython's own.
 */
PyObject *pycc_ext_obj_build_display(PyObject **items, long long n, int kind)
{
    PyObject *list;
    PyObject *result = NULL;
    Py_ssize_t i;
    int failed = 0;

    if (kind == 0) {
        /* A tuple is built directly: `PyTuple_SetItem` steals each item,
         * as `PyList_SetItem` does in `pycc_ext_obj_build_list`. */
        for (i = 0; i < (Py_ssize_t)n; i++) {
            if (items[i] == NULL) {
                failed = 1;
            }
        }
        result = failed ? NULL : PyTuple_New((Py_ssize_t)n);
        if (result == NULL) {
            for (i = 0; i < (Py_ssize_t)n; i++) {
                Py_XDECREF(items[i]);
            }
            return NULL;
        }
        for (i = 0; i < (Py_ssize_t)n; i++) {
            PyTuple_SetItem(result, i, items[i]);
        }
        return result;
    }
    list = pycc_ext_obj_build_list(items, n);
    if (list == NULL) {
        return NULL;
    }
    switch (kind) {
    case 0:
        result = PyList_AsTuple(list);
        break;
    case 1:
        result = PySet_New(list);
        break;
    case 3: {
        /* A display of constants: CPython folds it into a frozenset
         * constant, rebuilds that from its own order when merging
         * constants, and copies it into a new set (`SET_UPDATE`). */
        PyObject *folded = PyFrozenSet_New(list);
        PyObject *order = folded == NULL ? NULL : PySequence_Tuple(folded);
        PyObject *merged = order == NULL ? NULL : PyFrozenSet_New(order);
        result = merged == NULL ? NULL : PySet_New(merged);
        Py_XDECREF(merged);
        Py_XDECREF(order);
        Py_XDECREF(folded);
        break;
    }
    case 2:
        result = PyDict_New();
        for (i = 0; result != NULL && i + 1 < (Py_ssize_t)n; i += 2) {
            if (PyDict_SetItem(result, PyList_GetItem(list, i), PyList_GetItem(list, i + 1)) < 0) {
                Py_CLEAR(result);
            }
        }
        break;
    default:
        PyErr_SetString(PyExc_SystemError, "pycc: unknown display kind");
        break;
    }
    Py_DECREF(list);
    return result;
}

/* `del o[k]`: `o` borrowed, `k` packed and consumed. */
int pycc_ext_obj_delitem(PyObject *o, PyObject *k)
{
    int status = -1;

    if (o != NULL && k != NULL) {
        status = PyObject_DelItem(o, k);
    }
    Py_XDECREF(k);
    return status;
}

/*
 * A compiled generator crossing into CPython (`object` boxing, see
 * `crates/pycc_codegen/src/generator_box.rs`): an iterator holding one
 * reference to the native generator and the compiled function that
 * produces its next value as a new reference -- or NULL, with the
 * runtime's exception pending when the generator raised, and nothing
 * pending when it is exhausted (`tp_iternext`'s NULL-without-error then
 * means `StopIteration`). Releasing the iterator releases the generator,
 * which closes it if it is unfinished, running its `finally` clauses.
 */
extern void pycc_rt_gen_decref(void *gen);

typedef PyObject *(*pycc_ext_gen_next_fn)(void *gen);

typedef struct {
    PyObject_HEAD
    void *gen;
    pycc_ext_gen_next_fn next;
} PyccExtGenObj;

static PyObject *pycc_ext_gen_type = NULL;

static PyObject *pycc_ext_gen_iternext(PyObject *self)
{
    PyccExtGenObj *obj = (PyccExtGenObj *)self;
    PyObject *value;

    if (obj->gen == NULL) {
        return NULL;
    }
    value = obj->next(obj->gen);
    if (value == NULL && pycc_rt_ext_pending_type() >= 0) {
        pycc_ext_raise_pending();
    }
    return value;
}

extern signed char pycc_rt_gen_throw_pending(void *gen);
extern signed char pycc_rt_gen_close(void *gen);

/* The value after the generator was resumed for `send(None)`/`throw()`:
 * as `tp_iternext`, but exhaustion is CPython's `StopIteration`. */
static PyObject *pycc_ext_gen_step_result(PyObject *self)
{
    PyObject *value = pycc_ext_gen_iternext(self);

    if (value == NULL && !PyErr_Occurred()) {
        PyErr_SetNone(PyExc_StopIteration);
    }
    return value;
}

/* `g.send(value)`: a compiled generator receives nothing, so only `None`
 * (the same as `next(g)`). */
static PyObject *pycc_ext_gen_send(PyObject *self, PyObject *value)
{
    if (value != Py_None) {
        PyErr_SetString(PyExc_TypeError,
                        "pycc: a compiled generator yielding to CPython receives only None");
        return NULL;
    }
    return pycc_ext_gen_step_result(self);
}

/* `g.throw(exc)` (or the deprecated `throw(type[, value[, tb]])`): `exc`
 * raised in the generator where it is suspended, bridged into pycc's
 * exception (`pycc_ext_obj_error_bridge`), as `contextlib` throws the
 * exception of a `with` body into the generator managing it. */
static PyObject *pycc_ext_gen_throw(PyObject *self, PyObject *const *args, Py_ssize_t nargs)
{
    PyccExtGenObj *obj = (PyccExtGenObj *)self;
    PyObject *exc;

    if (nargs < 1 || nargs > 3) {
        PyErr_SetString(PyExc_TypeError, "throw expected at least 1 argument");
        return NULL;
    }
    exc = args[0];
    if (nargs >= 2 && args[1] != Py_None) {
        exc = args[1];
    }
    if (PyType_Check(exc) && PyType_IsSubtype((PyTypeObject *)exc,
                                              (PyTypeObject *)PyExc_BaseException)) {
        exc = PyObject_CallNoArgs(exc);
        if (exc == NULL) {
            return NULL;
        }
    } else if (PyExceptionInstance_Check(exc)) {
        Py_INCREF(exc);
    } else {
        PyErr_SetString(PyExc_TypeError,
                        "exceptions must be classes or instances deriving from BaseException");
        return NULL;
    }
    if (obj->gen == NULL) {
        PyErr_SetRaisedException(exc);
        return NULL;
    }
    PyErr_SetRaisedException(exc);
    pycc_ext_obj_error_bridge();
    if (!pycc_rt_gen_throw_pending(obj->gen)) {
        pycc_ext_raise_pending();
        return NULL;
    }
    return pycc_ext_gen_step_result(self);
}

/* `g.close()`: `GeneratorExit` raised where the generator is suspended. */
static PyObject *pycc_ext_gen_close(PyObject *self, PyObject *unused)
{
    PyccExtGenObj *obj = (PyccExtGenObj *)self;

    (void)unused;
    if (obj->gen != NULL && !pycc_rt_gen_close(obj->gen)) {
        pycc_ext_raise_pending();
        return NULL;
    }
    Py_RETURN_NONE;
}

static PyMethodDef pycc_ext_gen_methods[] = {
    {"send", (PyCFunction)pycc_ext_gen_send, METH_O, NULL},
    {"throw", (PyCFunction)(void (*)(void))pycc_ext_gen_throw, METH_FASTCALL, NULL},
    {"close", (PyCFunction)pycc_ext_gen_close, METH_NOARGS, NULL},
    {NULL, NULL, 0, NULL},
};

static void pycc_ext_gen_dealloc(PyObject *self)
{
    PyTypeObject *tp = Py_TYPE(self);
    PyccExtGenObj *obj = (PyccExtGenObj *)self;
    void *gen = obj->gen;
    freefunc tp_free;

    obj->gen = NULL;
    if (gen != NULL) {
        pycc_rt_gen_decref(gen);
    }
    tp_free = (freefunc)PyType_GetSlot(tp, Py_tp_free);
    tp_free(self);
    Py_DECREF(tp);
}

static PyType_Slot pycc_ext_gen_type_slots[] = {
    {Py_tp_dealloc, pycc_ext_gen_dealloc},
    {Py_tp_iter, PyObject_SelfIter},
    {Py_tp_iternext, pycc_ext_gen_iternext},
    {Py_tp_methods, pycc_ext_gen_methods},
    {0, NULL},
};

static PyType_Spec pycc_ext_gen_type_spec = {
    "builtins.generator",
    sizeof(PyccExtGenObj),
    0,
    Py_TPFLAGS_DEFAULT | Py_TPFLAGS_DISALLOW_INSTANTIATION | Py_TPFLAGS_IMMUTABLETYPE,
    pycc_ext_gen_type_slots,
};

/* Steals `gen`'s reference on every path. */
PyObject *pycc_ext_gen_wrap(void *gen, pycc_ext_gen_next_fn next)
{
    PyccExtGenObj *obj;
    allocfunc tp_alloc;

    if (pycc_ext_gen_type == NULL) {
        pycc_ext_gen_type = PyType_FromSpec(&pycc_ext_gen_type_spec);
        if (pycc_ext_gen_type == NULL) {
            pycc_rt_gen_decref(gen);
            return NULL;
        }
    }
    tp_alloc = (allocfunc)PyType_GetSlot((PyTypeObject *)pycc_ext_gen_type, Py_tp_alloc);
    obj = (PyccExtGenObj *)tp_alloc((PyTypeObject *)pycc_ext_gen_type, 0);
    if (obj == NULL) {
        pycc_rt_gen_decref(gen);
        return NULL;
    }
    obj->gen = gen;
    obj->next = next;
    return (PyObject *)obj;
}
