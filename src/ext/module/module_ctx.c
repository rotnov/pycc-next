/*
 * This module's record (`PyccExtModule`, `bridge.c`) and the entry points
 * that pass it to the runtime unit (v0.8, the whole-package build).
 *
 * The runtime unit is one copy for every module of a package's extension,
 * so a function of it that answers differently per module takes the module's
 * record as its first parameter. Compiled code and the generated companion
 * call the names below, as they called the runtime's own before; each passes
 * `&pycc_ext_mod`. The ones LLVM-generated code calls are `PYCC_ENTRY`
 * (hidden, and renamed per module in a package build by
 * `pycc_ext_member.h`); the others are this unit's own.
 */

/* The `__copy__` and slots of a carrier type created on demand
 * (`pycc_ext_carrier_type`): the shared deallocator and this module's
 * `__copy__`, no other method, and no descriptor or comparison slot,
 * because a class that has either gets a generated hidden carrier type
 * instead (#1427, #1448). */
static PyMethodDef pycc_ext_carrier_methods[] = {
    {"__copy__", (PyCFunction)(void (*)(void))pycc_ext_instance_copy, METH_NOARGS, NULL},
    {NULL, NULL, 0, NULL},
};

static PyType_Slot pycc_ext_carrier_slots[] = {
    {Py_tp_dealloc, pycc_ext_instance_dealloc},
    {Py_tp_methods, pycc_ext_carrier_methods},
    {0, NULL},
};

static PyccExtModule pycc_ext_mod = {
    PYCC_EXT_MODULE_NAME_STR,
    pycc_ext_compiled_class_isinstance,
    pycc_ext_carrier_class_isinstance,
    pycc_ext_carrier_class_copy_kinds,
    &pycc_fwd_slot_count,
    pycc_fwd_trampolines,
    pycc_ext_carrier_slots,
    NULL,
    NULL,
    NULL,
    0,
    NULL,
    NULL,
};

/* The module's `__dict__` its compiled functions read rebindable globals
 * from (`global_sync.c`), set before the body runs on CPython 3.13. Read by
 * LLVM-generated code directly. */
PYCC_ENTRY PyObject *pycc_ext_ns_dict = NULL;

static int pycc_ext_unpack_instance(PyObject *obj, const char *fn_name, Py_ssize_t index,
                                    const char *class_name, void **out)
{
    return pycc_ext_unpack_instance_in(&pycc_ext_mod, obj, fn_name, index, class_name, out);
}

static PyObject *pycc_ext_pack_instance(void *result)
{
    return pycc_ext_pack_instance_in(&pycc_ext_mod, result);
}

static PyObject *pycc_ext_pack_tuple_object(void *result)
{
    return pycc_ext_pack_tuple_object_in(&pycc_ext_mod, result);
}

static PyObject *pycc_ext_pack_str_tuple(void *result)
{
    return pycc_ext_pack_str_tuple_in(&pycc_ext_mod, result);
}

static int pycc_ext_slot_operand_is(PyObject *other, const char *class_name)
{
    return pycc_ext_slot_operand_is_in(&pycc_ext_mod, other, class_name);
}

static PyObject *pycc_ext_instance_copy(PyObject *self, PyObject *unused)
{
    (void)unused;
    return pycc_ext_instance_copy_in(&pycc_ext_mod, self);
}

static int pycc_ext_carrier_register(const char *class_name, PyObject *type)
{
    return pycc_ext_carrier_register_in(&pycc_ext_mod, class_name, type);
}

static PyObject *pycc_fwd_call(int index, PyObject *func, PyObject *const *args, size_t nargsf,
                               PyObject *kwnames)
{
    return pycc_fwd_call_in(&pycc_ext_mod, index, func, args, nargsf, kwnames);
}

PYCC_ENTRY PyObject *pycc_ext_obj_pack_instance(void *inst)
{
    return pycc_ext_obj_pack_instance_in(&pycc_ext_mod, inst);
}

PYCC_ENTRY PyObject *pycc_ext_obj_pack_list(void *list)
{
    return pycc_ext_obj_pack_list_in(&pycc_ext_mod, list);
}

PYCC_ENTRY void *pycc_ext_obj_tuple_rebox(void *tuple, const unsigned char *kinds, long long n)
{
    return pycc_ext_obj_tuple_rebox_in(&pycc_ext_mod, tuple, kinds, n);
}

PYCC_ENTRY int pycc_ext_obj_isinstance_compiled(PyObject *o, const char *name)
{
    return pycc_ext_obj_isinstance_compiled_in(&pycc_ext_mod, o, name);
}

PYCC_ENTRY int pycc_ext_obj_unbox_instance(PyObject *o, const char *class_name, void **out)
{
    return pycc_ext_obj_unbox_instance_in(&pycc_ext_mod, o, class_name, out);
}

PYCC_ENTRY PyObject *pycc_ext_obj_import_from(const char *module_name, const char *const *fromlist,
                                              long long nfrom, long long index, long long level)
{
    return pycc_ext_obj_import_from_in(&pycc_ext_mod, module_name, fromlist, nfrom, index, level);
}

PYCC_ENTRY void pycc_ext_global_sync(const char *name, void *slot, int kind,
                                     PyccGlobalCache *cache, char *initialized)
{
    pycc_ext_global_sync_in(pycc_ext_ns_dict, name, slot, kind, cache, initialized);
}
