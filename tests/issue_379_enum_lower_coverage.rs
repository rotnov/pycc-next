//! #379: Direct `lower_checked` coverage for enum error paths.
//!
//! cargo-llvm-cov 0.8.7 has a known issue (taiki-e/cargo-llvm-cov#276)
//! where coverage from different instantiations is not merged correctly
//! in the summary report. These tests call `pycc_hir::lower_checked`
//! directly to cover the enum-specific paths in `lower_class`.

use pycc_hir::lower_checked;
use pycc_parser::parse;

fn lower(source: &str) -> Result<pycc_hir::HirModule, pycc_diag::Diagnostic> {
    let module = parse(source).expect("parse should succeed");
    lower_checked(&module)
}

#[test]
fn enum_body_with_method_is_rejected_via_lower_checked() {
    let source = "class Color(Enum):\n    RED = 1\n    def f(self) -> int:\n        return 1\n";
    let err = lower(source).unwrap_err();
    assert_eq!(err.code, "C0001");
}

#[test]
fn duplicate_enum_member_is_rejected_via_lower_checked() {
    let source = "class Color(Enum):\n    RED = 1\n    RED = 2\n";
    let err = lower(source).unwrap_err();
    assert_eq!(err.code, "C0001");
}

#[test]
fn enum_member_float_value_is_rejected_via_lower_checked() {
    let source = "class Color(Enum):\n    RED = 1.5\n";
    let err = lower(source).unwrap_err();
    assert_eq!(err.code, "C0001");
}

#[test]
fn enum_member_bool_value_is_rejected_via_lower_checked() {
    let source = "class Color(Enum):\n    RED = True\n";
    let err = lower(source).unwrap_err();
    assert_eq!(err.code, "C0001");
}

#[test]
fn enum_member_non_literal_value_is_rejected_via_lower_checked() {
    let source = "x = 1\nclass Color(Enum):\n    RED = x\n";
    let err = lower(source).unwrap_err();
    assert_eq!(err.code, "C0001");
}

#[test]
fn enum_member_overflow_value_is_rejected_via_lower_checked() {
    let source = "class Color(Enum):\n    RED = 99999999999999999999999999\n";
    let err = lower(source).unwrap_err();
    assert_eq!(err.code, "C0001");
}

#[test]
fn valid_enum_class_lowers_via_lower_checked() {
    let source = "class Color(Enum):\n    RED = 1\n    GREEN = 2\n    BLUE = 3\n";
    lower(source).expect("valid enum should lower");
}

#[test]
fn single_member_enum_class_lowers_via_lower_checked() {
    let source = "class Color(Enum):\n    RED = 1\n";
    lower(source).expect("valid enum should lower");
}

#[test]
fn zero_value_enum_lowers_via_lower_checked() {
    let source = "class Color(Enum):\n    RED = 0\n";
    lower(source).expect("valid enum should lower");
}
