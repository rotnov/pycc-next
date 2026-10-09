//! Variant coverage for `MirExpr::ty`.
//!
//! This test exercises the crate root's `impl MirExpr { fn ty }` rather than
//! `expr::lower_expr`, so it lives in its own module instead of `expr.rs`.

use crate::*;
use pycc_hir::{BinOpKind, CmpOpKind, Ty};

#[test]
fn mir_expr_ty_covers_every_variant() {
    assert_eq!(MirExpr::IntLiteral(1).ty(), Ty::Int);
    assert_eq!(MirExpr::FloatLiteral(1.0).ty(), Ty::Float);
    assert_eq!(MirExpr::BoolLiteral(true).ty(), Ty::Bool);
    assert_eq!(MirExpr::StringLiteral("s".to_string()).ty(), Ty::Str);
    assert_eq!(MirExpr::FString(vec![]).ty(), Ty::Str);
    assert_eq!(MirExpr::NotImplemented.ty(), Ty::Object);
    assert_eq!(
        MirExpr::Name {
            name: "x".to_string(),
            ty: Ty::Int
        }
        .ty(),
        Ty::Int
    );
    assert_eq!(
        MirExpr::Call {
            callee: "f".to_string(),
            args: vec![],
            ty: Ty::Bool
        }
        .ty(),
        Ty::Bool
    );
    assert_eq!(
        MirExpr::BinOp {
            op: BinOpKind::Add,
            left: Box::new(MirExpr::IntLiteral(1)),
            right: Box::new(MirExpr::IntLiteral(2)),
            ty: Ty::Int,
        }
        .ty(),
        Ty::Int
    );
    assert_eq!(
        MirExpr::Compare {
            op: CmpOpKind::Eq,
            left: Box::new(MirExpr::IntLiteral(1)),
            right: Box::new(MirExpr::IntLiteral(2)),
            ty: Ty::Bool,
        }
        .ty(),
        Ty::Bool
    );
    assert_eq!(
        MirExpr::ListLiteral(vec![MirExpr::IntLiteral(1)]).ty(),
        Ty::List(Box::new(Ty::Int))
    );
    assert_eq!(
        MirExpr::Subscript {
            base: Box::new(MirExpr::Name {
                name: "x".to_string(),
                ty: Ty::List(Box::new(Ty::Int)),
            }),
            index: Box::new(MirExpr::IntLiteral(0)),
        }
        .ty(),
        Ty::Int
    );
    assert_eq!(
        MirExpr::ListAppend {
            list: crate::MirContainerReceiver::Name("x".to_string()),
            value: Box::new(MirExpr::IntLiteral(1)),
        }
        .ty(),
        Ty::None
    );
    assert_eq!(
        MirExpr::DictLiteral(vec![(
            MirExpr::StringLiteral("a".to_string()),
            MirExpr::IntLiteral(1)
        )])
        .ty(),
        Ty::Dict(Box::new((Ty::Str, Ty::Int)))
    );
    assert_eq!(
        MirExpr::DictGet {
            dict: Box::new(MirExpr::Name {
                name: "x".to_string(),
                ty: Ty::Dict(Box::new((Ty::Str, Ty::Int))),
            }),
            key: Box::new(MirExpr::StringLiteral("a".to_string())),
        }
        .ty(),
        Ty::Int
    );
    assert_eq!(
        MirExpr::SetLiteral {
            elements: vec![MirExpr::IntLiteral(1), MirExpr::IntLiteral(2)],
            ops: None,
        }
        .ty(),
        Ty::Set(Box::new(Ty::Int))
    );
    assert_eq!(
        MirExpr::TupleLiteral(vec![MirExpr::IntLiteral(1), MirExpr::BoolLiteral(true)]).ty(),
        Ty::Tuple(Box::new(vec![Ty::Int, Ty::Bool]))
    );
    assert_eq!(
        MirExpr::Slice {
            base: Box::new(MirExpr::Name {
                name: "x".to_string(),
                ty: Ty::List(Box::new(Ty::Int)),
            }),
            start: Some(Box::new(MirExpr::IntLiteral(1))),
            stop: None,
            step: None,
        }
        .ty(),
        Ty::List(Box::new(Ty::Int))
    );
    assert_eq!(
        MirExpr::ListPop {
            list: crate::MirContainerReceiver::Name("x".to_string()),
            ty: Ty::Int,
        }
        .ty(),
        Ty::Int
    );
    assert_eq!(
        MirExpr::DictGetOrDefault {
            dict: crate::MirContainerReceiver::Name("d".to_string()),
            key: Box::new(MirExpr::StringLiteral("a".to_string())),
            default: Box::new(MirExpr::IntLiteral(0)),
            ty: Ty::Int,
        }
        .ty(),
        Ty::Int
    );
    assert_eq!(
        MirExpr::SetAdd {
            set: "s".to_string(),
            value: Box::new(MirExpr::IntLiteral(1)),
            ops: None,
        }
        .ty(),
        Ty::None
    );
    assert_eq!(
        MirExpr::Instantiate(Box::new(InstantiateExpr {
            ctor: "Point.__init__".to_string(),
            class_name: "C".to_string(),
            slot_names: vec!["s0".to_string(), "s1".to_string()],
            args: vec![],
            ty: Ty::Instance(Box::new("Point".to_string())),
        }))
        .ty(),
        Ty::Instance(Box::new("Point".to_string()))
    );
    assert_eq!(
        MirExpr::AttrGet {
            base: Box::new(MirExpr::Name {
                name: "p".to_string(),
                ty: Ty::Instance(Box::new("Point".to_string())),
            }),
            slot: 0,
            ty: Ty::Int,
        }
        .ty(),
        Ty::Int
    );
    // `NoneLiteral`/`OptionalWrap` (D-197, #763, Part 1 of #747).
    assert_eq!(MirExpr::NoneLiteral.ty(), Ty::None);
    assert_eq!(
        MirExpr::OptionalWrap(Box::new(MirExpr::IntLiteral(1)), Box::new(Ty::Int)).ty(),
        Ty::Optional(Box::new(Ty::Int))
    );
    // `OptionalWrap`'s `.ty()` reports the wrapper's own declared `inner`,
    // not the wrapped value's own static type -- exercised by wrapping a
    // `NoneLiteral` (statically `Ty::None`) rather than an `IntLiteral`.
    assert_eq!(
        MirExpr::OptionalWrap(Box::new(MirExpr::NoneLiteral), Box::new(Ty::Int)).ty(),
        Ty::Optional(Box::new(Ty::Int))
    );
    // `Not` (#604, Part 3 of #573): always `bool`, regardless of the
    // operand's own type -- truthiness always produces a boolean result.
    assert_eq!(
        MirExpr::Not(Box::new(MirExpr::IntLiteral(0))).ty(),
        Ty::Bool
    );
    // `BoolOp` (#1211): the stored type -- the join in value context,
    // `bool` in truth context.
    for (truth_only, ty) in [(false, Ty::Int), (true, Ty::Bool)] {
        assert_eq!(
            MirExpr::BoolOp {
                op: BoolOpKind::Or,
                left: Box::new(MirExpr::BoolLiteral(true)),
                right: Box::new(MirExpr::IntLiteral(0)),
                ty: ty.clone(),
                truth_only,
            }
            .ty(),
            ty
        );
    }
    // `CompareChain` (#1212): always `bool`.
    assert_eq!(
        MirExpr::CompareChain {
            first: Box::new(MirExpr::IntLiteral(0)),
            links: vec![
                MirCompareLink {
                    kind: MirCompareKind::Plain(CmpOpKind::Lt),
                    right: MirExpr::IntLiteral(1),
                },
                MirCompareLink {
                    kind: MirCompareKind::Plain(CmpOpKind::Lt),
                    right: MirExpr::IntLiteral(2),
                },
            ],
        }
        .ty(),
        Ty::Bool
    );
}
