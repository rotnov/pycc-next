//! Generator functions (a body containing `yield`): their return type and
//! their `return` statements.

use super::{is_ext_module, is_non_strict};
use crate::{HirExpr, HirStmt, Ty, unsupported};
use pycc_diag::Diagnostic;

/// The return type of a generator function (a body containing `yield`):
/// its annotation (`Iterator[T]`, `Iterable[T]`, `Generator[T, None,
/// None]`), or `Generator[<inferred>]` when its signature is inferred.
pub(crate) fn generator_return_ty(
    return_ty: Ty,
    aliases: &[(String, Ty)],
    def: &pycc_ast::StmtFunctionDef,
) -> Result<Ty, Diagnostic> {
    match return_ty {
        Ty::Generator(_) => Ok(return_ty),
        Ty::Infer => Ok(Ty::generator(Ty::Infer)),
        // Non-strict mode: an unannotated public generator outside an
        // `--ext` module is inferred like any other function there.
        Ty::Object
            if is_non_strict(aliases) && !is_ext_module(aliases) && def.returns.is_none() =>
        {
            Ok(Ty::generator(Ty::Infer))
        }
        other => Err(unsupported(
            format!(
                "a generator function must be annotated `Iterator[T]`, `Iterable[T]` or \
                 `Generator[T, None, None]`, not `{}`",
                other.name()
            ),
            def.returns.as_deref().map_or(
                std::ops::Range::<u32>::from(def.range),
                pycc_ast::expr_range,
            ),
        )),
    }
}

/// A generator's `return` statements end it: `return` and `return None`
/// are kept as a bare `return`; `return value` becomes the
/// [`crate::GEN_RETURN_MARKER`] statement (the value for
/// `StopIteration.value`) followed by a bare `return`.
pub(crate) fn generator_returns(
    body: &mut Vec<HirStmt>,
    def: &pycc_ast::StmtFunctionDef,
) -> Result<(), Diagnostic> {
    let mut index = 0;
    while index < body.len() {
        match &mut body[index] {
            HirStmt::Return(Some(HirExpr::NoneLiteral)) => body[index] = HirStmt::Return(None),
            HirStmt::Return(Some(_)) => {
                let HirStmt::Return(Some(value)) =
                    std::mem::replace(&mut body[index], HirStmt::Return(None))
                else {
                    unreachable!()
                };
                body.insert(
                    index,
                    HirStmt::ExprStmt(HirExpr::Call {
                        callee: crate::GEN_RETURN_MARKER.to_string(),
                        args: vec![value],
                    }),
                );
                index += 1;
            }
            HirStmt::If { body, orelse, .. } => {
                generator_returns(body, def)?;
                generator_returns(orelse, def)?;
            }
            HirStmt::While { body, .. }
            | HirStmt::ForRange { body, .. }
            | HirStmt::ForList { body, .. }
            | HirStmt::ForObject { body, .. } => generator_returns(body, def)?,
            HirStmt::LoopElse { loop_stmt, orelse } => {
                let mut inner = vec![std::mem::replace(loop_stmt.as_mut(), HirStmt::Return(None))];
                generator_returns(&mut inner, def)?;
                **loop_stmt = inner.pop().expect("a loop statement stays one statement");
                generator_returns(orelse, def)?;
            }
            HirStmt::Try {
                body,
                handlers,
                orelse,
                finalbody,
            }
            | HirStmt::TryStar {
                body,
                handlers,
                orelse,
                finalbody,
            } => {
                generator_returns(body, def)?;
                for handler in handlers.iter_mut() {
                    generator_returns(&mut handler.body, def)?;
                }
                generator_returns(orelse, def)?;
                generator_returns(finalbody, def)?;
            }
            HirStmt::Match { cases, .. } => {
                for case in cases.iter_mut() {
                    generator_returns(&mut case.body, def)?;
                }
            }
            _ => {}
        }
        index += 1;
    }
    Ok(())
}
