//! Part 1 of #1371: `MirExpr::ObjCompare` and `MirExpr::ObjIsInstance`
//! emission (`foreign_compare.rs`), pinned on the LLVM IR of an `ext`
//! object.

use super::*;
use pycc_mir::{CmpOpKind, ObjBuiltinClass, ObjIsInstanceClass};

fn compare(op: CmpOpKind, left: MirExpr, right: MirExpr) -> MirExpr {
    MirExpr::ObjCompare {
        op,
        left: Box::new(left),
        right: Box::new(right),
    }
}

/// Compiles `exprs` as discarded module-body statements and hands the IR
/// to `check`.
fn module_ir(label: &str, exprs: Vec<MirExpr>, check: impl Fn(&str)) {
    compile_ext_items_checking_ir(
        label,
        with_foreign_numpy(
            exprs
                .into_iter()
                .map(|expr| MirItem::TopLevelStmt(MirStmt::ExprStmt(expr)))
                .collect(),
        ),
        check,
    );
}

#[test]
fn a_rich_comparison_is_one_richcompare_call_with_cpython_s_selector() {
    module_ir(
        "obj_compare_selectors",
        vec![
            compare(CmpOpKind::Lt, numpy_pi(), MirExpr::IntLiteral(1)),
            compare(CmpOpKind::LtE, numpy_pi(), MirExpr::FloatLiteral(1.5)),
            compare(CmpOpKind::Eq, numpy_pi(), numpy_pi()),
            compare(CmpOpKind::NotEq, MirExpr::BoolLiteral(true), numpy_pi()),
            compare(
                CmpOpKind::Gt,
                numpy_pi(),
                MirExpr::StringLiteral("a".to_string()),
            ),
            compare(CmpOpKind::GtE, MirExpr::IntLiteral(2), numpy_pi()),
        ],
        |ir| {
            // Selector, then the ownership mask: bit 0 a packed left
            // operand, bit 1 a packed right one.
            for (selector, owned) in [(0, 2), (1, 2), (2, 0), (3, 1), (4, 2), (5, 1)] {
                let needle = format!("i32 {selector}, i32 {owned})");
                assert!(
                    ir.lines()
                        .any(|line| line.contains("@pycc_ext_obj_richcompare(")
                            && line.contains(&needle)),
                    "{needle}: {ir}"
                );
            }
            for packer in [
                "@pycc_ext_obj_pack_int(",
                "@pycc_ext_obj_pack_float(",
                "@pycc_ext_obj_pack_bool(",
                "@pycc_ext_obj_pack_str(",
            ] {
                assert!(ir.contains(packer), "{packer}: {ir}");
            }
            // The result is checked for `NULL`, and never released.
            assert!(ir.contains("foreign_compare_failed"), "{ir}");
            assert!(!ir.contains("DecRef"), "{ir}");
        },
    );
}

#[test]
fn an_identity_test_is_a_pointer_compare_with_no_failure_edge() {
    module_ir(
        "obj_compare_identity",
        vec![
            compare(CmpOpKind::Is, numpy_pi(), MirExpr::NoneLiteral),
            compare(CmpOpKind::IsNot, MirExpr::NoneLiteral, numpy_pi()),
            compare(CmpOpKind::Is, numpy_pi(), numpy_pi()),
        ],
        |ir| {
            assert!(ir.contains("@pycc_ext_obj_none()"), "{ir}");
            assert!(ir.contains("icmp eq ptr"), "{ir}");
            assert!(ir.contains("icmp ne ptr"), "{ir}");
            assert!(!ir.contains("pycc_ext_obj_richcompare"), "{ir}");
        },
    );
}

#[test]
fn isinstance_is_one_shim_call_with_a_builtin_selector_or_an_object_class() {
    module_ir(
        "obj_compare_isinstance",
        vec![
            MirExpr::ObjIsInstance {
                value: Box::new(numpy_pi()),
                class: ObjIsInstanceClass::Builtin(ObjBuiltinClass::Float),
            },
            MirExpr::ObjIsInstance {
                value: Box::new(numpy_pi()),
                class: ObjIsInstanceClass::Object(Box::new(numpy_pi())),
            },
            // Part 7 of #1371: a compiled class goes to its own shim with
            // the class name as a C string.
            MirExpr::ObjIsInstance {
                value: Box::new(numpy_pi()),
                class: ObjIsInstanceClass::Compiled("ParserState".to_string()),
            },
        ],
        |ir| {
            assert!(
                ir.contains("c\"ParserState\\00\""),
                "the class name is a NUL-terminated global: {ir}"
            );
            assert!(
                ir.lines()
                    .any(|line| line.contains("call i32 @pycc_ext_obj_isinstance_compiled(")),
                "{ir}"
            );
            assert!(
                ir.lines()
                    .any(|line| line.contains("@pycc_ext_obj_isinstance(")
                        && line.contains("ptr null, i32 2)")),
                "{ir}"
            );
            assert!(
                ir.lines()
                    .any(|line| line.contains("@pycc_ext_obj_isinstance(")
                        && line.contains("i32 0)")),
                "{ir}"
            );
            assert!(ir.contains("foreign_isinstance_failed"), "{ir}");
        },
    );
}

/// In a function body the same nodes take the bridge-and-branch failure
/// edge (`foreign_fail.rs`), and an `int` temporary operand is protected
/// across the right operand and released after the comparison.
#[test]
fn a_function_body_comparison_bridges_its_failure_and_releases_an_int_temporary() {
    let sum = MirExpr::BinOp {
        op: pycc_mir::BinOpKind::Add,
        left: Box::new(MirExpr::IntLiteral(1)),
        right: Box::new(MirExpr::IntLiteral(2)),
        ty: Ty::Int,
    };
    compile_ext_items_checking_ir(
        "obj_compare_function",
        with_foreign_numpy(vec![MirItem::Function {
            name: "f".to_string(),
            params: vec![],
            return_ty: Ty::None,
            body: vec![
                MirStmt::ExprStmt(compare(CmpOpKind::Eq, sum, numpy_pi())),
                MirStmt::ExprStmt(MirExpr::ObjIsInstance {
                    value: Box::new(numpy_pi()),
                    class: ObjIsInstanceClass::Builtin(ObjBuiltinClass::Int),
                }),
                MirStmt::ExprStmt(MirExpr::ObjIsInstance {
                    value: Box::new(numpy_pi()),
                    class: ObjIsInstanceClass::Compiled("ParserState".to_string()),
                }),
                MirStmt::Return(None),
            ],
        }]),
        |ir| {
            assert!(ir.contains("@pycc_ext_obj_richcompare("), "{ir}");
            assert!(ir.contains("@pycc_ext_obj_error_bridge("), "{ir}");
            assert!(ir.contains("@pycc_ext_obj_isinstance("), "{ir}");
            assert!(ir.contains("@pycc_ext_obj_isinstance_compiled("), "{ir}");
        },
    );
}
