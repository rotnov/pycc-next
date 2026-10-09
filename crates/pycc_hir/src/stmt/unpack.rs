//! Tuple-unpacking assignment (`t1, ..., tn = value`, Part 1 of #891).
//!
//! `docs/TYPE_SYSTEM.md`'s "Tuple-unpacking assignment" section is the
//! canonical statement of the rule. CPython evaluates `value` once, unpacks
//! it into exactly `n` items -- raising before any target is bound when the
//! count differs -- and then assigns the items to the targets left to right.
//! This module lowers the statement into that order with ordinary
//! assignments, so every binding pass downstream sees nothing new:
//!
//! ```text
//! 0unpack_<offset> = Unpack(value, n)
//! t1 = 0unpack_<offset>[0]
//! ...
//! tn = 0unpack_<offset>[n - 1]
//! ```
//!
//! [`HirExpr::Unpack`] carries the arity check: `pycc_types` matches a
//! native `tuple[...]` value's length against `n` statically, and a CPython
//! object is unpacked at run time by CPython's own protocol into a fresh
//! `tuple` of exactly `n` items. The temporary's leading digit means no
//! source name can equal it, the D-117 argument `chain_assign`'s
//! `0chain_<offset>` temporary makes.
//!
//! Only a flat tuple or list of bare names is admitted. A starred, nested,
//! attribute or subscript element and an empty target are refused with
//! `C0001`, each naming what it met.

use crate::expr::keyword_bind::SignatureTable;
use crate::expr::{contains_named_expr, lower_expr};
use crate::{HirExpr, HirStmt, ImportBinding, unsupported};
use pycc_ast::{Expr, StmtAssign};
use pycc_diag::Diagnostic;

/// The prefix of every unpacking temporary. It begins with a digit, so no
/// Python source identifier can equal a name carrying it.
pub(crate) const UNPACK_TEMP_PREFIX: &str = "0unpack_";

/// The temporary an unpacking assignment whose statement starts at byte
/// `offset` binds its unpacked value to. The offset makes it unique within
/// one module.
fn synthesize_unpack_temp_name(offset: u32) -> String {
    format!("{UNPACK_TEMP_PREFIX}{offset}")
}

/// The element list of an unpacking target (`a, b` or `[a, b]`), or `None`
/// for any other target shape.
pub(super) fn unpack_target_elements(target: &Expr) -> Option<&[Expr]> {
    match target {
        Expr::Tuple(tuple) => Some(&tuple.elts),
        Expr::List(list) => Some(&list.elts),
        _ => None,
    }
}

/// The bare name an unpacking element binds, or the `C0001` refusing it.
fn element_name(element: &Expr) -> Result<&str, Diagnostic> {
    let refusal = match element {
        Expr::Name(name) => return Ok(name.id.as_str()),
        Expr::Starred(_) => {
            "a starred target (`*rest`) in a tuple-unpacking assignment is not supported yet"
                .to_string()
        }
        Expr::Tuple(_) | Expr::List(_) => {
            "a nested target (`a, (b, c) = ...`) in a tuple-unpacking assignment is not \
             supported yet"
                .to_string()
        }
        // An attribute or a subscript: the grammar admits no other target.
        other => format!(
            "only bare-name targets in a tuple-unpacking assignment are supported so far, got {}",
            pycc_ast::expr_kind_name(other)
        ),
    };
    Err(unsupported(refusal, pycc_ast::expr_range(element)))
}

/// Lowers the single-target `assign`, whose target `elements` came from
/// [`unpack_target_elements`], into the statements the module comment
/// shows, or refuses it with a `C0001`.
pub(super) fn lower_unpack(
    assign: &StmtAssign,
    elements: &[Expr],
    in_function: bool,
    class_name: Option<&str>,
    imports: &[ImportBinding],
    signatures: &SignatureTable,
) -> Result<Vec<HirStmt>, Diagnostic> {
    if elements.is_empty() {
        return Err(unsupported(
            "an empty unpacking target (`() = ...` or `[] = ...`) is not supported",
            pycc_ast::expr_range(&assign.targets[0]),
        ));
    }
    let names = elements
        .iter()
        .map(element_name)
        .collect::<Result<Vec<_>, _>>()?;
    let value = lower_expr(&assign.value, in_function, class_name, imports, signatures)?;
    if contains_named_expr(&value) {
        // The same refusal `lower_stmt` gives an assignment's value.
        return Err(unsupported(
            "a walrus assignment (`:=`) is only supported in an `if`/`while` \
             condition or as a bare expression statement (#774)",
            assign.range,
        ));
    }
    let temp = synthesize_unpack_temp_name(u32::from(assign.range.start()));
    let mut lowered = Vec::with_capacity(names.len() + 1);
    lowered.push(HirStmt::Assign {
        target: temp.clone(),
        value: HirExpr::Unpack {
            value: Box::new(value),
            arity: names.len(),
        },
    });
    for (index, name) in names.into_iter().enumerate() {
        lowered.push(HirStmt::Assign {
            target: name.to_string(),
            value: HirExpr::Subscript {
                base: Box::new(HirExpr::Name(temp.clone())),
                index: Box::new(HirExpr::IntLiteral(
                    i64::try_from(index).expect("a target list fits in i64"),
                )),
            },
        });
    }
    Ok(lowered)
}

#[cfg(test)]
mod tests;
