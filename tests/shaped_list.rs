//! A list display of elements of different types (`[name, index]`) is a
//! list of a fixed shape in strict mode (v0.8 accept 5,
//! `pycc_hir::Ty::ShapedList`): its literal index, `len`, comparison,
//! rendering, unpacking and sorting as a list element match CPython (strict,
//! `--non-strict` and `--ext`), and every operation not valid for every list
//! of that shape is a compile error.

#[path = "stdlib_support/mod.rs"]
mod support;

use support::{assert_ext_matches, assert_native_matches, check_fails};

#[test]
#[ignore = "requires CPython 3.13 on PATH"]
fn a_shaped_list_behaves_as_cpythons_list() {
    assert_native_matches("shaped_list.py");
}

#[test]
#[ignore = "requires CPython 3.13 with development headers on PATH"]
fn a_shaped_list_behaves_as_cpythons_list_in_an_ext_module() {
    assert_ext_matches("shaped_list.py", "import shaped_list\n");
}

#[test]
fn what_is_not_valid_for_every_list_of_the_shape_is_refused() {
    for (operation, error) in [
        ("b[0] = 'z'", "does not support item assignment"),
        ("b.append(1)", "does not support `.append()`"),
        ("for x in b:\n    print(x)", "cannot be iterated"),
        ("print(b + b)", "operator Add is not defined"),
        ("s = {b}", "T0038"),
        ("d = {b: 1}", "T0036"),
        ("print(hash(b))", "hash()"),
        ("print(b == ('k', 3))", "cannot compare"),
        ("print(b[-1])", "T0040"),
        ("print(b[2])", "T0040"),
        ("c = b[0:1]", "does not support slicing"),
        (
            "def f(v: tuple[str, int]) -> int:\n    return v[1]\nprint(f(b))",
            "expects `tuple[str, int]`",
        ),
    ] {
        let rendered = check_fails(&format!("b = ['k', 3]\n{operation}\n"));
        assert!(rendered.contains(error), "{operation}: {rendered}");
    }
}
