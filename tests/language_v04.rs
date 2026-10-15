//! v0.4 language gaps (ROADMAP v0.4 accept 5) against CPython 3.13:
//! natively (strict and `--non-strict`) and as an `--ext` module.

#[path = "stdlib_support/mod.rs"]
mod support;

use support::{assert_ext_matches, assert_native_matches, check_fails, fixture, outcome, python, run_native};

#[test]
#[ignore = "requires a CPython 3.13+ oracle (PYCC_PYTHON, default python3)"]
fn int_enum_members_are_ints() {
    assert_native_matches("v04_int_enum.py");
}

#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn an_ext_module_using_int_enum_members_as_ints_matches_its_source() {
    assert_ext_matches(
        "v04_int_enum_ext.py",
        "import v04_int_enum_ext as m\n\
         print(m.bump(4), m.at_least_mid(1), m.at_least_mid(2), m.label(3), m.level_text(2))\n\
         try:\n    m.label(9)\nexcept IndexError as e:\n    print('IndexError', e)\n",
    );
}

#[test]
#[ignore = "requires a CPython 3.13+ oracle (PYCC_PYTHON, default python3)"]
fn dataclass_functions_match_cpython() {
    assert_native_matches("v04_dataclass_functions.py");
}

#[test]
#[ignore = "requires a CPython 3.13+ oracle (PYCC_PYTHON, default python3)"]
fn asdict_of_mixed_fields_matches_cpython_in_non_strict_mode() {
    let source = fixture("v04_dataclass_asdict_mixed.py");
    let expected = outcome(
        &std::process::Command::new(python())
            .arg(&source)
            .output()
            .expect("the oracle runs"),
    );
    assert_eq!(
        run_native("v04_asdict_mixed", &source, &["--release", "--non-strict"]),
        expected
    );
}

#[test]
fn asdict_of_mixed_fields_is_refused_in_strict_mode() {
    let rendered = check_fails(
        "from dataclasses import asdict, dataclass\n\n@dataclass\nclass P:\n    x: int\n    \
         name: str\n\nprint(asdict(P(1, 'a')))\n",
    );
    assert!(
        rendered.contains("`dataclasses.asdict()` of a `P` is valid Python but not implemented yet"),
        "{rendered}"
    );
    // `replace()` naming a field the generated `__init__` does not take.
    let rendered = check_fails(
        "from dataclasses import dataclass, field, replace\n\n@dataclass\nclass P:\n    \
         x: int\n    y: int = field(default=0, init=False)\n\nprint(replace(P(1), y=2))\n",
    );
    assert!(rendered.contains("`dataclasses.replace()` of a `P`"), "{rendered}");
}

#[test]
#[ignore = "requires a CPython 3.13+ with development headers on PATH"]
fn an_ext_module_using_dataclass_functions_matches_its_source() {
    assert_ext_matches(
        "v04_dataclass_ext.py",
        "import v04_dataclass_ext as m\n\
         print(m.as_dict(1, 2), m.as_tuple(3, 4), m.names(), m.moved(1, 2, 5))\n",
    );
}

#[test]
#[ignore = "requires a CPython 3.13+ oracle (PYCC_PYTHON, default python3)"]
fn lists_of_instances_print_by_repr() {
    assert_native_matches("v04_print_instance_lists.py");
}

#[test]
#[ignore = "requires a CPython 3.13+ oracle (PYCC_PYTHON, default python3)"]
fn instances_sort_order_and_test_set_membership() {
    assert_native_matches("v04_instance_order.py");
}

#[test]
fn ordering_instances_without_an_ordering_method_is_refused() {
    let rendered = check_fails(
        "class P:\n    def __init__(self, x: int) -> None:\n        self.x = x\n\n\
         print(len(sorted([P(1), P(2)])))\n",
    );
    assert!(
        rendered.contains("instances of `P` cannot be ordered: the class defines neither `__lt__` nor `__gt__`"),
        "{rendered}"
    );
}
