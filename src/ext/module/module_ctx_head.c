/*
 * The module unit's entry points into the runtime unit (v0.8, the
 * whole-package build), declared ahead of the generated companion, which
 * calls them by these names. Each is defined in `module_ctx.c` over this
 * module's record (`pycc_ext_mod`), which the runtime unit's function takes
 * as its first parameter (`PyccExtModule`, `bridge.c`).
 */
static int pycc_ext_unpack_instance(PyObject *obj, const char *fn_name, Py_ssize_t index,
                                    const char *class_name, void **out);
static PyObject *pycc_ext_pack_instance(void *result);
static PyObject *pycc_ext_pack_tuple_object(void *result);
static PyObject *pycc_ext_pack_str_tuple(void *result);
static int pycc_ext_slot_operand_is(PyObject *other, const char *class_name);
static PyObject *pycc_ext_instance_copy(PyObject *self, PyObject *unused);
static int pycc_ext_carrier_register(const char *class_name, PyObject *type);
static PyObject *pycc_fwd_call(int index, PyObject *func, PyObject *const *args, size_t nargsf,
                               PyObject *kwnames);

/* The records of this module's compiled functions (`pycc_codegen::frames`'s
 * `finish_module`), bound to the module's dict at each exec. */
extern void *const pycc_fn_info_table[];
extern const long long pycc_fn_info_count;
