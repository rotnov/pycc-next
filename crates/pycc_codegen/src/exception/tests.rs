use super::*;

fn int_binop(op: pycc_mir::BinOpKind) -> MirExpr {
    MirExpr::BinOp {
        op,
        left: Box::new(MirExpr::IntLiteral(8)),
        right: Box::new(MirExpr::IntLiteral(2)),
        ty: pycc_mir::Ty::Int,
    }
}

#[test]
fn exception_guard_classification_keeps_the_arithmetic_fast_path_clean() {
    for op in [
        pycc_mir::BinOpKind::Add,
        pycc_mir::BinOpKind::Sub,
        pycc_mir::BinOpKind::Mul,
        pycc_mir::BinOpKind::BitAnd,
        pycc_mir::BinOpKind::BitOr,
        pycc_mir::BinOpKind::BitXor,
        // These raise, but only on the runtime path, which guards
        // itself (`int_fast::emit_guarded`).
        pycc_mir::BinOpKind::FloorDiv,
        pycc_mir::BinOpKind::Mod,
        pycc_mir::BinOpKind::LShift,
        pycc_mir::BinOpKind::RShift,
    ] {
        assert!(!expression_can_set_exception(&int_binop(op)));
    }
    assert!(expression_can_set_exception(&int_binop(
        pycc_mir::BinOpKind::Pow
    )));
    // A `float` `+ - * / **` guards itself (`float_fast`): an `int`
    // operand's overflowing conversion, a zero divisor and `pow`'s
    // errors are all handled on cold paths it emits. `//` and `%` are
    // still guarded after the node, and so is `int / int`.
    let float_op = |op, left: MirExpr| MirExpr::BinOp {
        op,
        left: Box::new(left),
        right: Box::new(MirExpr::FloatLiteral(1.5)),
        ty: pycc_mir::Ty::Float,
    };
    for op in [
        pycc_mir::BinOpKind::Add,
        pycc_mir::BinOpKind::Div,
        pycc_mir::BinOpKind::Pow,
    ] {
        assert!(!expression_can_set_exception(&float_op(
            op,
            MirExpr::IntLiteral(1)
        )));
        assert!(!expression_can_set_exception(&float_op(
            op,
            MirExpr::FloatLiteral(1.0)
        )));
    }
    for op in [pycc_mir::BinOpKind::FloorDiv, pycc_mir::BinOpKind::Mod] {
        assert!(expression_can_set_exception(&float_op(
            op,
            MirExpr::FloatLiteral(1.0)
        )));
    }
    assert!(expression_can_set_exception(&MirExpr::BinOp {
        op: pycc_mir::BinOpKind::Div,
        left: Box::new(MirExpr::IntLiteral(1)),
        right: Box::new(MirExpr::IntLiteral(2)),
        ty: pycc_mir::Ty::Float,
    }));
    // `str()` of an `int` can exceed CPython's digit limit.
    assert!(expression_can_set_exception(&MirExpr::FString(vec![
        pycc_mir::MirFStringPart::Interpolation(Box::new(MirExpr::IntLiteral(1))),
    ])));
    assert!(!expression_can_set_exception(&MirExpr::FString(vec![
        pycc_mir::MirFStringPart::Literal("x".to_string()),
    ])));

    assert!(expression_can_set_exception(&MirExpr::Call {
        callee: "f".to_string(),
        args: Vec::new(),
        ty: pycc_mir::Ty::Int,
    }));
    assert!(expression_can_set_exception(&MirExpr::DictGet {
        dict: Box::new(MirExpr::DictLiteral(vec![(
            MirExpr::StringLiteral("key".to_string()),
            MirExpr::IntLiteral(1),
        )])),
        key: Box::new(MirExpr::StringLiteral("key".to_string())),
    }));
    // A list index raises through its inlined bounds check's own cold
    // branch, so neither subscript needs the flag test.
    assert!(!expression_can_set_exception(&MirExpr::Subscript {
        base: Box::new(MirExpr::ListLiteral(vec![MirExpr::IntLiteral(1)])),
        index: Box::new(MirExpr::IntLiteral(0)),
    }));
    assert!(!expression_can_set_exception(&MirExpr::Subscript {
        base: Box::new(MirExpr::TupleLiteral(vec![MirExpr::IntLiteral(1)])),
        index: Box::new(MirExpr::IntLiteral(0)),
    }));
    assert!(expression_can_set_exception(&MirExpr::Instantiate(
        Box::new(pycc_mir::InstantiateExpr {
            ctor: "C.__init__".to_string(),
            class_name: "C".to_string(),
            slot_names: vec![],
            args: Vec::new(),
            ty: pycc_mir::Ty::Instance(Box::new("C".to_string())),
        },)
    )));
    // Part 2 of #1027: a buffer element load is always raising -- its
    // base is a `memoryview`, so `pycc_rt_buffer_f64_get`'s bounds check
    // can always set an `IndexError`. Unlike `Subscript` above there is
    // no false direction to pin: no base shape makes it non-raising.
    // #1388: an instance attribute read can raise `AttributeError`, but
    // guards itself on its cold unassigned path
    // (`attr_slot::emit_slot_load_checked`), so `emit_expr` adds none.
    assert!(!expression_can_set_exception(&MirExpr::AttrGet {
        base: Box::new(MirExpr::Name {
            name: "c".to_string(),
            ty: pycc_mir::Ty::Instance(Box::new("C".to_string())),
        }),
        slot: 0,
        ty: pycc_mir::Ty::Int,
    }));
    assert!(expression_can_set_exception(&MirExpr::BufferGet {
        base: Box::new(MirExpr::Name {
            name: "b".to_string(),
            ty: pycc_mir::Ty::MemoryView,
        }),
        index: Box::new(MirExpr::IntLiteral(0)),
    }));
    assert!(!expression_can_set_exception(&MirExpr::IntLiteral(1)));
}

/// An operation on CPython objects routes its own failure through
/// `foreign_fail`'s edge as soon as its helper fails, so it leaves no pending
/// pycc exception behind and gets no guard after the node. That the routing
/// holds for each of them -- a failure caught in the same function, and one
/// propagating to its caller -- is `tests/object_failure_edges.rs`.
#[test]
fn object_operations_route_their_own_failure() {
    use pycc_mir::Ty;
    let object = || {
        Box::new(MirExpr::Name {
            name: "o".to_string(),
            ty: Ty::Object,
        })
    };
    let nodes = [
        MirExpr::ObjAttrGet {
            base: object(),
            attr: "a".to_string(),
            ty: Ty::Object,
        },
        MirExpr::ObjMethodCall {
            base: object(),
            method: "m".to_string(),
            args: Vec::new(),
        },
        MirExpr::ObjCall {
            callee: object(),
            args: Vec::new(),
        },
        MirExpr::ObjKeywordCall(Box::new(pycc_mir::ObjKeywordCall {
            call: MirExpr::ObjCall {
                callee: object(),
                args: Vec::new(),
            },
            names: vec!["k".to_string()],
            values: vec![MirExpr::IntLiteral(1)],
        })),
        MirExpr::ObjLen { base: object() },
        MirExpr::ObjSubscript {
            base: object(),
            index: object(),
        },
        MirExpr::ObjUnpackFloatTuple {
            base: object(),
            arity: 2,
        },
        MirExpr::ObjUnpack {
            value: object(),
            arity: 2,
        },
        MirExpr::ObjIsInstance {
            value: object(),
            class: pycc_mir::ObjIsInstanceClass::Object(object()),
        },
        MirExpr::ObjContains {
            negate: false,
            item: object(),
            container: object(),
        },
        MirExpr::ObjSlice {
            base: object(),
            start: None,
            stop: None,
            step: None,
        },
        MirExpr::ObjType { base: object() },
        MirExpr::ObjList {
            elements: vec![MirExpr::IntLiteral(1)],
        },
        MirExpr::ObjectBox(Box::new(MirExpr::IntLiteral(1))),
        MirExpr::ObjectUnbox(object(), Box::new(Ty::Int)),
        MirExpr::ObjectCoerce(object(), Box::new(Ty::Int)),
        MirExpr::TupleRebox(object(), Box::new(Ty::Object)),
        MirExpr::ObjectIndex(object(), pycc_mir::IndexUse::List),
        MirExpr::ObjBinOp {
            op: pycc_mir::BinOpKind::Add,
            in_place: false,
            left: object(),
            right: object(),
        },
        MirExpr::ObjDisplay {
            kind: pycc_mir::ObjDisplayKind::Dict,
            elements: vec![*object(), *object()],
        },
        MirExpr::ObjUnaryOp {
            op: pycc_mir::UnaryOpKind::USub,
            operand: object(),
        },
        MirExpr::ObjCompare {
            op: pycc_mir::CmpOpKind::Lt,
            left: object(),
            right: object(),
        },
        MirExpr::ObjCompare {
            op: pycc_mir::CmpOpKind::Is,
            left: object(),
            right: object(),
        },
    ];
    for node in &nodes {
        assert!(!expression_can_set_exception(node), "{node:?}");
    }
    // A call of a compiled function still sets pycc's own pending state.
    assert!(expression_can_set_exception(&MirExpr::Call {
        callee: "f".to_string(),
        args: Vec::new(),
        ty: Ty::Int,
    }));
}
