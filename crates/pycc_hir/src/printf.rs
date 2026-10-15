//! printf-style formatting, `template % args`, with a literal template:
//! the template split into its literal text and its conversions, shared by
//! `pycc_types` (which admits the expression as a `str`) and `pycc_mir`
//! (which lowers it to an f-string of one `__pycc_printf(arg, conversion)`
//! runtime call per conversion, `pycc_rt::format::printf`).

use crate::{HirExpr, Ty};

/// One piece of a template.
#[derive(Debug, Clone, PartialEq)]
pub enum PrintfPiece {
    Literal(String),
    /// A conversion's text after the `%` (flags, width, precision and the
    /// conversion character, `-08.3f`) and its conversion character.
    Conversion { spec: String, conv: char },
}

/// `template` split into pieces, or `None` for a template this compiler
/// does not split: a mapping key (`%(name)s`), a `*` width or precision,
/// an unknown conversion character, or a dangling `%`.
#[must_use]
pub fn parse_template(template: &str) -> Option<Vec<PrintfPiece>> {
    let mut pieces = Vec::new();
    let mut literal = String::new();
    let mut chars = template.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '%' {
            literal.push(c);
            continue;
        }
        let mut spec = String::new();
        loop {
            let next = chars.next()?;
            match next {
                '%' if spec.is_empty() => {
                    literal.push('%');
                    break;
                }
                '-' | '+' | ' ' | '#' | '0'..='9' | '.' | 'h' | 'l' | 'L' => spec.push(next),
                'd' | 'i' | 'u' | 'o' | 'x' | 'X' | 'e' | 'E' | 'f' | 'F' | 'g' | 'G' | 'c'
                | 's' | 'r' | 'a' => {
                    spec.push(next);
                    if !literal.is_empty() {
                        pieces.push(PrintfPiece::Literal(std::mem::take(&mut literal)));
                    }
                    pieces.push(PrintfPiece::Conversion { spec, conv: next });
                    break;
                }
                _ => return None,
            }
        }
    }
    if !literal.is_empty() {
        pieces.push(PrintfPiece::Literal(literal));
    }
    Some(pieces)
}

/// The arguments `args` supplies: a tuple display's elements, or `args`
/// itself.
#[must_use]
pub fn printf_args(args: &HirExpr) -> Vec<&HirExpr> {
    match args {
        HirExpr::TupleLiteral(elements) => elements.iter().collect(),
        other => vec![other],
    }
}

/// Whether conversion `conv` formats a value of type `ty` natively
/// (`pycc_rt::format::printf`'s entry points).
#[must_use]
pub fn admits(conv: char, ty: &Ty) -> bool {
    match conv {
        's' | 'r' | 'a' => matches!(ty, Ty::Int | Ty::Bool | Ty::Float | Ty::Str),
        'd' | 'i' | 'u' | 'e' | 'E' | 'f' | 'F' | 'g' | 'G' => {
            matches!(ty, Ty::Int | Ty::Bool | Ty::Float)
        }
        'o' | 'x' | 'X' => matches!(ty, Ty::Int | Ty::Bool),
        'c' => matches!(ty, Ty::Int | Ty::Str),
        _ => false,
    }
}

/// The conversions of `template % args` when it formats natively: the
/// template is a literal this compiler splits, `args` supplies exactly one
/// argument per conversion, and each argument's type (`arg_ty`) is one its
/// conversion takes.
#[must_use]
pub fn native_printf(
    template: &str,
    args: &HirExpr,
    mut arg_ty: impl FnMut(&HirExpr) -> Option<Ty>,
) -> Option<Vec<PrintfPiece>> {
    let pieces = parse_template(template)?;
    let args = printf_args(args);
    let convs: Vec<char> = pieces
        .iter()
        .filter_map(|piece| match piece {
            PrintfPiece::Conversion { conv, .. } => Some(*conv),
            PrintfPiece::Literal(_) => None,
        })
        .collect();
    if convs.len() != args.len() {
        return None;
    }
    for (conv, arg) in convs.iter().zip(args) {
        if !admits(*conv, &arg_ty(arg)?) {
            return None;
        }
    }
    Some(pieces)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn templates_split_into_literals_and_conversions() {
        assert_eq!(
            parse_template("a %-5.2f%% %d").unwrap(),
            vec![
                PrintfPiece::Literal("a ".into()),
                PrintfPiece::Conversion { spec: "-5.2f".into(), conv: 'f' },
                PrintfPiece::Literal("% ".into()),
                PrintfPiece::Conversion { spec: "d".into(), conv: 'd' },
            ]
        );
        assert_eq!(parse_template("%(x)s"), None);
        assert_eq!(parse_template("%*d"), None);
        assert_eq!(parse_template("50%"), None);
    }
}
