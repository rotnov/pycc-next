//! Which compiled functions of an `--ext` module run with a live CPython
//! frame (`frames`): the ones that can run CPython code, which could look
//! at the frame stack (`sys._getframe`, `inspect`, `warnings`, a
//! traceback) while they are on it.
//!
//! A function needs no frame when it only computes natively: every
//! operation it performs is native, and every function it calls is
//! another such function. That is the greatest fixed point over the direct
//! calls, so recursion qualifies. Anything that can reach CPython -- an
//! object operation, a closure or generator call, a foreign import, a call
//! of a function outside the module -- needs one. A frameless function
//! still appears in a traceback: its entry is built when the exception
//! reaches CPython (`frames.c`).

use pycc_mir::{MirExpr, MirItem, MirModule, MirStmt};
use std::collections::{HashMap, HashSet};

/// The names of the functions of `mir` that need a live frame.
pub(crate) fn framed_functions(mir: &MirModule) -> HashSet<String> {
    let bodies: HashMap<&str, &[MirStmt]> = mir
        .items
        .iter()
        .filter_map(|item| match item {
            MirItem::Function {
                name,
                body,
                return_ty,
                ..
            } if !matches!(return_ty, pycc_mir::Ty::Generator(_)) => {
                Some((name.as_str(), body.as_slice()))
            }
            _ => None,
        })
        .collect();
    let mut frameless: HashSet<&str> = bodies.keys().copied().collect();
    loop {
        let before = frameless.len();
        let current = frameless.clone();
        frameless.retain(|name| !body_reaches_cpython(bodies[name], &current));
        if frameless.len() == before {
            break;
        }
    }
    mir.items
        .iter()
        .filter_map(|item| match item {
            MirItem::Function { name, .. } if !frameless.contains(name.as_str()) => {
                Some(name.clone())
            }
            _ => None,
        })
        .collect()
}

fn body_reaches_cpython(body: &[MirStmt], frameless: &HashSet<&str>) -> bool {
    body.iter()
        .any(|stmt| stmt_reaches_cpython(stmt, frameless))
}

fn stmt_reaches_cpython(stmt: &MirStmt, frameless: &HashSet<&str>) -> bool {
    matches!(
        stmt,
        MirStmt::ObjDelSlice { .. }
            | MirStmt::ObjSetSlice { .. }
            | MirStmt::ObjAttrSet { .. }
            | MirStmt::ObjDelAttr { .. }
            | MirStmt::ObjSetItem { .. }
            | MirStmt::ObjDelItem { .. }
            | MirStmt::ObjRaise { .. }
            | MirStmt::ForObject { .. }
            | MirStmt::ForGen { .. }
            | MirStmt::ForeignImport { .. }
            | MirStmt::Yield(_)
            | MirStmt::GenReturn(_)
            | MirStmt::ListCompAssign {
                source: pycc_mir::CompSource::Object(_),
                ..
            }
            | MirStmt::DictCompAssign {
                source: pycc_mir::CompSource::Object(_),
                ..
            }
            | MirStmt::SetCompAssign {
                source: pycc_mir::CompSource::Object(_),
                ..
            }
    ) || stmt
        .exprs()
        .into_iter()
        .any(|expr| expr_reaches_cpython(expr, frameless))
        || stmt
            .bodies()
            .into_iter()
            .any(|body| body_reaches_cpython(body, frameless))
}

fn expr_reaches_cpython(expr: &MirExpr, frameless: &HashSet<&str>) -> bool {
    let own = match expr {
        MirExpr::Call { callee, .. } => !frameless.contains(callee.as_str()),
        MirExpr::Instantiate(inst) => !frameless.contains(inst.ctor.as_str()),
        MirExpr::RtCall(call) => call.sig.symbol.starts_with("pycc_ext_"),
        MirExpr::Comprehension(comp) => matches!(comp.source, pycc_mir::CompSource::Object(_)),
        MirExpr::ObjAttrGet { .. }
        | MirExpr::ObjMethodCall { .. }
        | MirExpr::ObjCall { .. }
        | MirExpr::ObjKeywordCall(_)
        | MirExpr::ObjLen { .. }
        | MirExpr::ObjSubscript { .. }
        | MirExpr::ObjCompare { .. }
        | MirExpr::ObjBinOp { .. }
        | MirExpr::ObjUnaryOp { .. }
        | MirExpr::ObjContains { .. }
        | MirExpr::ObjType { .. }
        | MirExpr::ObjSlice { .. }
        | MirExpr::ObjList { .. }
        | MirExpr::ObjDisplay { .. }
        | MirExpr::ObjIsInstance { .. }
        | MirExpr::ObjUnpack { .. }
        | MirExpr::ObjUnpackFloatTuple { .. }
        | MirExpr::ObjectUnbox(..)
        | MirExpr::ObjectCoerce(..)
        | MirExpr::ObjectIndex(..)
        | MirExpr::CallClosure { .. }
        | MirExpr::GenNext { .. }
        | MirExpr::GenToList { .. }
        | MirExpr::GenAnyAll { .. }
        | MirExpr::GenYield { .. }
        | MirExpr::GenYieldFrom { .. }
        | MirExpr::GenSend { .. }
        | MirExpr::GenThrow { .. }
        | MirExpr::GenClose { .. } => true,
        _ => false,
    };
    own || expr
        .children()
        .into_iter()
        .any(|child| expr_reaches_cpython(child, frameless))
}
