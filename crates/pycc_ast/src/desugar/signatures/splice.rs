//! `f(**{'a': x, 'b': y})`: a `**` operand that is a dict display of
//! distinct string-literal keys spelling identifiers is the keyword
//! arguments `f(a=x, b=y)`.
//!
//! CPython evaluates the display's keys and values left to right before
//! binding, as it evaluates keyword arguments, so the rewrite keeps the
//! evaluation order. It is skipped when the display could bind differently:
//! a repeated key (a dict keeps the last value silently, a call would
//! raise), a key that also names an explicit keyword of the call (CPython's
//! "got multiple values" `TypeError`, kept on the run-time path), a
//! `**mapping` entry inside the display, or a key that is not an identifier
//! (only a `**kwargs` parameter can receive it).

use ruff_python_ast::{AtomicNodeIndex, Expr, ExprCall, Identifier, Keyword};
use ruff_text_size::Ranged;
use std::collections::HashSet;

/// `call` with each such operand replaced by its keywords, or `None` when
/// it has none.
pub(super) fn spliced(call: &ExprCall) -> Option<ExprCall> {
    let mut copy = call.clone();
    splice_dict_displays(&mut copy).then_some(copy)
}

/// Python's reserved words: never a keyword argument's name.
const RESERVED: &[&str] = &[
    "False", "None", "True", "and", "as", "assert", "async", "await", "break", "class",
    "continue", "def", "del", "elif", "else", "except", "finally", "for", "from", "global",
    "if", "import", "in", "is", "lambda", "nonlocal", "not", "or", "pass", "raise", "return",
    "try", "while", "with", "yield",
];

fn is_identifier(text: &str) -> bool {
    let mut chars = text.chars();
    matches!(chars.next(), Some(c) if c == '_' || c.is_ascii_alphabetic())
        && chars.all(|c| c == '_' || c.is_ascii_alphanumeric())
        && !RESERVED.contains(&text)
}

/// Replaces each such `**` operand of `call` with its keywords, returning
/// whether it replaced any.
fn splice_dict_displays(call: &mut ExprCall) -> bool {
    if !call
        .arguments
        .keywords
        .iter()
        .any(|k| k.arg.is_none() && matches!(k.value, Expr::Dict(_)))
    {
        return false;
    }
    let explicit: HashSet<String> = call
        .arguments
        .keywords
        .iter()
        .filter_map(|k| k.arg.as_ref().map(|a| a.to_string()))
        .collect();
    let old = std::mem::take(&mut call.arguments.keywords);
    let mut any = false;
    for keyword in old {
        match expansion(&keyword, &explicit) {
            Some(expanded) => {
                any = true;
                call.arguments.keywords.extend(expanded);
            }
            None => call.arguments.keywords.push(keyword),
        }
    }
    any
}

/// The keywords `keyword` (a `**{...}` display) stands for, or `None`.
fn expansion(keyword: &Keyword, explicit: &HashSet<String>) -> Option<Vec<Keyword>> {
    if keyword.arg.is_some() {
        return None;
    }
    let Expr::Dict(dict) = &keyword.value else {
        return None;
    };
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for item in &dict.items {
        let Some(Expr::StringLiteral(key)) = &item.key else {
            return None;
        };
        let name = key.value.to_str().to_string();
        if !is_identifier(&name) || explicit.contains(&name) || !seen.insert(name.clone()) {
            return None;
        }
        out.push(Keyword {
            node_index: AtomicNodeIndex::default(),
            range: item.value.range(),
            arg: Some(Identifier::new(name.as_str(), key.range())),
            value: item.value.clone(),
        });
    }
    Some(out)
}
