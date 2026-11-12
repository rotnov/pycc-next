/*
 * A comprehension that unpacks its item into two names
 * (`[f(k, v) for k, v in src]`, `crates/pycc_codegen/src/object_pairs.rs`):
 * one step of its loop binds the two names straight from the source, with no
 * item read twice by subscript (docs/design/compile-little.md, section 2.6).
 *
 * `state` is four `long long`s the generated code keeps: what is iterated
 * (`0` an iterator, `2` an exact `dict` whose `items()` the comprehension
 * runs over, not started yet, `1` that dict once started), the position
 * `PyDict_Next` resumes from, the dict's size when the iteration began, and
 * how many items it may still hand out. Over an exact dict the step is
 * `PyDict_Next`: no view, no iterator and no pair tuple are made, and the
 * checks are those of the dict's items iterator (`dictiter_iternextitem`):
 * a size changed since the iteration began raises `RuntimeError:
 * dictionary changed size during iteration` at the next step, and an item
 * beyond the count it began with (keys replaced, the size kept) raises
 * `RuntimeError: dictionary keys changed during iteration`.
 *
 * Over an iterator the item is unpacked as `UNPACK_SEQUENCE` unpacks it
 * (`pycc_ext_obj_unpack`, with CPython's messages) and released at once, as
 * CPython drops it once unpacked.
 *
 * Returns 1 with a new reference to each half in `out[0]` and `out[1]`, 0
 * once the source is exhausted, or -1 with the exception set.
 */

/* How many steps took an exact dict's entries straight, which the tests
 * read with `ctypes` (`RUNTIME_COUNTERS` in `src/ext_build/link.rs`). */
static long long pycc_ext_dict_pair_count = 0;

long long pycc_ext_dict_pair_steps(void)
{
    return pycc_ext_dict_pair_count;
}

long long pycc_ext_obj_pair_next(PyObject *it, long long *state, PyObject **out)
{
    PyObject *item;
    PyObject *pair;

    if (it == NULL || state == NULL || out == NULL) {
        PyErr_SetString(PyExc_SystemError, "pycc_ext_obj_pair_next called with a NULL argument");
        return -1;
    }
    if (state[0] != 0) {
        Py_ssize_t pos;
        PyObject *key;
        PyObject *value;

        if (state[0] == 2) {
            state[0] = 1;
            state[1] = 0;
            state[2] = state[3] = (long long)PyDict_Size(it);
        } else if ((long long)PyDict_Size(it) != state[2]) {
            PyErr_SetString(PyExc_RuntimeError, "dictionary changed size during iteration");
            return -1;
        }
        pos = (Py_ssize_t)state[1];
        if (!PyDict_Next(it, &pos, &key, &value)) {
            return 0;
        }
        state[1] = (long long)pos;
        if (state[3] == 0) {
            PyErr_SetString(PyExc_RuntimeError, "dictionary keys changed during iteration");
            return -1;
        }
        state[3]--;
        pycc_ext_dict_pair_count++;
        out[0] = Py_NewRef(key);
        out[1] = Py_NewRef(value);
        return 1;
    }
    item = PyIter_Next(it);
    if (item == NULL) {
        return PyErr_Occurred() == NULL ? 0 : -1;
    }
    pair = pycc_ext_obj_unpack(item, 2);
    if (pair == NULL) {
        Py_DECREF(item);
        return -1;
    }
    out[0] = Py_NewRef(PyTuple_GetItem(pair, 0));
    out[1] = Py_NewRef(PyTuple_GetItem(pair, 1));
    Py_DECREF(pair);
    Py_DECREF(item);
    return 1;
}
