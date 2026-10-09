//! Part 1 of #1255: a list or set comprehension over a CPython object
//! (`object_comprehension.rs`), pinned on the LLVM IR of an `ext` object.

use super::*;
use pycc_mir::{CompSource, MirCompElt, MirComprehension};

fn var() -> MirExpr {
    MirExpr::Name {
        name: "x".to_string(),
        ty: Ty::Object,
    }
}

fn sum() -> MirExpr {
    MirExpr::BinOp {
        op: BinOpKind::Add,
        left: Box::new(MirExpr::IntLiteral(1)),
        right: Box::new(MirExpr::IntLiteral(2)),
        ty: Ty::Int,
    }
}

fn comprehension(cond: Option<MirExpr>, elt: MirCompElt) -> MirExpr {
    MirExpr::Comprehension(Box::new(MirComprehension {
        var: "x".to_string(),
        var_ty: Ty::Object,
        source: CompSource::Object(numpy_pi()),
        cond,
        elt,
    }))
}

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

/// A list and a set comprehension each iterate through the shared
/// `get_iter`/`iter_next` halves, build their result with the matching kind
/// code, pack every element (an object one by `pack_object`, an `int` one by
/// `pack_int`) into one `collect` call, and release nothing they produced.
#[test]
fn a_comprehension_over_an_object_collects_packed_elements_into_a_new_collection() {
    module_ir(
        "objcomp_module",
        vec![
            comprehension(Some(var()), MirCompElt::List(var())),
            comprehension(Some(sum()), MirCompElt::Set(sum(), None)),
        ],
        |ir| {
            for needle in [
                "call ptr @pycc_ext_obj_new_collection(i64 0)",
                "call ptr @pycc_ext_obj_new_collection(i64 1)",
                "@pycc_ext_obj_pack_object(",
                "@pycc_ext_obj_pack_int(",
                "@pycc_ext_obj_truthy(",
                "objcomp_result_failed",
                "objcomp_collect_failed",
                "foreign_iter_get_failed",
                "foreign_iter_next_fail",
                "objcomp_keep",
            ] {
                assert!(ir.contains(needle), "{needle}: {ir}");
            }
            let collects = ir
                .lines()
                .filter(|line| line.contains("call i32 @pycc_ext_obj_collect("))
                .count();
            assert_eq!(collects, 2, "{ir}");
            assert!(!ir.contains("DecRef"), "{ir}");
        },
    );
}

/// In a function body every failure edge bridges the CPython exception into
/// the enclosing handler instead of returning from module exec.
#[test]
fn a_function_body_comprehension_over_an_object_bridges_its_failures() {
    compile_ext_items_checking_ir(
        "objcomp_function",
        with_foreign_numpy(vec![MirItem::Function {
            name: "f".to_string(),
            params: vec![],
            return_ty: Ty::None,
            body: vec![
                MirStmt::ExprStmt(comprehension(None, MirCompElt::List(sum()))),
                MirStmt::Return(None),
            ],
        }]),
        |ir| {
            assert!(ir.contains("@pycc_ext_obj_new_collection("), "{ir}");
            assert!(ir.contains("@pycc_ext_obj_collect("), "{ir}");
            assert!(ir.contains("@pycc_ext_obj_error_bridge("), "{ir}");
            assert!(!ir.contains("objcomp_keep"), "{ir}");
        },
    );
}

/// `pycc_types` refuses a dict comprehension over an object (I0404), so one
/// reaching codegen is an internal error.
#[test]
#[should_panic(expected = "only a list or set comprehension of packable elements")]
fn a_dict_comprehension_over_an_object_is_an_internal_error() {
    module_ir(
        "objcomp_dict",
        vec![comprehension(
            None,
            MirCompElt::Dict {
                key: var(),
                value: var(),
            },
        )],
        |_| {},
    );
}

/// `pycc_hir` lowers the statement form of a comprehension over an object
/// to a plain assignment, so a `ListCompAssign` carrying an object source is
/// an internal error rather than a native loop over a pointer.
#[test]
#[should_panic(expected = "reached the native comprehension loop")]
fn a_statement_form_over_an_object_is_an_internal_error() {
    compile_ext_items_checking_ir(
        "objcomp_stmt",
        with_foreign_numpy(vec![MirItem::TopLevelStmt(MirStmt::ListCompAssign {
            target: "xs".to_string(),
            var: "x".to_string(),
            var_ty: Ty::Object,
            source: CompSource::Object(numpy_pi()),
            cond: None,
            elt: Box::new(var()),
        })]),
        |_| {},
    );
}
