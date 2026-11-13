//! The generic arm of a region call: one shared, cold function per shape
//! of call in place of the call written out at every site.
//!
//! A copy takes a region call's generic arm only after a member was rebound
//! while a region call was running (docs/design/selected-regions.md,
//! section 4.4): it calls whatever the callee's name holds, through the
//! object protocol, with its native arguments boxed, and converts the
//! result to what the copy needs. Written out at each of a region's call
//! sites, that is most of a copy's code, run by no measured workload. So a
//! site calls a *slow call* instead, which takes the callee, the text its
//! conversion's `TypeError` starts with and the site's arguments, as the
//! site passes them:
//!
//! ```python
//! def __pycc_rslow0(f: object, what: str, a0: object, a1: int) -> int:
//!     return __pycc_rconv_i(f(a0, a1), what)
//! ```
//!
//! One serves every site whose arguments have the same kinds and keywords
//! and whose result is converted alike ([`Shape`]); for an unpack, the
//! slow call unpacks CPython's result and converts its `int` items
//! ([`Form::Unpack`]), and its caller takes its multi-value twin. The
//! callee is evaluated at the site, before the arguments, as CPython
//! evaluates it. A slow call has no frame of its own
//! (`pycc_codegen::frames`): what it raises is raised at its site, and
//! codegen keeps it out of line (`cold`, `noinline`).

use super::super::super::build::{call, load, string};
use super::super::super::signatures::parse_helper;
use ruff_python_ast::{Expr, ExprCall, Stmt};
use ruff_text_size::TextRange;
use std::cell::RefCell;
use std::collections::BTreeMap;

/// The prefix of a slow call's name (`pycc_mir::unreferenced` and
/// `pycc_codegen::frames` know it).
pub const SLOW_PREFIX: &str = "__pycc_rslow";

/// What a slow call does with the callee's result.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum Form {
    /// Returns it, converted by `__pycc_rconv_<shape>` (`None`: an object,
    /// returned as it is).
    Value(Option<String>),
    /// Unpacks it into one item per entry, converting each `int` item
    /// (`true`) by `__pycc_rconv_i`.
    Unpack(Vec<bool>),
}

/// A slow call's key: per argument of the site, in source order, whether it
/// is an `int` and its keyword (`None`: positional); the result's
/// annotation; and the [`Form`].
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct Shape {
    args: Vec<(bool, Option<String>)>,
    result: String,
    form: Form,
}

/// The slow calls a unit's copies use, by shape.
#[derive(Debug, Default)]
pub(super) struct Slows(RefCell<BTreeMap<Shape, String>>);

impl Slows {
    /// The call of the slow call for `site`, whose arguments are `ints`'
    /// kinds, converting its callee's result as `form` says to `result`
    /// (an annotation); `what` starts the conversion's `TypeError`
    /// (`skip() returned `).
    pub(super) fn call(&self, site: &ExprCall, ints: &[bool], result: &str, form: Form, what: &str) -> Expr {
        let args: Vec<(bool, Option<String>)> = site
            .arguments
            .args
            .iter()
            .map(|_| None)
            .chain(site.arguments.keywords.iter().map(|k| k.arg.as_ref().map(|a| a.to_string())))
            .zip(ints)
            .map(|(keyword, int)| (*int, keyword))
            .collect();
        let converts = !matches!(form, Form::Value(None));
        let shape = Shape { args, result: result.to_string(), form };
        let mut slows = self.0.borrow_mut();
        let next = slows.len();
        let name = slows.entry(shape).or_insert_with(|| format!("{SLOW_PREFIX}{next}")).clone();
        let at = super::rewrite::added(site);
        let mut operands = vec![(*site.func).clone()];
        if converts {
            operands.push(string(what, at));
        }
        operands.extend(site.arguments.args.iter().cloned());
        operands.extend(site.arguments.keywords.iter().map(|k| k.value.clone()));
        call(load(&name, at), operands, site.range)
    }

    /// The definitions of the slow calls made.
    pub(super) fn defs(&self) -> Vec<Stmt> {
        let mut out = Vec::new();
        for (shape, name) in self.0.borrow().iter() {
            out.extend(parse_helper(&def(name, shape), TextRange::default()));
        }
        out
    }
}

/// The source of the slow call `name` of `shape`.
fn def(name: &str, shape: &Shape) -> String {
    let converts = !matches!(shape.form, Form::Value(None));
    let mut params = vec!["f: object".to_string()];
    if converts {
        params.push("what: str".to_string());
    }
    let mut passed = Vec::new();
    for (i, (int, keyword)) in shape.args.iter().enumerate() {
        params.push(format!("a{i}: {}", if *int { "int" } else { "object" }));
        passed.push(match keyword {
            Some(k) => format!("{k}=a{i}"),
            None => format!("a{i}"),
        });
    }
    let made = format!("f({})", passed.join(", "));
    let body = match &shape.form {
        Form::Value(None) => format!("    return {made}\n"),
        Form::Value(Some(conv)) => format!("    return __pycc_rconv_{conv}({made}, what)\n"),
        Form::Unpack(ints) => {
            let items: Vec<String> = (0..ints.len()).map(|i| format!("r{i}")).collect();
            let converted: Vec<String> = ints
                .iter()
                .enumerate()
                .map(|(i, int)| {
                    if *int {
                        format!("__pycc_rconv_i(r{i}, what + 'a tuple whose item {i} is ')")
                    } else {
                        format!("r{i}")
                    }
                })
                .collect();
            format!("    {} = {made}\n    return {}\n", items.join(", "), converted.join(", "))
        }
    };
    format!("def {name}({}) -> {}:\n{body}", params.join(", "), shape.result)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn site(source: &str) -> ExprCall {
        let parsed = ruff_python_parser::parse_expression(source).unwrap().into_expr();
        let Expr::Call(call) = parsed else { panic!("a call") };
        call
    }

    #[test]
    fn sites_of_one_shape_share_a_slow_call() {
        let slows = Slows::default();
        let a = site("ns['skip'](src, pos, chars=WS)");
        let b = site("ns['other'](s, p, chars=X)");
        let c = site("ns['skip'](src, pos)");
        let form = || Form::Value(Some("i".to_string()));
        let made: Vec<Expr> = [&a, &b]
            .iter()
            .map(|s| slows.call(s, &[false, true, false], "int", form(), "skip() returned "))
            .collect();
        // The callee first, then the text, then the site's arguments.
        for (call, from) in made.iter().zip([&a, &b]) {
            let Expr::Call(call) = call else { panic!("a call") };
            assert!(matches!(call.func.as_ref(), Expr::Name(n) if n.id.as_str() == "__pycc_rslow0"));
            assert_eq!(call.arguments.args[0], *from.func);
            assert!(matches!(&call.arguments.args[1], Expr::StringLiteral(s) if s.value.to_str() == "skip() returned "));
            assert_eq!(call.arguments.args.len(), 5);
            assert!(call.arguments.keywords.is_empty());
        }
        slows.call(&c, &[false, true], "int", form(), "skip() returned ");
        slows.call(&c, &[false, true], "tuple[int, object]", Form::Unpack(vec![true, false]), "f() returned ");
        let Expr::Call(plain) = slows.call(&c, &[false, true], "object", Form::Value(None), "") else {
            panic!("a call")
        };
        // An object result takes no conversion, and no text.
        assert_eq!(plain.arguments.args.len(), 3);
        let mut defs: Vec<String> = slows.0.borrow().iter().map(|(shape, name)| def(name, shape)).collect();
        defs.sort();
        assert_eq!(
            defs,
            [
                "def __pycc_rslow0(f: object, what: str, a0: object, a1: int, a2: object) -> int:\n    return __pycc_rconv_i(f(a0, a1, chars=a2), what)\n",
                "def __pycc_rslow1(f: object, what: str, a0: object, a1: int) -> int:\n    return __pycc_rconv_i(f(a0, a1), what)\n",
                "def __pycc_rslow2(f: object, what: str, a0: object, a1: int) -> tuple[int, object]:\n    r0, r1 = f(a0, a1)\n    return __pycc_rconv_i(r0, what + 'a tuple whose item 0 is '), r1\n",
                "def __pycc_rslow3(f: object, a0: object, a1: int) -> object:\n    return f(a0, a1)\n",
            ]
        );
    }
}
