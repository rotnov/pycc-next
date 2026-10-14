//! `match` statement lowering.

use super::*;

// -- #381: match statement lowering coverage ---------------------------

fn lower_match_stmt(source: &str) -> HirStmt {
    let module = pycc_parser_test_helper::parse(source);
    let hir = lower_checked(&module).expect("match fixture must lower");
    hir.items
        .into_iter()
        .find_map(|item| match item {
            HirItem::TopLevelStmt(s) if matches!(s, HirStmt::Match { .. }) => Some(s),
            _ => None,
        })
        .expect("expected a top-level Match statement")
}

fn lower_match_in_function(source: &str) -> Vec<HirStmt> {
    let module = pycc_parser_test_helper::parse(source);
    let hir = lower_checked(&module).expect("match fixture must lower");
    hir.items
        .into_iter()
        .find_map(|item| match item {
            HirItem::Function { body, .. } => Some(body),
            HirItem::TopLevelStmt(_) => None,
        })
        .expect("expected a function")
}

fn match_cases(stmt: &HirStmt) -> &[HirMatchCase] {
    match stmt {
        HirStmt::Match { cases, .. } => cases,
        _ => panic!("expected HirStmt::Match"),
    }
}

#[test]
#[should_panic(expected = "expected HirStmt::Match")]
fn match_cases_panics_on_non_match() {
    match_cases(&HirStmt::ExprStmt(HirExpr::IntLiteral(0)));
}

#[test]
fn lowers_match_with_int_literal_pattern() {
    let stmt =
        lower_match_stmt("x = 1\nmatch x:\n    case 1:\n        pass\n    case _:\n        pass\n");
    assert_eq!(
        stmt,
        HirStmt::Match {
            subject: HirExpr::Name("x".to_string()),
            cases: vec![
                HirMatchCase {
                    pattern: HirPattern::Literal(HirExpr::IntLiteral(1)),
                    guard: None,
                    body: vec![],
                },
                HirMatchCase {
                    pattern: HirPattern::Wildcard,
                    guard: None,
                    body: vec![],
                },
            ],
        }
    );
}

#[test]
fn lowers_match_with_float_literal_pattern() {
    let stmt = lower_match_stmt(
        "x = 1.0\nmatch x:\n    case 2.5:\n        pass\n    case _:\n        pass\n",
    );
    let cases = match_cases(&stmt);
    assert_eq!(cases.len(), 2);
    assert_eq!(
        cases[0].pattern,
        HirPattern::Literal(HirExpr::FloatLiteral(2.5))
    );
}

#[test]
fn lowers_match_with_string_literal_pattern() {
    let stmt = lower_match_stmt(
        "x = \"hi\"\nmatch x:\n    case \"hi\":\n        pass\n    case _:\n        pass\n",
    );
    let cases = match_cases(&stmt);
    assert_eq!(cases.len(), 2);
    assert_eq!(
        cases[0].pattern,
        HirPattern::Literal(HirExpr::StringLiteral("hi".to_string()))
    );
}

#[test]
fn lowers_match_with_singleton_true_pattern() {
    let stmt = lower_match_stmt(
        "x = True\nmatch x:\n    case True:\n        pass\n    case _:\n        pass\n",
    );
    let cases = match_cases(&stmt);
    assert_eq!(cases.len(), 2);
    assert_eq!(cases[0].pattern, HirPattern::Singleton(true));
}

#[test]
fn lowers_match_with_singleton_false_pattern() {
    let stmt = lower_match_stmt(
        "x = False\nmatch x:\n    case False:\n        pass\n    case _:\n        pass\n",
    );
    let cases = match_cases(&stmt);
    assert_eq!(cases.len(), 2);
    assert_eq!(cases[0].pattern, HirPattern::Singleton(false));
}

#[test]
fn lowers_match_with_none_singleton_pattern() {
    let stmt = lower_match_stmt(
        "def f() -> None:\n    pass\nmatch f():\n    case None:\n        pass\n    case _:\n        pass\n",
    );
    let cases = match_cases(&stmt);
    assert_eq!(cases.len(), 2);
    assert_eq!(cases[0].pattern, HirPattern::NoneSingleton);
}

#[test]
fn lowers_match_with_capture_pattern() {
    let stmt = lower_match_stmt("x = 42\nmatch x:\n    case y:\n        pass\n");
    let cases = match_cases(&stmt);
    assert_eq!(cases.len(), 1);
    assert_eq!(cases[0].pattern, HirPattern::Capture("y".to_string()));
}

#[test]
fn lowers_match_with_wildcard_pattern() {
    let stmt = lower_match_stmt("x = 1\nmatch x:\n    case _:\n        pass\n");
    let cases = match_cases(&stmt);
    assert_eq!(cases.len(), 1);
    assert_eq!(cases[0].pattern, HirPattern::Wildcard);
}

#[test]
fn lowers_match_with_sequence_pattern() {
    let stmt = lower_match_stmt(
        "x = [1, 2]\nmatch x:\n    case [a, b]:\n        pass\n    case _:\n        pass\n",
    );
    let cases = match_cases(&stmt);
    assert_eq!(cases.len(), 2);
    assert_eq!(
        cases[0].pattern,
        HirPattern::Sequence(vec![
            HirPattern::Capture("a".to_string()),
            HirPattern::Capture("b".to_string()),
        ])
    );
}

#[test]
fn lowers_match_with_sequence_star_pattern() {
    let stmt = lower_match_stmt(
        "x = [1, 2]\nmatch x:\n    case [a, *rest]:\n        pass\n    case _:\n        pass\n",
    );
    let cases = match_cases(&stmt);
    assert_eq!(cases.len(), 2);
    assert_eq!(
        cases[0].pattern,
        HirPattern::SequenceStar(
            vec![HirPattern::Capture("a".to_string())],
            Some("rest".to_string()),
        )
    );
}

#[test]
fn lowers_match_with_sequence_star_wildcard_pattern() {
    let stmt = lower_match_stmt(
        "x = [1, 2]\nmatch x:\n    case [a, *_]:\n        pass\n    case _:\n        pass\n",
    );
    let cases = match_cases(&stmt);
    assert_eq!(cases.len(), 2);
    assert_eq!(
        cases[0].pattern,
        HirPattern::SequenceStar(vec![HirPattern::Capture("a".to_string())], None,)
    );
}

#[test]
fn lowers_match_with_mapping_pattern() {
    let stmt = lower_match_stmt(
        "x = {\"k\": 1}\nmatch x:\n    case {\"k\": v}:\n        pass\n    case _:\n        pass\n",
    );
    let cases = match_cases(&stmt);
    assert_eq!(cases.len(), 2);
    assert_eq!(
        cases[0].pattern,
        HirPattern::Mapping(
            vec![(
                HirExpr::StringLiteral("k".to_string()),
                HirPattern::Capture("v".to_string())
            )],
            None,
        )
    );
}

#[test]
fn lowers_match_with_mapping_rest_pattern() {
    let stmt = lower_match_stmt(
        "x = {\"k\": 1}\nmatch x:\n    case {\"k\": v, **rest}:\n        pass\n    case _:\n        pass\n",
    );
    let cases = match_cases(&stmt);
    assert_eq!(cases.len(), 2);
    assert_eq!(
        cases[0].pattern,
        HirPattern::Mapping(
            vec![(
                HirExpr::StringLiteral("k".to_string()),
                HirPattern::Capture("v".to_string())
            )],
            Some("rest".to_string()),
        )
    );
}

#[test]
fn lowers_match_with_class_pattern() {
    let stmt = lower_match_stmt(
        "class P:\n    def __init__(self):\n        pass\nx = P()\nmatch x:\n    case P():\n        pass\n    case _:\n        pass\n",
    );
    let cases = match_cases(&stmt);
    assert_eq!(cases.len(), 2);
    assert_eq!(
        cases[0].pattern,
        HirPattern::Class {
            class_name: "P".to_string(),
            positional: vec![],
            keyword: vec![],
        }
    );
}

#[test]
fn lowers_match_with_class_positional_pattern() {
    let stmt = lower_match_stmt(
        "class P:\n    def __init__(self, a: int):\n        self.a = a\nx = P(1)\nmatch x:\n    case P(1):\n        pass\n    case _:\n        pass\n",
    );
    let cases = match_cases(&stmt);
    assert_eq!(cases.len(), 2);
    assert_eq!(
        cases[0].pattern,
        HirPattern::Class {
            class_name: "P".to_string(),
            positional: vec![HirPattern::Literal(HirExpr::IntLiteral(1))],
            keyword: vec![],
        }
    );
}

#[test]
fn lowers_match_with_class_keyword_pattern() {
    let stmt = lower_match_stmt(
        "class P:\n    def __init__(self):\n        self.a = 1\nx = P()\nmatch x:\n    case P(a=1):\n        pass\n    case _:\n        pass\n",
    );
    let cases = match_cases(&stmt);
    assert_eq!(cases.len(), 2);
    assert_eq!(
        cases[0].pattern,
        HirPattern::Class {
            class_name: "P".to_string(),
            positional: vec![],
            keyword: vec![("a".to_string(), HirPattern::Literal(HirExpr::IntLiteral(1)))],
        }
    );
}

#[test]
fn lowers_match_with_or_pattern() {
    let stmt = lower_match_stmt(
        "x = 2\nmatch x:\n    case 1 | 2 | 3:\n        pass\n    case _:\n        pass\n",
    );
    let cases = match_cases(&stmt);
    assert_eq!(cases.len(), 2);
    assert_eq!(
        cases[0].pattern,
        HirPattern::Or(vec![
            HirPattern::Literal(HirExpr::IntLiteral(1)),
            HirPattern::Literal(HirExpr::IntLiteral(2)),
            HirPattern::Literal(HirExpr::IntLiteral(3)),
        ])
    );
}

#[test]
fn lowers_match_with_as_pattern() {
    let stmt = lower_match_stmt(
        "x = [1, 2]\nmatch x:\n    case [a, b] as pair:\n        pass\n    case _:\n        pass\n",
    );
    let cases = match_cases(&stmt);
    assert_eq!(cases.len(), 2);
    assert_eq!(
        cases[0].pattern,
        HirPattern::As(
            Box::new(HirPattern::Sequence(vec![
                HirPattern::Capture("a".to_string()),
                HirPattern::Capture("b".to_string()),
            ])),
            "pair".to_string(),
        )
    );
}

#[test]
fn lowers_match_with_guard() {
    let stmt = lower_match_stmt(
        "x = 5\nmatch x:\n    case y if y > 3:\n        pass\n    case _:\n        pass\n",
    );
    let cases = match_cases(&stmt);
    assert_eq!(cases.len(), 2);
    assert!(cases[0].guard.is_some());
    assert!(cases[1].guard.is_none());
}

#[test]
fn lowers_match_inside_function() {
    let body = lower_match_in_function(
        "x = 1\ndef f(x: int) -> int:\n    match x:\n        case 1:\n            return 10\n        case _:\n            return 0\n",
    );
    assert!(body.iter().any(|s| matches!(s, HirStmt::Match { .. })));
}

#[test]
fn lowers_match_with_pass_only_body() {
    let stmt =
        lower_match_stmt("x = 1\nmatch x:\n    case 1:\n        pass\n    case _:\n        pass\n");
    let cases = match_cases(&stmt);
    assert!(cases[0].body.is_empty());
    assert!(cases[1].body.is_empty());
}

#[test]
fn lowers_match_with_non_literal_value_pattern_is_c0001() {
    let module = pycc_parser_test_helper::parse(
        "x = 1\nmatch x:\n    case foo.bar:\n        pass\n    case _:\n        pass\n",
    );
    let err = lower_checked(&module).unwrap_err();
    assert_eq!(err.code, "C0001");
    assert!(
        err.message
            .contains("only a literal value pattern")
    );
}

#[test]
fn lowers_match_with_non_name_class_pattern_is_c0001() {
    let module = pycc_parser_test_helper::parse(
        "x = 1\nmatch x:\n    case int.foo():\n        pass\n    case _:\n        pass\n",
    );
    let err = lower_checked(&module).unwrap_err();
    assert_eq!(err.code, "C0001");
    assert!(
        err.message
            .contains("only a bare-name class pattern is supported so far")
    );
}

#[test]
fn lowers_match_with_nested_match() {
    let stmt = lower_match_stmt(
        "x = 1\ny = 2\nmatch x:\n    case 1:\n        match y:\n            case 2:\n                pass\n            case _:\n                pass\n    case _:\n        pass\n",
    );
    let cases = match_cases(&stmt);
    assert_eq!(cases.len(), 2);
    let inner_cases = match_cases(&cases[0].body[0]);
    assert_eq!(inner_cases.len(), 2);
}
