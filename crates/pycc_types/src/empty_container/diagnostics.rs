//! The `T0003` diagnostics for an empty container no evidence typed.

use super::*;

/// `T0003` for an empty list literal no source of evidence could type. Both
/// remedies are real in every position: the annotation source is purely
/// syntactic and the producer scan runs over a function body and over the
/// module body alike.
pub(crate) fn unresolved_list() -> Diagnostic {
    Diagnostic::error(
        "T0003",
        "an empty list literal has no inferable element type here".to_string(),
        Span::new(0, 0),
    )
    .with_help("annotate the assignment target (`xs: list[int] = []`) or append a value to it")
}

/// `T0003` for an empty dict literal no source of evidence could type.
pub(crate) fn unresolved_dict() -> Diagnostic {
    Diagnostic::error(
        "T0003",
        "an empty dict literal has no inferable key/value types here".to_string(),
        Span::new(0, 0),
    )
    .with_help(
        "annotate the assignment target (`d: dict[str, int] = {}`) or assign an entry into it",
    )
}

/// Names the binding in a `T0003` raised while checking `target`'s assigned
/// `value`. `T0003`'s span is the `Span::new(0, 0)` every container diagnostic
/// in this crate uses (`HirStmt::Assign` carries no span at all), so the
/// binding's name in the message is the only locator a user gets.
///
/// The substitution is gated on `value` *itself* being the empty literal that
/// failed (#1021 review round 1). A `T0003` also propagates out of a nested
/// element position -- `xs: list[int] = [[]]` fails on the inner `[]` while
/// `xs` is validly annotated -- and naming `xs` there points the user at the
/// wrong node. Such a diagnostic keeps `unresolved_list`/`unresolved_dict`'s
/// generic `" here"` wording, which is the only honest locator available when
/// the failing node is not the assigned value.
pub(crate) fn name_binding(
    mut diagnostic: Diagnostic,
    target: &str,
    value: &HirExpr,
) -> Diagnostic {
    if diagnostic.code == "T0003" && empty_literal(value).is_some() {
        // A placed diagnostic carries its own short label (#10); only the
        // headline repeated as the label follows the new headline.
        let repeats_headline = diagnostic.label.as_deref() == Some(diagnostic.message.as_str());
        diagnostic.message = diagnostic
            .message
            .replace(" here", &format!(" for `{target}`"));
        if repeats_headline {
            diagnostic.label = Some(diagnostic.message.clone());
        }
    }
    diagnostic
}
