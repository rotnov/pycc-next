//! Locals that may hold a *borrowed* reference (`heap_rc`): no count
//! traffic on their stores and no release on the way out.
//!
//! `c = bodies[j]` in a hot loop would otherwise increment the new element
//! and decrement the old one on every trip. That is unnecessary exactly when
//! the list the element came from keeps owning it for as long as `c` can be
//! read, which this module proves conservatively for a function:
//!
//! - every binding of the local is `v = p[i]` or `for v in p`, where `p` is a
//!   `list` parameter (the frame owns it for the whole call) the body never
//!   rebinds;
//! - every statement and expression in the body is on an **allowlist** of
//!   shapes that can neither remove a list element nor run code that might:
//!   reads of names, plain attribute slots and list/tuple elements,
//!   arithmetic and comparisons whose operands are all `int`/`float`/`bool`/
//!   `str`, `len` of a list or `str`, assignments, attribute stores, `if`,
//!   `while`, `for` over a range or a list, `break`, `continue`, `return`.
//!   A call into user code, a constructor, an instance comparison (which
//!   dispatches `__eq__`), a property, a list mutation, a CPython object
//!   operation, `print`, `try`, `raise` -- anything not listed --
//!   disqualifies the whole function.
//!
//! The walk is structural (`MirExpr::children`, `MirStmt::exprs` and
//! `MirStmt::bodies` give the shape; the matches below name what is
//! admitted), so a new or unexpected shape costs speed, never safety.
//!
//! A borrowed local's value is still a borrowed read when used, so storing
//! it anywhere else takes a reference as usual.

use super::*;
use pycc_mir::Ty;
use std::collections::HashSet;

/// The locals of a function with parameters `params` and body `body` that
/// may hold borrowed references; `candidates` are its counted locals.
pub(super) fn borrowed_locals<'a>(
    params: &[(String, Ty)],
    body: &[MirStmt],
    candidates: impl Iterator<Item = &'a String>,
) -> HashSet<String> {
    if !body.iter().all(stmt_is_admitted) {
        return HashSet::new();
    }
    let mut bindings: Vec<Binding<'_>> = Vec::new();
    for stmt in body {
        collect_bindings(stmt, &mut bindings);
    }
    let stable_params: HashSet<&str> = params
        .iter()
        .filter(|(name, ty)| {
            matches!(ty, Ty::List(_)) && !bindings.iter().any(|binding| binding.target == name)
        })
        .map(|(name, _)| name.as_str())
        .collect();
    if stable_params.is_empty() {
        return HashSet::new();
    }
    candidates
        .filter(|name| {
            bindings
                .iter()
                .filter(|binding| binding.target == name.as_str())
                .all(|binding| {
                    binding
                        .source
                        .is_some_and(|source| stable_params.contains(source))
                })
        })
        .cloned()
        .collect()
}

/// One binding of a local: its name and, when the bound value is an
/// element of a named list (`v = p[i]`, `for v in p`), that list's name.
/// The allowlist admits no other binding form (no walrus, comprehension
/// target or `except ... as`).
struct Binding<'a> {
    target: &'a str,
    source: Option<&'a str>,
}

fn collect_bindings<'a>(stmt: &'a MirStmt, out: &mut Vec<Binding<'a>>) {
    match stmt {
        MirStmt::Assign { target, value } => out.push(Binding {
            target,
            source: element_source(value),
        }),
        MirStmt::ForList { var, list, .. } => out.push(Binding {
            target: var,
            source: Some(list),
        }),
        MirStmt::ForRange { var, .. } => out.push(Binding {
            target: var,
            source: None,
        }),
        _ => {}
    }
    for body in stmt.bodies() {
        for inner in body {
            collect_bindings(inner, out);
        }
    }
}

/// `p` when `value` is `p[i]` for a list-typed name `p`.
fn element_source(value: &MirExpr) -> Option<&str> {
    let MirExpr::Subscript { base, .. } = value else {
        return None;
    };
    match base.as_ref() {
        MirExpr::Name {
            name,
            ty: Ty::List(_),
        } => Some(name),
        _ => None,
    }
}

/// Whether `stmt`, and everything nested in it, is on the allowlist.
fn stmt_is_admitted(stmt: &MirStmt) -> bool {
    let shape = matches!(
        stmt,
        MirStmt::Assign { .. }
            | MirStmt::ExprStmt(_)
            | MirStmt::If { .. }
            | MirStmt::While { .. }
            | MirStmt::ForRange { .. }
            | MirStmt::ForList { .. }
            | MirStmt::AttrSet { .. }
            | MirStmt::Return(_)
            | MirStmt::Seq(_)
            | MirStmt::LoopElse { .. }
            | MirStmt::Break
            | MirStmt::Continue
            | MirStmt::NoOp
            | MirStmt::ReleaseName(_)
    );
    shape
        && stmt.exprs().into_iter().all(expr_is_admitted)
        && stmt
            .bodies()
            .into_iter()
            .all(|body| body.iter().all(stmt_is_admitted))
}

/// A type whose operators run no user code.
fn is_primitive(ty: &Ty) -> bool {
    matches!(ty, Ty::Int | Ty::Float | Ty::Bool | Ty::Str)
}

/// Whether `expr`, and everything nested in it, is on the allowlist.
fn expr_is_admitted(expr: &MirExpr) -> bool {
    let shape = match expr {
        MirExpr::IntLiteral(_)
        | MirExpr::FloatLiteral(_)
        | MirExpr::BoolLiteral(_)
        | MirExpr::StringLiteral(_)
        | MirExpr::NoneLiteral
        | MirExpr::Name { .. }
        | MirExpr::IntBoundary(_) => true,
        // A plain slot read: a property getter lowers to a call instead.
        MirExpr::AttrGet { .. } => true,
        MirExpr::Subscript { base, .. } => matches!(base.ty(), Ty::List(_) | Ty::Tuple(_)),
        MirExpr::BinOp { left, right, .. }
        | MirExpr::Compare { left, right, .. }
        | MirExpr::BoolOp { left, right, .. } => {
            is_primitive(&left.ty()) && is_primitive(&right.ty())
        }
        MirExpr::Not(operand) => is_primitive(&operand.ty()),
        MirExpr::IfExp { test, .. } => is_primitive(&test.ty()),
        MirExpr::Call { callee, args, .. } => {
            callee == "len"
                && matches!(args.as_slice(), [arg] if matches!(arg.ty(), Ty::List(_) | Ty::Str))
        }
        _ => false,
    };
    shape && expr.children().into_iter().all(expr_is_admitted)
}

#[cfg(test)]
mod tests {
    use super::*;
    use pycc_mir::{CmpOpKind, MirExpr, MirStmt, Ty};

    fn body_ty() -> Ty {
        Ty::Instance(Box::new("Body".to_string()))
    }

    fn name(name: &str, ty: Ty) -> MirExpr {
        MirExpr::Name {
            name: name.to_string(),
            ty,
        }
    }

    fn element_read(var: &str, list: &str) -> MirStmt {
        MirStmt::Assign {
            target: var.to_string(),
            value: MirExpr::Subscript {
                base: Box::new(name(list, Ty::List(Box::new(body_ty())))),
                index: Box::new(MirExpr::IntLiteral(0)),
            },
        }
    }

    fn params() -> Vec<(String, Ty)> {
        vec![("bodies".to_string(), Ty::List(Box::new(body_ty())))]
    }

    fn candidates() -> Vec<String> {
        vec!["c".to_string()]
    }

    fn found(body: &[MirStmt]) -> HashSet<String> {
        borrowed_locals(&params(), body, candidates().iter())
    }

    /// `c.x` as a float read.
    fn slot_read(var: &str) -> MirExpr {
        MirExpr::AttrGet {
            base: Box::new(name(var, body_ty())),
            slot: 0,
            ty: Ty::Float,
        }
    }

    #[test]
    fn an_element_read_from_a_stable_parameter_is_borrowed() {
        assert!(found(&[element_read("c", "bodies")]).contains("c"));
        let for_list = MirStmt::ForList {
            var: "c".to_string(),
            var_ty: body_ty(),
            list: "bodies".to_string(),
            body: vec![MirStmt::AttrSet {
                base: name("c", body_ty()),
                slot: 0,
                value: MirExpr::BinOp {
                    op: pycc_mir::BinOpKind::Add,
                    left: Box::new(slot_read("c")),
                    right: Box::new(MirExpr::FloatLiteral(1.0)),
                    ty: Ty::Float,
                },
            }],
        };
        assert!(found(&[for_list]).contains("c"));
    }

    #[test]
    fn a_rebound_parameter_or_a_mutation_or_a_call_disqualifies() {
        let rebinding = vec![
            element_read("c", "bodies"),
            MirStmt::Assign {
                target: "bodies".to_string(),
                value: name("other", Ty::List(Box::new(body_ty()))),
            },
        ];
        assert!(found(&rebinding).is_empty());
        let call = vec![
            element_read("c", "bodies"),
            MirStmt::ExprStmt(MirExpr::Call {
                callee: "f".to_string(),
                args: vec![],
                ty: Ty::None,
            }),
        ];
        assert!(found(&call).is_empty());
        let other_source = vec![element_read("c", "local_list")];
        assert!(found(&other_source).is_empty());
    }

    /// An instance comparison dispatches `__eq__`, which may run anything:
    /// it is not on the allowlist even though it is a plain `Compare`.
    #[test]
    fn an_instance_comparison_disqualifies() {
        let body = vec![
            element_read("c", "bodies"),
            MirStmt::ExprStmt(MirExpr::Compare {
                op: CmpOpKind::Eq,
                left: Box::new(name("c", body_ty())),
                right: Box::new(name("c", body_ty())),
                ty: Ty::Bool,
            }),
        ];
        assert!(found(&body).is_empty());
    }

    /// Any shape the allowlist does not name -- here a list display nested
    /// deep inside an admitted statement -- disqualifies.
    #[test]
    fn an_unlisted_shape_anywhere_disqualifies() {
        let body = vec![
            element_read("c", "bodies"),
            MirStmt::If {
                test: MirExpr::BoolLiteral(true),
                body: vec![MirStmt::Assign {
                    target: "t".to_string(),
                    value: MirExpr::ListLiteral(vec![MirExpr::IntLiteral(1)]),
                }],
                orelse: vec![],
            },
        ];
        assert!(found(&body).is_empty());
    }
}
