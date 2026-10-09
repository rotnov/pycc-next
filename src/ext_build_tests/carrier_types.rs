//! #1435: the shim helpers that make every pycc instance crossing into
//! CPython a `PyccExtInstance` carrier of its run-time class, and their
//! placement around the generated `pycc_ext_exports.inc` include.

use super::*;

#[test]
fn the_carrier_helpers_sit_on_the_right_side_of_the_generated_include() {
    let shim = shim_c();
    let include = shim
        .find("#include \"pycc_ext_exports.inc\"")
        .expect("the generated include");
    // A dying carrier unlinks its instance, but only when the
    // instance still names it -- a carrier superseded by a re-run
    // `__init__` no longer owns the link.
    assert!(
        shim.contains(
            "    if (inst != NULL && pycc_rt_ext_instance_carrier(inst) == (void *)self) {\n        \
             pycc_rt_ext_instance_set_carrier(inst, NULL);\n    }\n"
        ),
        "{shim}"
    );
    // The helpers the generated registration and `tp_init` call are
    // defined above the include too.
    for helper in [
        "static int pycc_ext_carrier_register(const char *class_name, PyObject *type)\n",
        "static void pycc_ext_carrier_bind(PyObject *self, void *inst)\n",
    ] {
        let at = shim.find(helper).expect(helper);
        assert!(at < include, "{helper}");
    }
    // The packer the compiled code calls is an external definition after
    // the include, where the module name macro exists.
    let packer = shim
        .find("PyObject *pycc_ext_obj_pack_instance(void *inst)\n")
        .expect("the instance packer");
    assert!(packer > include);
}
