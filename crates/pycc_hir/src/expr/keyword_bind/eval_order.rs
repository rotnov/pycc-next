//! Whether binding a keyword call would observably change the order its
//! argument values are evaluated in (issue #1204).
//!
//! [`super::bind_keyword_arguments`] returns one positional vector in
//! *parameter* order, and `HirExpr::Call` evaluates that vector left to
//! right. CPython evaluates a call's argument values in *source* order, so a
//! keyword call whose values land out of parameter order would run their
//! side effects in the wrong order once bound. This module decides which
//! calls that can happen to, so [`super::is_bindable_call`] can keep them on
//! the unchanged `C0001` rejection. The rule itself is stated once, in
//! `docs/TYPE_SYSTEM.md` ("Keyword argument evaluation order").

use pycc_ast::ExprCall;

use super::Signature;
use crate::expr::unobservable::is_unobservable;

/// Whether binding `call` against `signature` would move a value whose
/// evaluation could be observed.
///
/// Answers `false` whenever a keyword names no parameter, names a
/// positional-only one, or names a parameter already supplied: the binder
/// reports each of those as CPython's own `TypeError` (`T0021`), and that
/// diagnostic must not be displaced by the capability rejection.
pub(super) fn observably_reorders(signature: &Signature, call: &ExprCall) -> bool {
    let positional_len = call.arguments.args.len();
    let mut supplied = vec![false; signature.names.len()];
    supplied
        .iter_mut()
        .take(positional_len)
        .for_each(|slot| *slot = true);
    let mut previous: Option<usize> = None;
    let mut in_parameter_order = true;
    for keyword in &call.arguments.keywords {
        let Some(index) = keyword.arg.as_ref().and_then(|name| {
            signature
                .names
                .iter()
                .position(|param| param == name.as_str())
        }) else {
            return false;
        };
        if index < signature.posonly_count || supplied[index] {
            return false;
        }
        supplied[index] = true;
        if previous.is_some_and(|previous| index < previous) {
            in_parameter_order = false;
        }
        previous = Some(index);
    }
    !in_parameter_order
        && !call
            .arguments
            .args
            .iter()
            .chain(call.arguments.keywords.iter().map(|keyword| &keyword.value))
            .all(is_unobservable)
}

#[cfg(test)]
mod tests {
    const DEF_ABC: &str = "def f(a: int, b: int = 2, c: int = 3) -> None:\n    print(a + b + c)\n\ndef g(n: int) -> int:\n    return n\n\n";

    fn lower(source: &str) -> Result<crate::HirModule, pycc_diag::Diagnostic> {
        let module = pycc_parser::parse(source).expect("test fixture must parse");
        crate::lower_checked(&module)
    }

    #[test]
    fn an_out_of_order_call_with_an_observable_value_keeps_the_capability_rejection() {
        for call in [
            "f(b=g(2), a=g(1))",
            "f(b=2, a=g(1))",
            "f(c=1, b=g(2), a=1)",
            "f(g(1), c=1, b=2)",
            "f(b=-g(2), a=1)",
            "f(a=g(1), c=g(3), b=2)",
        ] {
            let source = format!("{DEF_ABC}{call}\n");
            let diagnostic = lower(&source).expect_err(call);
            assert_eq!(diagnostic.code, "C0001", "call: {call}");
            assert_eq!(
                diagnostic.message, "keyword call arguments are not supported yet",
                "call: {call}"
            );
            let start = u32::try_from(source.rfind(call).expect("fixture holds the call"))
                .expect("fixture is short");
            let end = start + u32::try_from(call.len()).expect("call is short");
            assert_eq!(
                diagnostic.span,
                Some(pycc_diag::Span::new(start, end)),
                "call: {call}"
            );
            assert_eq!(diagnostic.help, None, "call: {call}");
        }
    }

    #[test]
    fn an_in_order_or_unobservable_call_still_binds() {
        for call in [
            "f(a=g(1), b=g(2))",
            "f(g(1), b=g(2))",
            "f(a=g(1), c=g(3))",
            "f(b=2, a=1)",
            "f(c=-3, b=x, a=1)",
            "f(c=3, b=x, a=x)",
            "f(c=None, b=True, a='s')",
        ] {
            // HIR lowering does no type checking, so `Ok` here means the
            // call was bound rather than rejected with `C0001`.
            lower(&format!("{DEF_ABC}x = 1\n{call}\n")).expect(call);
        }
    }

    #[test]
    fn a_keyword_the_binder_rejects_keeps_its_type_error_even_out_of_order() {
        for (call, message) in [
            (
                "f(b=g(2), d=g(1))",
                "`f` got an unexpected keyword argument `d`",
            ),
            (
                "f(1, b=g(2), a=g(1))",
                "`f` got multiple values for argument `a`",
            ),
        ] {
            let diagnostic = lower(&format!("{DEF_ABC}{call}\n")).expect_err(call);
            assert_eq!(diagnostic.code, "T0021", "call: {call}");
            assert_eq!(diagnostic.message, message, "call: {call}");
        }
        let diagnostic = lower(
            "def h(a: int, /, b: int) -> None:\n    print(a + b)\n\ndef g(n: int) -> int:\n    return n\n\nh(b=g(2), a=g(1))\n",
        )
        .expect_err("positional-only keyword");
        assert_eq!(diagnostic.code, "T0021");
        assert_eq!(
            diagnostic.message,
            "`h` got positional-only parameter `a` passed as a keyword argument"
        );
    }
}
