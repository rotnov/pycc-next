
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
 */
int pycc_ext_obj_coerce(PyObject *o, int kind, const char *what, void *out)
{
    static const char *const wanted_names[] = {"int", "float", "bool", "str"};
    PyObject *type_name;
    int admitted;
    double as_double;

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
 * A tuple (`kind` 0), set (1) or dict (2) display with an object element.
 * `items` follows `pycc_ext_obj_build_list`'s contract (every packed element
 * consumed on every path); a dict's items alternate key and value. The
 * elements are first collected into a list, which keeps that one consuming
 * path, and then converted, so insertion order and duplicate-key handling
 * are CPython's own.
 */
PyObject *pycc_ext_obj_build_display(PyObject **items, long long n, int kind)
{
    PyObject *list = pycc_ext_obj_build_list(items, n);
    PyObject *result = NULL;
    Py_ssize_t i;

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
