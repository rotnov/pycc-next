use super::*;
use pycc_hir::ndarray::{NdOp, NdTy};
use pycc_mir::{CmpOpKind, MirNdExpr};

fn int(n: i64) -> MirExpr {
    MirExpr::IntLiteral(n)
}

fn name(n: &str) -> MirExpr {
    MirExpr::Name {
        name: n.to_string(),
        ty: Ty::Int,
    }
}

fn bin(op: BinOpKind, l: MirExpr, r: MirExpr) -> MirExpr {
    MirExpr::BinOp {
        op,
        left: Box::new(l),
        right: Box::new(r),
        ty: Ty::Int,
    }
}

fn assign(target: &str, value: MirExpr) -> MirStmt {
    MirStmt::Assign {
        target: target.to_string(),
        value,
    }
}

fn byte_of(buf: &str) -> MirExpr {
    MirExpr::Nd(Box::new(MirNdExpr {
        op: NdOp::Get { chained: false },
        base: MirExpr::Name {
            name: buf.to_string(),
            ty: Ty::Object,
        },
        args: vec![int(0)],
        nd: NdTy::bytearray(),
        cached: false,
    }))
}

fn abs(x: MirExpr) -> MirExpr {
    MirExpr::Call {
        callee: "abs".to_string(),
        args: vec![x],
        ty: Ty::Int,
    }
}

fn ranges(body: &[MirStmt], names: &[&str]) -> IntRanges {
    let names: Vec<String> = names.iter().map(|n| n.to_string()).collect();
    int_ranges(body, names.iter(), true)
}

fn range_of(r: &IntRanges, n: &str) -> Option<(i64, i64)> {
    r.locals.get(n).map(|r| (r.lo, r.hi))
}

#[test]
fn bytes_and_their_arithmetic_have_ranges() {
    let body = vec![
        assign("a", byte_of("buf")),
        assign("b", int(0)),
        assign("b", byte_of("buf")),
        assign(
            "p",
            bin(
                BinOpKind::Sub,
                bin(BinOpKind::Add, name("a"), name("b")),
                int(7),
            ),
        ),
        assign("pa", abs(bin(BinOpKind::Sub, name("p"), name("a")))),
        assign("m", bin(BinOpKind::BitAnd, name("p"), int(0xF0))),
        assign("q", bin(BinOpKind::FloorDiv, name("p"), int(4))),
        assign("r", bin(BinOpKind::Mod, name("p"), int(10))),
        assign("s", bin(BinOpKind::RShift, name("a"), int(3))),
        assign("t", bin(BinOpKind::LShift, name("a"), int(4))),
        assign("u", bin(BinOpKind::BitXor, name("a"), name("b"))),
        assign("v", bin(BinOpKind::Mul, name("q"), name("s"))),
    ];
    let r = ranges(
        &body,
        &["a", "b", "p", "pa", "m", "q", "r", "s", "t", "u", "v"],
    );
    assert_eq!(range_of(&r, "a"), Some((0, 255)));
    assert_eq!(range_of(&r, "b"), Some((0, 255)));
    assert_eq!(range_of(&r, "p"), Some((-7, 503)));
    assert_eq!(range_of(&r, "pa"), Some((0, 503)));
    assert_eq!(range_of(&r, "m"), Some((0, 0xF0)));
    assert_eq!(range_of(&r, "q"), Some((-2, 125)));
    assert_eq!(range_of(&r, "r"), Some((0, 9)));
    assert_eq!(range_of(&r, "s"), Some((0, 31)));
    assert_eq!(range_of(&r, "t"), Some((0, 4080)));
    assert_eq!(range_of(&r, "u"), Some((0, 255)));
    assert_eq!(range_of(&r, "v"), Some((-62, 3875)));
}

#[test]
fn unknown_values_and_growing_locals_have_none() {
    let param = name("n");
    let body = vec![
        // A parameter's arithmetic is unknown, but masking it is a byte.
        assign("x", bin(BinOpKind::Add, param.clone(), int(1))),
        assign("y", bin(BinOpKind::BitAnd, param.clone(), int(255))),
        assign("z", bin(BinOpKind::Mod, param.clone(), int(8))),
        // `i` grows on every trip.
        assign("i", int(0)),
        MirStmt::While {
            test: MirExpr::BoolLiteral(true),
            body: vec![assign("i", bin(BinOpKind::Add, name("i"), int(1)))],
        },
        // A result past the smallint range.
        assign("big", bin(BinOpKind::Mul, int(1 << 40), int(1 << 30))),
        // A `bool` converted to `int` keeps its word.
        assign(
            "flag",
            MirExpr::IntBoundary(Box::new(MirExpr::BoolLiteral(true))),
        ),
        // `**` is not modelled.
        assign("pw", bin(BinOpKind::Pow, int(2), int(3))),
    ];
    let r = ranges(&body, &["x", "y", "z", "i", "big", "flag", "pw"]);
    assert_eq!(range_of(&r, "x"), None);
    assert_eq!(range_of(&r, "y"), Some((0, 255)));
    assert_eq!(range_of(&r, "z"), Some((0, 7)));
    assert_eq!(range_of(&r, "i"), None);
    assert_eq!(range_of(&r, "big"), None);
    assert_eq!(range_of(&r, "flag"), None);
    assert_eq!(range_of(&r, "pw"), None);
}

#[test]
fn a_local_bound_any_other_way_has_none() {
    let body = vec![
        assign("a", int(1)),
        MirStmt::ForList {
            var: "a".to_string(),
            var_ty: Ty::Int,
            list: "xs".to_string(),
            body: vec![],
        },
        assign("b", int(2)),
        MirStmt::ExprStmt(MirExpr::NamedExpr {
            name: "b".to_string(),
            value: Box::new(name("n")),
            ty: Ty::Int,
        }),
        assign("c", int(3)),
    ];
    let r = ranges(&body, &["a", "b", "c", "d"]);
    assert_eq!(range_of(&r, "a"), None);
    assert_eq!(range_of(&r, "b"), None);
    assert_eq!(range_of(&r, "c"), Some((3, 3)));
    // Never bound at all.
    assert_eq!(range_of(&r, "d"), None);
}

#[test]
fn range_loops_bound_their_variable() {
    let up = MirStmt::ForRange {
        var: "i".to_string(),
        start: int(2),
        stop: int(10),
        step: int(3),
        body: vec![assign("j", bin(BinOpKind::Mul, name("i"), name("i")))],
    };
    let down = MirStmt::ForRange {
        var: "k".to_string(),
        start: int(9),
        stop: int(-1),
        step: int(-1),
        body: vec![],
    };
    let any = MirStmt::ForRange {
        var: "m".to_string(),
        start: int(-4),
        stop: int(4),
        step: name("s"),
        body: vec![],
    };
    let unknown = MirStmt::ForRange {
        var: "u".to_string(),
        start: int(0),
        stop: name("n"),
        step: int(1),
        body: vec![],
    };
    let r = ranges(&[up, down, any, unknown], &["i", "j", "k", "m", "u"]);
    assert_eq!(range_of(&r, "i"), Some((2, 9)));
    assert_eq!(range_of(&r, "j"), Some((4, 81)));
    assert_eq!(range_of(&r, "k"), Some((0, 9)));
    assert_eq!(range_of(&r, "m"), Some((-4, 4)));
    assert_eq!(range_of(&r, "u"), None);
}

#[test]
fn a_user_abs_and_conditional_expressions() {
    let pick = MirExpr::IfExp {
        test: Box::new(MirExpr::Compare {
            op: CmpOpKind::Lt,
            left: Box::new(name("a")),
            right: Box::new(int(3)),
            ty: Ty::Bool,
        }),
        body: Box::new(int(-5)),
        orelse: Box::new(name("a")),
        ty: Ty::Int,
    };
    let body = vec![
        assign("a", byte_of("buf")),
        assign("p", pick),
        assign("q", abs(bin(BinOpKind::Sub, int(-10), name("a")))),
    ];
    let names = ["a".to_string(), "p".to_string(), "q".to_string()];
    let r = int_ranges(&body, names.iter(), true);
    assert_eq!(range_of(&r, "p"), Some((-5, 255)));
    assert_eq!(range_of(&r, "q"), Some((10, 265)));
    assert!(r.is_small_local("a") && !r.is_small_local("zz"));
    // A module function named `abs` is not the builtin.
    let r = int_ranges(&body, names.iter(), false);
    assert_eq!(range_of(&r, "q"), None);
}

#[test]
fn interval_arithmetic_edges() {
    let r = |lo, hi| Range { lo, hi };
    assert_eq!(binop_range(BinOpKind::BitAnd, r(-3, 5), r(-1, 0)), None);
    assert_eq!(
        binop_range(BinOpKind::BitOr, r(0, 0), r(0, 0)),
        Some(r(0, 0))
    );
    assert_eq!(binop_range(BinOpKind::BitOr, r(-1, 0), r(0, 0)), None);
    assert_eq!(binop_range(BinOpKind::RShift, r(-9, 9), r(0, 200)), None);
    assert_eq!(
        binop_range(BinOpKind::RShift, r(-9, 9), r(1, 100)),
        Some(r(-5, 4))
    );
    assert_eq!(
        binop_range(BinOpKind::LShift, r(1, 1), r(61, 61)),
        Some(r(1 << 61, 1 << 61))
    );
    assert_eq!(binop_range(BinOpKind::LShift, r(1, 1), r(62, 62)), None);
    assert_eq!(binop_range(BinOpKind::FloorDiv, r(-7, 7), r(0, 2)), None);
    assert_eq!(
        binop_range(BinOpKind::FloorDiv, r(-7, 7), r(2, 3)),
        Some(r(-4, 3))
    );
    assert_eq!(binop_range(BinOpKind::Mod, r(1, 4), r(5, 5)), Some(r(1, 4)));
    assert_eq!(
        binop_range(BinOpKind::Mod, r(-1, 4), r(5, 6)),
        Some(r(0, 5))
    );
    assert_eq!(binop_range(BinOpKind::Mod, r(1, 4), r(-5, 5)), None);
    assert_eq!(binop_range(BinOpKind::Div, r(1, 4), r(1, 5)), None);
    assert_eq!(floor_div(-7, 2), -4);
    assert_eq!(floor_div(7, 2), 3);
}
