//! `"literal".format(args)` -> a concatenation of the literal's text and
//! each replacement field rendered as `str(arg)`, `repr(arg)` or
//! `format(arg, spec)`.
//!
//! Only a literal format string whose fields are all automatically
//! numbered (`{}`) or all explicitly numbered (`{0}`) is rewritten, with a
//! `!s`/`!r` conversion and a constant format spec at most, positional
//! arguments only, and every argument used exactly once in order unless
//! all of them are names or literals (whose re-reading or skipping cannot
//! be observed). Anything else -- keyword fields, `{0.attr}`, `{0[i]}`,
//! nested specs, a field the arguments do not cover -- is left alone, so
//! CPython's own `str.format` (or a diagnostic) handles it.
//!
//! `{}` renders as `str(arg)`: `format(arg, "")` and `str(arg)` agree for
//! every value without its own `__format__`.

use super::build::{call_name, string};
use super::is_simple;
use ruff_python_ast::{Expr, ExprCall, Operator};
use ruff_text_size::TextRange;

enum Piece {
    Text(String),
    Field {
        index: usize,
        conversion: Option<char>,
        spec: String,
    },
}

/// The rewritten call, or `None` when `call_expr` is not a rewritable
/// `"...".format(...)`. `builtin` says whether a builtin name is unshadowed.
pub(super) fn rewrite(call_expr: &ExprCall, builtin: &dyn Fn(&str) -> bool) -> Option<Expr> {
    let Expr::Attribute(attr) = call_expr.func.as_ref() else {
        return None;
    };
    if attr.attr.as_str() != "format" || !call_expr.arguments.keywords.is_empty() {
        return None;
    }
    let Expr::StringLiteral(literal) = attr.value.as_ref() else {
        return None;
    };
    let args = &call_expr.arguments.args;
    if args.iter().any(|a| matches!(a, Expr::Starred(_))) {
        return None;
    }
    if !["str", "repr", "format"].iter().all(|name| builtin(name)) {
        return None;
    }
    let pieces = parse(literal.value.to_str(), args.len())?;
    let used: Vec<usize> = pieces
        .iter()
        .filter_map(|p| match p {
            Piece::Field { index, .. } => Some(*index),
            Piece::Text(_) => None,
        })
        .collect();
    let in_order = used.iter().copied().eq(0..args.len());
    if !in_order && !args.iter().all(is_simple) {
        return None;
    }
    let range = call_expr.range;
    let mut parts: Vec<Expr> = Vec::new();
    for piece in pieces {
        parts.push(match piece {
            Piece::Text(text) => string(&text, range),
            Piece::Field {
                index,
                conversion,
                spec,
            } => render(args[index].clone(), conversion, &spec, range),
        });
    }
    let mut parts = parts.into_iter();
    let first = parts.next().unwrap_or_else(|| string("", range));
    Some(parts.fold(first, |acc, part| {
        Expr::BinOp(ruff_python_ast::ExprBinOp {
            node_index: Default::default(),
            range,
            left: Box::new(acc),
            op: Operator::Add,
            right: Box::new(part),
        })
    }))
}

fn render(arg: Expr, conversion: Option<char>, spec: &str, range: TextRange) -> Expr {
    let converted = match conversion {
        Some('r') => Some(call_name("repr", vec![arg.clone()], range)),
        Some(_) => Some(call_name("str", vec![arg.clone()], range)),
        None => None,
    };
    match (converted, spec.is_empty()) {
        (Some(c), true) => c,
        (None, true) => call_name("str", vec![arg], range),
        (Some(c), false) => call_name("format", vec![c, string(spec, range)], range),
        (None, false) => call_name("format", vec![arg, string(spec, range)], range),
    }
}

/// The literal's pieces, or `None` when a field is outside this rewrite's
/// subset or the field numbering does not fit `nargs` arguments.
fn parse(text: &str, nargs: usize) -> Option<Vec<Piece>> {
    let mut pieces = Vec::new();
    let mut buf = String::new();
    let mut chars = text.chars().peekable();
    let mut auto = None::<bool>;
    let mut next_auto = 0usize;
    while let Some(c) = chars.next() {
        match c {
            '{' if chars.peek() == Some(&'{') => {
                chars.next();
                buf.push('{');
            }
            '}' if chars.peek() == Some(&'}') => {
                chars.next();
                buf.push('}');
            }
            '}' => return None,
            '{' => {
                let mut field = String::new();
                loop {
                    match chars.next()? {
                        '}' => break,
                        '{' => return None,
                        ch => field.push(ch),
                    }
                }
                let (head, spec) = match field.split_once(':') {
                    Some((head, spec)) => (head, spec.to_string()),
                    None => (field.as_str(), String::new()),
                };
                let (name, conversion) = match head.split_once('!') {
                    Some((name, conv)) => match conv {
                        "r" => (name, Some('r')),
                        "s" => (name, Some('s')),
                        _ => return None,
                    },
                    None => (head, None),
                };
                let index = if name.is_empty() {
                    if auto == Some(false) {
                        return None;
                    }
                    auto = Some(true);
                    next_auto += 1;
                    next_auto - 1
                } else {
                    if auto == Some(true) || !name.bytes().all(|b| b.is_ascii_digit()) {
                        return None;
                    }
                    auto = Some(false);
                    name.parse().ok()?
                };
                if index >= nargs {
                    return None;
                }
                if !buf.is_empty() {
                    pieces.push(Piece::Text(std::mem::take(&mut buf)));
                }
                pieces.push(Piece::Field {
                    index,
                    conversion,
                    spec,
                });
            }
            other => buf.push(other),
        }
    }
    if !buf.is_empty() {
        pieces.push(Piece::Text(buf));
    }
    Some(pieces)
}
