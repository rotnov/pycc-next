//! Lowering an f-string: its literal parts and interpolations, a `!r`/`!s`
//! conversion becoming a `repr()`/`str()` call of the value.

use super::keyword_bind::SignatureTable;
use super::lower_expr;
use crate::{FStringPart, HirExpr, ImportBinding, unsupported};
use pycc_diag::Diagnostic;

pub(super) fn lower_fstring(
    fstring: &pycc_ast::ExprFString,
    in_function: bool,
    class_name: Option<&str>,
    imports: &[ImportBinding],
    signatures: &SignatureTable,
) -> Result<HirExpr, Diagnostic> {
    let parts = fstring
        .value
        .elements()
        .map(|element| -> Result<FStringPart, Diagnostic> {
            Ok(match element {
                pycc_ast::InterpolatedStringElement::Literal(lit) => {
                    FStringPart::Literal(lit.value.to_string())
                }
                pycc_ast::InterpolatedStringElement::Interpolation(interp) => {
                    if interp.debug_text.is_some() {
                        // #720: the `=` debug specifier (`f"{n=}"`) renders the
                        // source text plus `repr(value)`, which this crate does
                        // not implement. Silently discarding `debug_text` would
                        // compile cleanly and print the wrong value, so reject
                        // it explicitly instead.
                        return Err(unsupported(
                            "f-string debug specifier (=) is not supported yet",
                            interp.range,
                        ));
                    }
                    // `!r` / `!s` are `repr(x)` / `str(x)` of the value.
                    let conversion = match interp.conversion {
                        pycc_ast::ConversionFlag::None => None,
                        pycc_ast::ConversionFlag::Repr => Some("repr"),
                        pycc_ast::ConversionFlag::Str => Some("str"),
                        pycc_ast::ConversionFlag::Ascii => {
                            return Err(unsupported(
                                "the f-string conversion flag `!a` is not supported yet",
                                interp.range,
                            ));
                        }
                    };
                    if interp.format_spec.is_some() {
                        return Err(unsupported(
                            "f-string format spec ({x:...}) is not supported yet",
                            interp.range,
                        ));
                    }
                    let value = lower_expr(
                        &interp.expression,
                        in_function,
                        class_name,
                        imports,
                        signatures,
                    )?;
                    FStringPart::Interpolation(Box::new(match conversion {
                        Some(callee) => HirExpr::Call {
                            callee: callee.to_string(),
                            args: vec![value],
                        },
                        None => value,
                    }))
                }
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(HirExpr::FString(parts))
}
