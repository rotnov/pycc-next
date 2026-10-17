//! Lowering an f-string: its literal parts and interpolations, a `!r`/`!s`
//! conversion becoming a `repr()`/`str()` call of the value.

use super::keyword_bind::SignatureTable;
use super::lower_expr;
use crate::{FStringPart, HirExpr, ImportBinding, unsupported};
use pycc_diag::Diagnostic;

/// One element of an f-string expression's parts.
enum Element<'a> {
    /// A plain string literal part's text.
    Text(String),
    Of(&'a pycc_ast::InterpolatedStringElement),
}

pub(super) fn lower_fstring(
    fstring: &pycc_ast::ExprFString,
    in_function: bool,
    class_name: Option<&str>,
    imports: &[ImportBinding],
    signatures: &SignatureTable,
) -> Result<HirExpr, Diagnostic> {
    // Every part of an implicit concatenation (`"a" f"{b}"`): a plain
    // string literal's text, an f-string's elements.
    let elements = fstring.value.iter().flat_map(|part| match part {
        pycc_ast::FStringPart::Literal(lit) => {
            vec![Element::Text(lit.value.to_string())]
        }
        pycc_ast::FStringPart::FString(f) => f.elements.iter().map(Element::Of).collect(),
    });
    let parts = elements
        .map(|element| -> Result<Vec<FStringPart>, Diagnostic> {
            let element = match element {
                Element::Text(text) => return Ok(vec![FStringPart::Literal(text)]),
                Element::Of(element) => element,
            };
            Ok(match element {
                pycc_ast::InterpolatedStringElement::Literal(lit) => {
                    vec![FStringPart::Literal(lit.value.to_string())]
                }
                pycc_ast::InterpolatedStringElement::Interpolation(interp) => {
                    // The `=` debug specifier (`f"{n=}"`): the source text
                    // between the braces (up to a conversion or spec), then
                    // the value -- its `repr` unless a conversion or a format
                    // spec says otherwise, as CPython renders it.
                    let debug = interp.debug_text.as_ref().map(|debug| debug.as_str().to_string());
                    // `!r` / `!s` are `repr(x)` / `str(x)` of the value.
                    let conversion = match interp.conversion {
                        pycc_ast::ConversionFlag::None
                            if debug.is_some() && interp.format_spec.is_none() =>
                        {
                            Some("repr")
                        }
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
                    // A constant spec (`{x:>8.3f}`) is `format(value, spec)`;
                    // one with a nested field (`{x:{w}}`) is not compiled.
                    let spec = match &interp.format_spec {
                        None => None,
                        Some(spec) => {
                            let mut text = String::new();
                            for element in &spec.elements {
                                match element {
                                    pycc_ast::InterpolatedStringElement::Literal(lit) => {
                                        text.push_str(&lit.value);
                                    }
                                    pycc_ast::InterpolatedStringElement::Interpolation(_) => {
                                        return Err(unsupported(
                                            "an f-string format spec with a nested field \
                                             ({x:{w}}) is not supported yet",
                                            interp.range,
                                        ));
                                    }
                                }
                            }
                            Some(text)
                        }
                    };
                    let value = lower_expr(
                        &interp.expression,
                        in_function,
                        class_name,
                        imports,
                        signatures,
                    )?;
                    let value = match conversion {
                        Some(callee) => HirExpr::Call {
                            callee: callee.to_string(),
                            args: vec![value],
                        },
                        None => value,
                    };
                    let value = FStringPart::Interpolation(Box::new(match spec {
                        Some(spec) => HirExpr::Call {
                            callee: "__pycc_format".to_string(),
                            args: vec![value, HirExpr::StringLiteral(spec)],
                        },
                        None => value,
                    }));
                    match debug {
                        Some(text) => vec![FStringPart::Literal(text), value],
                        None => vec![value],
                    }
                }
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(HirExpr::FString(parts.into_iter().flatten().collect()))
}
