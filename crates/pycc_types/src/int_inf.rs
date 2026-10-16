//! A variable that holds `float('inf')` until an `int` replaces
//! it (`best = float('inf')` ... `best = min(best, cost)`).
//!
//! CPython's variable holds a float, then an int; one native type cannot
//! hold both. When a module does not check, every name some scope binds to
//! an infinity literal is a candidate: its infinities are rewritten into
//! `int` infinities (`__pycc_int_inf_pos()` / `__pycc_int_inf_neg()`,
//! `pycc_rt::int_inf`) -- all candidates at once, then one at a time -- and
//! a rewrite is kept when the module then checks, or fails in fewer places.
//! The name is then an `int` throughout, and every operation CPython defines
//! on the infinity with an exact result answers as it does
//! (`pycc_rt::int_inf`). A name whose other bindings are floats keeps the
//! `float('inf')` it had: rewriting it would only add failures.

use crate::module::{DiagnosticKey, KeyedDiagnostics};
use pycc_hir::{HirExpr, HirItem, HirModule, HirStmt, UnaryOpKind};

/// How many rewrites may be kept one after another.
const MAX_ROUNDS: usize = 8;

/// The scope of a candidate: the module's statements, or the body of the
/// function item at that index.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Scope {
    Module,
    Function(usize),
}

/// Runs `check` over `hir`, retrying with the infinities of candidate names
/// rewritten (module doc): all of them, then -- while that still fails --
/// without each one whose rewrite the failure gets further without; once
/// the module checks, without each one it also checks without.
pub(crate) fn with_int_inf<T>(
    hir: &HirModule,
    check: impl Fn(&HirModule) -> Result<T, KeyedDiagnostics>,
) -> Result<T, KeyedDiagnostics> {
    let first = check(hir);
    if first.is_ok() {
        return first;
    }
    let mut chosen = candidates(hir);
    if chosen.is_empty() {
        return first;
    }
    let attempt = |chosen: &[(Scope, String)]| {
        let mut module = hir.clone();
        for (scope, name) in chosen {
            rewrite(&mut module, *scope, name);
        }
        check(&module)
    };
    let mut result = attempt(&chosen);
    for _ in 0..MAX_ROUNDS {
        let Err(diagnostics) = &result else {
            return result;
        };
        let reached = progress(diagnostics);
        let mut improved = None;
        for index in 0..chosen.len() {
            let mut fewer = chosen.clone();
            fewer.remove(index);
            if fewer.is_empty() {
                break;
            }
            let trial = attempt(&fewer);
            let better = match &trial {
                Ok(_) => true,
                Err(diagnostics) => progress(diagnostics) > reached,
            };
            if better {
                improved = Some((fewer, trial));
                break;
            }
        }
        let Some((fewer, trial)) = improved else {
            break;
        };
        chosen = fewer;
        result = trial;
    }
    if result.is_err() {
        // No rewrite checks: the diagnostics are the program's own.
        return first;
    }
    // Only the names the module needs rewritten stay rewritten: a float
    // name that also checks rewritten keeps its `float('inf')`.
    let mut index = 0;
    while index < chosen.len() && chosen.len() > 1 {
        let mut fewer = chosen.clone();
        fewer.remove(index);
        match attempt(&fewer) {
            Ok(trial) => {
                chosen = fewer;
                result = Ok(trial);
            }
            Err(_) => index += 1,
        }
    }
    result
}

/// How far a failing check got: the checker stops at a module statement
/// that fails, so the first failing item's index comes first, then fewer
/// failures.
fn progress(diagnostics: &KeyedDiagnostics) -> (usize, std::cmp::Reverse<usize>) {
    let first = diagnostics
        .iter()
        .map(|(key, _)| match key {
            DiagnosticKey::Function(index) | DiagnosticKey::TopLevel(index) => *index,
            _ => 0,
        })
        .min()
        .unwrap_or(0);
    (first, std::cmp::Reverse(diagnostics.len()))
}

/// Every `(scope, name)` some assignment binds to an infinity literal.
fn candidates(hir: &HirModule) -> Vec<(Scope, String)> {
    let mut found: Vec<(Scope, String)> = Vec::new();
    for (index, item) in hir.items.iter().enumerate() {
        let (scope, body) = match item {
            HirItem::Function { body, .. } => (Scope::Function(index), body.clone()),
            HirItem::TopLevelStmt(stmt) => (Scope::Module, vec![stmt.clone()]),
        };
        let mut body = body;
        crate::empty_container::for_each_stmt_mut(&mut body, &mut |stmt| {
            if let HirStmt::Assign { target, value } = stmt
                && infinity(value).is_some()
                && !found.iter().any(|(s, n)| *s == scope && n == target)
            {
                found.push((scope, target.clone()));
            }
        });
    }
    found
}

/// Rewrites the infinities bound to `name` in `scope`.
fn rewrite(hir: &mut HirModule, scope: Scope, name: &str) {
    match scope {
        Scope::Function(index) => {
            if let Some(HirItem::Function { body, .. }) = hir.items.get_mut(index) {
                rewrite_body(body, name);
            }
        }
        Scope::Module => {
            for item in &mut hir.items {
                if let HirItem::TopLevelStmt(stmt) = item {
                    rewrite_body(std::slice::from_mut(stmt), name);
                }
            }
        }
    }
}

fn rewrite_body(body: &mut [HirStmt], name: &str) {
    crate::empty_container::for_each_stmt_mut(body, &mut |stmt| {
        if let HirStmt::Assign { target, value } = stmt
            && target == name
            && let Some(negative) = infinity(value)
        {
            *value = HirExpr::Call {
                callee: if negative {
                    "__pycc_int_inf_neg"
                } else {
                    "__pycc_int_inf_pos"
                }
                .to_string(),
                args: Vec::new(),
            };
        }
    });
}

/// The sign of an infinity literal (`float('inf')`, `float('-inf')`,
/// `math.inf`, or a negation of one): `Some(true)` for `-inf`.
fn infinity(expr: &HirExpr) -> Option<bool> {
    match expr {
        HirExpr::Call { callee, args } if callee == "float" => match args.as_slice() {
            [HirExpr::StringLiteral(text)] => {
                let text = text.trim().to_ascii_lowercase();
                let (negative, rest) = match text.strip_prefix('-') {
                    Some(rest) => (true, rest),
                    None => (false, text.strip_prefix('+').unwrap_or(&text)),
                };
                matches!(rest, "inf" | "infinity").then_some(negative)
            }
            _ => None,
        },
        HirExpr::Name(name) if name == "math.inf" => Some(false),
        HirExpr::UnaryOp {
            op: UnaryOpKind::USub,
            operand,
        } => infinity(operand).map(|negative| !negative),
        _ => None,
    }
}
