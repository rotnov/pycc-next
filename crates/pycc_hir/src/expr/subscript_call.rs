//! Lowering a call whose callee is a subscript, `X[Y](args)` (PEP 695
//! generic class instantiation, #387; a call of a subscript result, Part 2a
//! of #1371).
//!
//! Lowering has no type information, so it tells the two readings apart
//! syntactically. The call is a `HirExpr::GenericClassInstantiate`, exactly
//! as before Part 2a, when `X` is a bare name and either
//!
//! * `X` is one of the module's *class-like* names
//!   (`SignatureTable::is_class_like`: bound once, by a top-level `class` or
//!   a project `from` import), so every type-argument diagnostic
//!   (`C[1](x)`, `C[unknown](x)`, an imported `Box[Foo](x)`) stays a located
//!   `C0001` here; or
//! * `Y` is a bare `int`/`float`/`bool`/`str`, so `C[int](x)` keeps today's
//!   behaviour for every other bare-name base. An object table keyed by
//!   a type (`handlers[int](x)`) is therefore still a generic instantiation
//!   and still refused downstream (`T0001`) -- a documented residual.
//!
//! Every other subscript callee -- a non-name base (`self.cbs[k](x)`,
//! `(1 + 2)[int](1)`) or a name base with any other key (`callbacks[k](x)`)
//! -- lowers to `HirExpr::ExprCall` with the subscript as its callee.
//! `pycc_types` admits that only on a CPython object and otherwise reports
//! the pre-Part-2a "calling a subscript expression" `C0001`, at the type
//! stage's own position.

use pycc_ast::{Expr, ExprCall, ExprSubscript};
use pycc_diag::Diagnostic;

use super::keyword_bind::SignatureTable;
use super::{lower_expr, type_arg_name_to_ty};
use crate::{HirExpr, ImportBinding};

/// Whether `slice` is one of the four scalar type names a generic class
/// instantiation accepts as its type argument.
fn is_scalar_type_name(slice: &Expr) -> bool {
    matches!(slice, Expr::Name(name) if matches!(name.id.as_str(), "int" | "float" | "bool" | "str"))
}

/// Lowers `call`, whose callee is the subscript `sub` (module doc).
pub(super) fn lower(
    call: &ExprCall,
    sub: &ExprSubscript,
    in_function: bool,
    class_name: Option<&str>,
    imports: &[ImportBinding],
    signatures: &SignatureTable,
) -> Result<HirExpr, Diagnostic> {
    if let Expr::Name(base) = sub.value.as_ref()
        && (signatures.is_class_like(base.id.as_str()) || is_scalar_type_name(&sub.slice))
    {
        let type_arg = type_arg_name_to_ty(&sub.slice)?;
        let args = lower_args(call, in_function, class_name, imports, signatures)?;
        return Ok(HirExpr::GenericClassInstantiate {
            class: base.id.as_str().to_string(),
            type_arg,
            args,
        });
    }
    // The callee is lowered before the arguments, which is CPython's own
    // evaluation order and the order MIR and codegen keep.
    let callee = lower_expr(&call.func, in_function, class_name, imports, signatures)?;
    let args = lower_args(call, in_function, class_name, imports, signatures)?;
    Ok(HirExpr::ExprCall {
        callee: Box::new(callee),
        args,
    })
}

fn lower_args(
    call: &ExprCall,
    in_function: bool,
    class_name: Option<&str>,
    imports: &[ImportBinding],
    signatures: &SignatureTable,
) -> Result<Vec<HirExpr>, Diagnostic> {
    call.arguments
        .args
        .iter()
        .map(|e| lower_expr(e, in_function, class_name, imports, signatures))
        .collect()
}
