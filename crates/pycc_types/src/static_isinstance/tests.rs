use super::*;

fn lowered(source: &str) -> HirModule {
    let module = pycc_parser::parse(source).expect("test fixture must parse");
    pycc_hir::lower_checked(&module).expect("test fixture must lower")
}

fn list(inner: Ty) -> Ty {
    Ty::List(Box::new(inner))
}

#[test]
fn a_builtin_type_decides_its_class_tests() {
    let ints = list(Ty::Int);
    assert_eq!(instance_of(&ints, "list", false), Some(true));
    // A deque is typed `list[T]`: once the module builds one, `list` is open.
    assert_eq!(instance_of(&ints, "list", true), None);
    assert_eq!(instance_of(&ints, "tuple", true), Some(false));
    assert_eq!(instance_of(&Ty::Str, "str", false), Some(true));
    assert_eq!(instance_of(&Ty::Str, "list", false), Some(false));
    assert_eq!(
        instance_of(&Ty::VarTuple(Box::new(Ty::Int)), "tuple", false),
        Some(true)
    );
    assert_eq!(
        instance_of(&Ty::Dict(Box::new((Ty::Str, Ty::Int))), "dict", false),
        Some(true)
    );
    assert_eq!(
        instance_of(&Ty::Set(Box::new(Ty::Int)), "frozenset", false),
        Some(false)
    );
    assert_eq!(
        instance_of(&Ty::FrozenSet(Box::new(Ty::Int)), "frozenset", false),
        Some(true)
    );
    assert_eq!(instance_of(&Ty::None, "int", false), Some(false));
    // An `int` slot may hold a `bool`; a `bool` is an `int`.
    assert_eq!(instance_of(&Ty::Int, "int", false), Some(true));
    assert_eq!(instance_of(&Ty::Int, "bool", false), None);
    assert_eq!(instance_of(&Ty::Bool, "int", false), Some(true));
    assert_eq!(instance_of(&Ty::Bool, "str", false), Some(false));
    // A number may hold either kind.
    assert_eq!(instance_of(&Ty::Float, "float", false), None);
    assert_eq!(instance_of(&Ty::Num, "list", false), Some(false));
    let optional = Ty::Optional(Box::new(Ty::Str));
    assert_eq!(instance_of(&optional, "list", false), Some(false));
    assert_eq!(instance_of(&optional, "str", false), None);
    assert_eq!(instance_of(&Ty::Object, "list", false), None);
}

#[test]
fn and_or_and_not_combine_with_short_circuits() {
    let name = |n: &str| HirExpr::Name(n.to_string());
    let test = |subject: &str, class: &str| HirExpr::Call {
        callee: "isinstance".to_string(),
        args: vec![name(subject), name(class)],
    };
    let not = |operand: HirExpr| HirExpr::UnaryOp {
        op: UnaryOpKind::Not,
        operand: Box::new(operand),
    };
    let bool_op = |op, left, right| HirExpr::BoolOp {
        op,
        left: Box::new(left),
        right: Box::new(right),
        truth_only: true,
    };
    let type_of = |expr: &HirExpr| match expr {
        HirExpr::Name(n) if n == "xs" => Some(list(Ty::Int)),
        HirExpr::Name(n) if n == "s" => Some(Ty::Str),
        _ => None,
    };
    // `not isinstance(xs, list) and isinstance(o, int)`: false at once, and
    // `o` is never evaluated, so its open type does not matter.
    let and = bool_op(BoolOpKind::And, not(test("xs", "list")), test("o", "int"));
    assert_eq!(
        decide(&and, &type_of, false),
        Some((false, vec![name("xs")]))
    );
    // `isinstance(s, str) or ...` is true at once.
    let or = bool_op(BoolOpKind::Or, test("s", "str"), test("o", "int"));
    assert_eq!(decide(&or, &type_of, false), Some((true, vec![name("s")])));
    // An evaluated subject of open type leaves the test undecided.
    let open = bool_op(BoolOpKind::Or, test("s", "list"), test("o", "int"));
    assert_eq!(decide(&open, &type_of, false), None);
    // A tuple of classes is true when any is.
    let tuple = HirExpr::Call {
        callee: "isinstance".to_string(),
        args: vec![
            name("s"),
            HirExpr::TupleLiteral(vec![name("list"), name("str")]),
        ],
    };
    assert_eq!(
        decide(&tuple, &type_of, false),
        Some((true, vec![name("s")]))
    );
}

const NORMALIZE: &str = "\
def _rows(xs: list[list[int]]) -> int:
    if not isinstance(xs[0], list):
        return -1
    return len(xs[0])


print(_rows([[1, 2]]))
";

#[test]
fn a_decided_guard_keeps_its_branch_and_evaluates_its_subject() {
    let (folded, folds) = fold(&lowered(NORMALIZE)).expect("the guard folds");
    assert_eq!(folds.len(), 1);
    assert!(!folds[0].value);
    let HirItem::Function { body, .. } = &folded.items[0] else {
        panic!("the function stays first");
    };
    // `xs[0]` still runs (it may raise `IndexError`); the `return -1`
    // branch is gone.
    assert!(matches!(
        &body[0],
        HirStmt::ExprStmt(HirExpr::Subscript { .. })
    ));
    assert_eq!(body.len(), 2);
    assert!(matches!(
        &body[1],
        HirStmt::Return(Some(HirExpr::Call { .. }))
    ));
    // The checker's signatures confirm the fold.
    let mut signatures = crate::module::FunctionSignatures::default();
    signatures.insert("_rows".to_string(), (vec![list(list(Ty::Int))], Ty::Int));
    assert!(verify(&folded, &signatures, &folds).is_ok());
    // Signatures under which the test would go the other way refuse it.
    signatures.insert("_rows".to_string(), (vec![list(Ty::Int)], Ty::Int));
    let refused = verify(&folded, &signatures, &folds).unwrap_err();
    assert_eq!(refused.code, "C0001");
    assert!(refused.message.contains("`_rows`"), "{}", refused.message);
}

#[test]
fn a_shadowed_builtin_or_an_undecidable_subject_folds_nothing() {
    let shadowed = NORMALIZE.replace("print(", "list = tuple\nprint(");
    assert!(fold(&lowered(&shadowed)).is_none());
    let object = "\
def _rows(xs):
    if not isinstance(xs[0], C):
        return -1
    return 0


class C:
    pass
";
    assert!(fold(&lowered(object)).is_none());
}
