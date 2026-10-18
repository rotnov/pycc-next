//! `while s[i] in chars: i += 1` as one runtime scan.
//!
//! The loop tests one character per trip: it reads `s[i]`, looks it up in
//! a set of `str` and increments `i`. When its body is exactly that
//! increment, the whole loop is `pycc_rt_str_skip_in_set`, which walks the
//! text itself (a bit test per character for ASCII text and a frozenset)
//! and returns the index the loop would stop at. A loop that stops on the
//! `IndexError` of reading past the end stops there too, with the index
//! already stored, and the statement that follows the scan checks `s[i]`,
//! raising that same `IndexError` (and doing nothing when the scan stopped
//! on a character). `while s[i] not in chars` is the same scan negated.

use crate::{MirExpr, MirItem, MirModule, MirStmt, RtCallExpr, Ty};
use pycc_hir::builtins::BuiltinSig;
use pycc_hir::{BinOpKind, CmpOpKind};

const GETITEM: &str = "pycc_rt_str_getitem";
const SCAN: &str = "pycc_rt_str_skip_in_set";
/// Raises the `IndexError` of `s[i]`, if any, with no character made.
const CHECK: &str = "pycc_rt_str_check_index";

/// Rewrites every recognized scan loop of `module`.
pub(crate) fn rewrite(module: &mut MirModule) {
    for item in &mut module.items {
        match item {
            MirItem::Function { body, .. } => rewrite_body(body),
            MirItem::TopLevelStmt(stmt) => {
                for body in stmt.bodies_mut() {
                    rewrite_body(body);
                }
            }
            MirItem::ForeignImport { .. } => {}
        }
    }
}

fn rewrite_body(body: &mut Vec<MirStmt>) {
    let mut at = 0;
    while at < body.len() {
        for inner in body[at].bodies_mut() {
            rewrite_body(inner);
        }
        if let Some(scan) = recognize(&body[at]) {
            body.splice(at..=at, scan);
            at += 2;
        } else {
            at += 1;
        }
    }
}

/// The two statements replacing `stmt` when it is a scan loop.
fn recognize(stmt: &MirStmt) -> Option<[MirStmt; 2]> {
    let MirStmt::While { test, body } = stmt else {
        return None;
    };
    let MirExpr::Compare {
        op: op @ (CmpOpKind::In | CmpOpKind::NotIn),
        left,
        right,
        ..
    } = test
    else {
        return None;
    };
    let MirExpr::RtCall(read) = left.as_ref() else {
        return None;
    };
    let [text @ MirExpr::Name { ty: Ty::Str, .. }, MirExpr::Name { name: index, ty: Ty::Int }] =
        read.args.as_slice()
    else {
        return None;
    };
    let is_text_set = matches!(
        right.ty(),
        Ty::Set(element) | Ty::FrozenSet(element) if *element == Ty::Str
    );
    if read.sig.symbol != GETITEM || !is_text_set || !matches!(right.as_ref(), MirExpr::Name { .. })
    {
        return None;
    }
    // Position markers (`MirStmt::Pos`) say nothing about the loop's shape.
    let body: Vec<&MirStmt> = body.iter().filter(|stmt| !matches!(stmt, MirStmt::Pos(_))).collect();
    let [
        MirStmt::Assign {
            target,
            value:
                MirExpr::BinOp {
                    op: BinOpKind::Add,
                    left: counter,
                    right: step,
                    ..
                },
        },
    ] = body.as_slice()
    else {
        return None;
    };
    let increments = target == index
        && matches!(counter.as_ref(), MirExpr::Name { name, .. } if name == index)
        && matches!(step.as_ref(), MirExpr::IntLiteral(1));
    let names = [text, right.as_ref()];
    if !increments
        || names
            .iter()
            .any(|e| matches!(e, MirExpr::Name { name, .. } if name == index))
    {
        return None;
    }
    let index_read = MirExpr::Name {
        name: index.clone(),
        ty: Ty::Int,
    };
    let scan_args = vec![
        text.clone(),
        index_read.clone(),
        (**right).clone(),
        MirExpr::BoolLiteral(*op == CmpOpKind::NotIn),
    ];
    let scan = MirExpr::RtCall(Box::new(RtCallExpr {
        sig: BuiltinSig {
            symbol: SCAN,
            params: scan_args.iter().map(MirExpr::ty).collect(),
            ret: Ty::Int,
            ret_word: false,
            word_args: vec![false; scan_args.len()],
            kind_args: Vec::new(),
            desc_args: Vec::new(),
        },
        args: scan_args,
    }));
    let check_args = vec![text.clone(), index_read];
    let reread = MirExpr::RtCall(Box::new(RtCallExpr {
        sig: BuiltinSig {
            symbol: CHECK,
            params: check_args.iter().map(MirExpr::ty).collect(),
            ret: Ty::Bool,
            ret_word: false,
            word_args: vec![false; check_args.len()],
            kind_args: Vec::new(),
            desc_args: Vec::new(),
        },
        args: check_args,
    }));
    Some([
        MirStmt::Assign {
            target: index.clone(),
            value: scan,
        },
        MirStmt::ExprStmt(reread),
    ])
}
