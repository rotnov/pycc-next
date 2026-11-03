//! v0.12: both C units of an extension, the shim's and a package's, open
//! with `hot_api.c`, so every call below it takes the routes CPython's own
//! import takes (`PyObject_CallNoArgs` as a vectorcall, the singletons as
//! the stable ABI's data) instead of functions a BOLT-built interpreter
//! keeps in its cold text.

use super::*;

const ROUTES: [&str; 6] = [
    "#define Py_None (&_Py_NoneStruct)",
    "#define PyObject_CallNoArgs(f) PyObject_Vectorcall((f), NULL, 0, NULL)",
    "#define PySet_New(iterable) pycc_set_new(iterable)",
    "#define PyDict_GetItemString(dict, key) pycc_dict_get_item_string((dict), (key))",
    "#define PyMem_Calloc(n, size) pycc_calloc(PyMem_Malloc, (n), (size))",
    "#define PyTuple_SetItem(tuple, index, item) pycc_tuple_setitem((tuple), (index), (item))",
];

#[test]
fn the_runtime_and_module_units_see_the_routes_before_any_call() {
    for unit in [runtime_unit_text(), shim_split::module_unit()] {
        for route in ROUTES {
            let at = unit.find(route).expect("each unit carries the routes");
            assert!(at < unit.find("pycc_rt_int_from_i64").expect("bridge.c follows"), "{route}");
        }
    }
}

#[test]
fn the_package_unit_opens_with_the_routes() {
    // Normalized as `shim_c` normalizes the shim: a Windows checkout embeds
    // the C sources with CRLF endings.
    let finder = crate::package_build::FINDER_C.replace("\r\n", "\n");
    assert!(finder.starts_with("/*\n * How the shim (and the package unit"));
    for route in ROUTES {
        assert!(
            finder.find(route).expect("the package unit carries the routes")
                < finder.find("typedef struct PyccPkgMember").expect("finder.c follows"),
            "{route}"
        );
    }
}
