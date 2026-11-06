#define PY_SSIZE_T_CLEAN
#include <Python.h>
/* Callgrind collection region markers:
 *   --toggle-collect=hr_mark_region   (phase runs)
 *   --toggle-collect=hr_replay_region (per-function replay) */
__attribute__((noinline)) PyObject *hr_mark_region(PyObject *f) { return PyObject_CallNoArgs(f); }
/* call f(*args) for every args tuple in the list; minimal driver overhead */
__attribute__((noinline)) PyObject *hr_replay_region(PyObject *f, PyObject *calls)
{
    Py_ssize_t n = PyList_GET_SIZE(calls);
    for (Py_ssize_t i = 0; i < n; i++) {
        PyObject *a = PyList_GET_ITEM(calls, i);
        PyObject *r = PyObject_Vectorcall(f, &PyTuple_GET_ITEM(a, 0), PyTuple_GET_SIZE(a), NULL);
        if (!r) return NULL;
        Py_DECREF(r);
    }
    Py_RETURN_NONE;
}
static PyObject *run(PyObject *self, PyObject *f) { return hr_mark_region(f); }
static PyObject *replay(PyObject *self, PyObject *const *a, Py_ssize_t n)
{
    if (n != 2 || !PyList_CheckExact(a[1])) { PyErr_SetString(PyExc_TypeError, "replay(f, [tuple,...])"); return NULL; }
    return hr_replay_region(a[0], a[1]);
}
static PyObject *noop(PyObject *self, PyObject *const *a, Py_ssize_t n) { Py_RETURN_NONE; }
static PyMethodDef M[] = {{"run", run, METH_O, NULL},
                          {"replay", (PyCFunction)(void (*)(void))replay, METH_FASTCALL, NULL},
                          {"noop", (PyCFunction)(void (*)(void))noop, METH_FASTCALL, NULL}, {NULL}};
static struct PyModuleDef D = {PyModuleDef_HEAD_INIT, "hrmark", NULL, -1, M};
PyMODINIT_FUNC PyInit_hrmark(void) { return PyModule_Create(&D); }
