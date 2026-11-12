//! [`natively_used`] on hand-built copies: tomli's scanner, whose `str`
//! meets only objects, a loop indexing it with a native `int`, and
//! pypng's filters, whose loops use a local computed from the parameter.

use super::natively_used;
use crate::{BinOpKind, CmpOpKind, IndexUse, MirExpr, MirItem, MirModule, MirStmt, Ty};

/// A list of `str`s: `pycc_mir` lowers a `str`'s index to a runtime call
/// rather than a `Subscript`, which these hand-built copies stand in for.
fn strs() -> Ty {
    Ty::List(Box::new(Ty::Str))
}

fn name(name: &str, ty: Ty) -> MirExpr {
    MirExpr::Name {
        name: name.to_string(),
        ty,
    }
}

fn module(params: Vec<(&str, Ty)>, body: Vec<MirStmt>) -> MirModule {
    MirModule {
        items: vec![MirItem::Function {
            name: "_pycc_native_f".to_string(),
            params: params
                .into_iter()
                .map(|(p, ty)| (p.to_string(), ty))
                .collect(),
            return_ty: Ty::Object,
            body,
        }],
        class_defs: Vec::new(),
    }
}

fn used(module: &MirModule, params: &[&str]) -> Vec<String> {
    let params: Vec<String> = params.iter().map(|p| p.to_string()).collect();
    natively_used(module, "_pycc_native_f", &params)
        .into_iter()
        .collect()
}

/// `src[i]` for a native `i`, compared with a literal.
fn native_index_test() -> MirExpr {
    MirExpr::Compare {
        op: CmpOpKind::Eq,
        left: Box::new(MirExpr::Subscript {
            base: Box::new(name("src", strs())),
            index: Box::new(name("i", Ty::Int)),
        }),
        right: Box::new(MirExpr::StringLiteral(" ".to_string())),
        ty: Ty::Bool,
    }
}

/// `while src[pos] in chars:` with `pos` and `chars` objects: the index is
/// converted and the character packed at every step.
#[test]
fn a_str_meeting_only_objects_in_its_loop_is_not_used_natively() {
    let test = MirExpr::ObjContains {
        negate: false,
        item: Box::new(MirExpr::Subscript {
            base: Box::new(name("src", strs())),
            index: Box::new(MirExpr::ObjectIndex(
                Box::new(name("pos", Ty::Object)),
                IndexUse::List,
            )),
        }),
        container: Box::new(name("chars", Ty::Object)),
    };
    let m = module(
        vec![("src", strs()), ("pos", Ty::Object), ("chars", Ty::Object)],
        vec![MirStmt::While {
            test,
            body: Vec::new(),
        }],
    );
    assert!(used(&m, &["src"]).is_empty());
}

/// `src[i] == " "` with a native `i` inside a loop uses `src` natively; the
/// same comparison outside a loop runs once per call, as the conversion
/// does, and does not count.
#[test]
fn a_str_indexed_natively_in_a_loop_is_used() {
    let inside = module(
        vec![("src", strs())],
        vec![MirStmt::While {
            test: MirExpr::BoolLiteral(true),
            body: vec![MirStmt::ExprStmt(native_index_test()), MirStmt::Break],
        }],
    );
    assert_eq!(used(&inside, &["src"]), ["src"]);
    let outside = module(
        vec![("src", strs())],
        vec![MirStmt::ExprStmt(native_index_test())],
    );
    assert!(used(&outside, &["src"]).is_empty());
}

/// A parameter the copy declares an object, a function the module does not
/// have, and a `for` that walks the parameter.
#[test]
fn only_native_parameters_of_the_named_copy_count() {
    let m = module(
        vec![("src", Ty::Object)],
        vec![MirStmt::While {
            test: native_index_test(),
            body: Vec::new(),
        }],
    );
    assert!(used(&m, &["src"]).is_empty());
    assert!(natively_used(&m, "_pycc_native_g", &["src".to_string()]).is_empty());
    let walked = module(
        vec![("xs", Ty::List(Box::new(Ty::Int)))],
        vec![MirStmt::ForList {
            var: "x".to_string(),
            var_ty: Ty::Int,
            list: "xs".to_string(),
            body: Vec::new(),
        }],
    );
    assert_eq!(used(&walked, &["xs"]), ["xs"]);
}

/// `ai = 0 - filter_unit` before the loop, then `result[ai]` in it: the
/// parameter is used through the native local it computes, transitively.
#[test]
fn a_native_local_computed_from_a_parameter_carries_its_use() {
    let assign = |target: &str, from: &str| MirStmt::Assign {
        target: target.to_string(),
        value: MirExpr::BinOp {
            op: BinOpKind::Sub,
            left: Box::new(MirExpr::IntLiteral(0)),
            right: Box::new(name(from, Ty::Int)),
            ty: Ty::Int,
        },
    };
    let read = MirStmt::ExprStmt(MirExpr::Subscript {
        base: Box::new(name("result", Ty::List(Box::new(Ty::Int)))),
        index: Box::new(name("ai", Ty::Int)),
    });
    let body = |stmts: Vec<MirStmt>| {
        let mut body = stmts;
        body.push(MirStmt::While {
            test: MirExpr::BoolLiteral(true),
            body: vec![read.clone(), MirStmt::Break],
        });
        body
    };
    let params = vec![("unit", Ty::Int), ("result", Ty::List(Box::new(Ty::Int)))];
    let direct = module(params.clone(), body(vec![assign("ai", "unit")]));
    assert_eq!(used(&direct, &["unit"]), ["unit"]);
    let through = module(
        params.clone(),
        body(vec![assign("t", "unit"), assign("ai", "t")]),
    );
    assert_eq!(used(&through, &["unit"]), ["unit"]);
    // A local the loop does not compute with carries nothing.
    let unrelated = module(
        params,
        body(vec![assign("other", "unit"), assign("ai", "ai")]),
    );
    assert!(used(&unrelated, &["unit"]).is_empty());
}
