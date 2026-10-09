use pycc_ast::ModModule;
use pycc_diag::{Diagnostic, Span};
use ruff_python_parser::{Mode, ParseOptions};

/// Parses `source` as a Python module and reports **every** syntax error
/// ruff's recovering parser found, each as an `L0001` diagnostic (#864
/// Part 1, D-217).
///
/// The `Err` vector is never empty: it is built from ruff's own
/// `Parsed::errors()` list, and this function only returns `Err` when
/// `has_valid_syntax()` is false, i.e. when that list is non-empty.
///
/// Diagnostics are emitted in ruff's *discovery* order (`Parsed::errors()`
/// verbatim), which is **not** guaranteed to be source order -- measured
/// against `ruff_python_parser 0.0.6`, a recovering parse can report an
/// error that starts later in the file before one that starts earlier.
/// That order is kept on purpose: `Parsed::into_result` (what [`parse`]
/// used to call) yields `errors()[0]`, so preserving discovery order is
/// exactly what keeps the *first* diagnostic for any input byte-identical
/// to the pre-#864 single diagnostic. Do not sort this list by span.
///
/// Like `into_result`, this ignores `unsupported_syntax_errors()` (version-
/// gated syntax): a module with only those still parses and reaches HIR.
///
/// A module that parses has its top-level string annotations replaced by
/// the expressions they contain (`def f() -> "C"` reads as `-> C`; Part 1
/// of #889), so every later stage sees one spelling of an annotation. See
/// [`pycc_ast::normalize_string_annotations`] for exactly what is rewritten.
pub fn parse_all(source: &str) -> Result<ModModule, Vec<Diagnostic>> {
    let parsed = ruff_python_parser::parse_unchecked(source, ParseOptions::from(Mode::Module))
        // `Mode::Module` always produces a `Mod::Module`, so the `None`
        // arm of `try_into_module` is unreachable here; `expect` keeps that
        // arm inside `core` rather than as an uncoverable in-crate region
        // (D-014's 100%-region gate).
        .try_into_module()
        .expect("Mode::Module always yields a module");
    if parsed.has_valid_syntax() {
        let mut module = parsed.into_syntax();
        pycc_ast::normalize_string_annotations(&mut module, source);
        return Ok(module);
    }
    Err(parsed.into_errors().into_iter().map(to_l0001).collect())
}

/// First-error view of [`parse_all`]: parses `source` and reports only the
/// first syntax error ruff found, byte-identical to the pre-#864 behaviour
/// (`parse(s).unwrap_err() == parse_all(s).unwrap_err()[0]` for every `s`).
///
/// Kept for the many callers (test modules, benches) that only need to know
/// whether a module parses; the driver uses [`parse_all`].
pub fn parse(source: &str) -> Result<ModModule, Diagnostic> {
    parse_all(source).map_err(|mut all| all.swap_remove(0))
}

/// Maps one ruff `ParseError` to pycc's `L0001` diagnostic. The message is
/// ruff's own `Display` output, which embeds the byte range
/// (`... at byte range 9..10`); that text is part of today's snapshot
/// fixtures and stays as is.
fn to_l0001(error: ruff_python_parser::ParseError) -> Diagnostic {
    let span = Span::new(
        error.location.start().to_u32(),
        error.location.end().to_u32(),
    );
    Diagnostic::error("L0001", error.to_string(), span)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Five syntax errors whose ruff discovery order is *not* source order:
    /// entry 1 (`5..10`, "Invalid assignment target") starts before entry 0
    /// (`11..12`, "Expected `)`"). Measured against ruff_python_parser 0.0.6.
    const UNSORTED: &str = "x = (1 +\ny = 2\n  z = 3\ndef f():\nreturn 1\n";

    #[test]
    fn parses_a_function_returning_none_that_calls_print() {
        let module = parse("def main() -> None:\n    print(42)\n").expect("should parse");
        assert_eq!(module.body.len(), 1);
    }

    #[test]
    fn a_string_annotation_reaches_callers_unquoted() {
        let module = parse("def f(a: \"int\") -> None:\n    pass\n").expect("should parse");
        let def = module.body[0].as_function_def_stmt().unwrap();
        let annotation = def.parameters.args[0].parameter.annotation.as_deref();
        assert!(annotation.unwrap().is_name_expr());
    }

    #[test]
    fn parses_top_level_statements_with_no_main() {
        let module = parse("print(42)\n").expect("should parse");
        assert_eq!(module.body.len(), 1);
    }

    #[test]
    fn syntax_error_becomes_an_l0001_diagnostic() {
        let err = parse("def main(:\n").unwrap_err();
        assert_eq!(err.code, "L0001");
    }

    #[test]
    fn syntax_error_carries_the_real_byte_span_not_a_placeholder() {
        let err = parse("def main(:\n").unwrap_err();
        // "def main(:\n" -- ruff's parser fails at the malformed parameter
        // list; this must no longer be Span::new(0, 0) for every input
        // regardless of where the error actually is.
        assert_ne!(err.span, Some(Span::new(0, 0)));
    }

    #[test]
    fn parse_all_ok_for_valid_source() {
        let module = parse_all("def main() -> None:\n    print(42)\n").expect("should parse");
        assert_eq!(module.body.len(), 1);
    }

    #[test]
    fn parse_all_reports_every_syntax_error_in_discovery_order() {
        let all = parse_all(UNSORTED).unwrap_err();
        // Exactly five: a `take(n)` truncation or a de-duplication would
        // change this count.
        assert_eq!(all.len(), 5);
        assert!(all.iter().all(|d| d.code == "L0001"));
        // Discovery order, verbatim: a span sort would put `5..10` first,
        // a reverse sort would put `32..38` first.
        assert_eq!(all[0].span, Some(Span::new(11, 12)));
        assert_eq!(all[1].span, Some(Span::new(5, 10)));
        assert_eq!(all[4].span, Some(Span::new(32, 38)));
    }

    #[test]
    fn parse_first_error_is_parse_all_first_entry() {
        for source in ["def main(:\n", UNSORTED] {
            assert_eq!(
                parse(source).unwrap_err(),
                parse_all(source).unwrap_err()[0],
                "first diagnostic must stay byte-identical for {source:?}"
            );
        }
    }

    #[test]
    fn parse_all_single_error_is_one_entry() {
        let all = parse_all("x = \n").unwrap_err();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].message, "Expected an expression at byte range 4..5");
    }

    #[test]
    fn parse_all_zero_width_eof_error_keeps_its_span() {
        let all = parse_all("def f(").unwrap_err();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].span, Some(Span::new(6, 6)));
    }
}
