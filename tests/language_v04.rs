//! v0.4 language gaps (ROADMAP v0.4 accept 5) against CPython 3.13:
//! natively (strict and `--non-strict`) and as an `--ext` module.

#[path = "stdlib_support/mod.rs"]
mod support;

use support::{assert_ext_matches, assert_native_matches};

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
