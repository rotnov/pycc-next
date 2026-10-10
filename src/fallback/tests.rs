//! Unit tests for the non-strict fallback plan (#12).

use super::names::{Rewriter, bound_names, module_references, opaque_bound_names};
use super::*;
use pycc_diag::Span;

const NATIVE: Publishes = Publishes {
    functions: true,
    classes: false,
};

fn stmts(source: &str) -> Vec<Stmt> {
    pycc_parser::parse_all(source)
        .expect("parses")
        .body
        .into_iter()
        .collect()
}

fn set(names: &[&str]) -> BTreeSet<String> {
    names.iter().map(|n| n.to_string()).collect()
}

fn failure_at(start: u32) -> Failure {
    Failure {
        diagnostic: Diagnostic::error("C0001", "a `lambda`", Span::new(start, start + 1)),
        origin: None,
    }
}

#[test]
fn a_python_string_literal_escapes_what_it_must() {
    assert_eq!(
        py_str("a\"b\\c\n\t\r\x01é"),
        "\"a\\\"b\\\\c\\n\\t\\r\\x01é\""
    );
}

#[test]
fn bound_names_cover_every_binding_form_of_one_scope() {
    let body = stmts(
        "import os.path, json as j\nfrom m import a, b as c\nx, (y, *z) = 1, (2, 3)\n\
         for i in r: pass\nwith o as w: pass\ntry:\n    pass\nexcept E as e:\n    pass\n\
         def f(p):\n    inner = 1\nclass K:\n    attr = 1\nglobal g\nq = [t for t in r]\n\
         match v:\n    case [h, *rest]: pass\n    case {**kw}: pass\n",
    );
    assert_eq!(
        bound_names(&body),
        set(&[
            "K", "a", "c", "e", "f", "g", "h", "i", "j", "kw", "os", "q", "rest", "w", "x", "y",
            "z"
        ])
    );
}

#[test]
fn opaque_names_are_the_ones_a_subscript_cannot_replace() {
    let body = stmts("if c:\n    import json\n    x = 1\n    def f(): pass\n");
    assert_eq!(opaque_bound_names(&body[0]), set(&["f", "json"]));
}

#[test]
fn module_references_follow_python_scoping() {
    let body = stmts(
        "def f(a):\n    b = a + g\n    global h\n    h = [b for b in k]\n    return lambda c: c + m\n",
    );
    assert_eq!(module_references(&body[0]), set(&["g", "h", "k", "m"]));
    let class = stmts("class C:\n    x = y\n    def m(self):\n        return x\n");
    assert_eq!(module_references(&class[0]), set(&["x", "y"]));
}

#[test]
fn the_rewriter_redirects_module_globals_only() {
    let source = "def f(n):\n    global t, u\n    t = n + t + v\n    w = 1\n    return w\n";
    let body = stmts(source);
    let globals = set(&["t", "v", "w"]);
    let mut rewriter = Rewriter::new(&globals, "NS");
    pycc_ast::visitor::Visitor::visit_stmt(&mut rewriter, &body[0]);
    let mut edits: Vec<(String, String)> = rewriter
        .edits
        .iter()
        .map(|e| {
            (
                source[e.range.start as usize..e.range.end as usize].to_string(),
                e.text.clone(),
            )
        })
        .collect();
    edits.sort();
    assert_eq!(
        edits,
        [
            ("global t, u".to_string(), "global u".to_string()),
            ("t".to_string(), "NS['t']".to_string()),
            ("t".to_string(), "NS['t']".to_string()),
            ("v".to_string(), "NS['v']".to_string()),
        ]
    );
    let only = set(&["t"]);
    let mut rewriter = Rewriter::new(&only, "NS");
    pycc_ast::visitor::Visitor::visit_stmt(
        &mut rewriter,
        &stmts("def g():\n    global t\n    t = 1\n")[0],
    );
    assert!(rewriter.edits.iter().any(|e| e.text == "pass"));
}

#[test]
fn the_rewriter_skips_comprehension_targets_and_annotations() {
    let source = "x: T = [x for x in x]\n";
    let globals = set(&["x", "T"]);
    let mut rewriter = Rewriter::new(&globals, "NS");
    pycc_ast::visitor::Visitor::visit_stmt(&mut rewriter, &stmts(source)[0]);
    let starts: Vec<u32> = rewriter.edits.iter().map(|e| e.range.start).collect();
    // The target `x` and the outermost iterable `x`; not `T`, not the
    // comprehension's own `x`s.
    assert_eq!(starts.len(), 2, "{:?}", rewriter.edits);
    assert!(starts.contains(&0));
    assert!(starts.contains(&(source.rfind('x').unwrap() as u32)));
}

const PROGRAM: &str = "\"\"\"doc\"\"\"\nfrom __future__ import annotations\n\
def helper(x):\n    return x + 1\n\n\
def lam(v):\n    return (lambda: helper(v))()\n\n\
count = 0\n\
print(lam(count))\n";

#[test]
fn a_plan_renders_fallback_code_as_an_exec_in_the_namespace() {
    let mut plan = Plan::new(PROGRAM, "m.py", NATIVE).expect("parses");
    let lam = PROGRAM.find("lambda").unwrap() as u32;
    assert!(plan.absorb(None, &[failure_at(lam)]));
    let rendered = plan.render();
    let text = &rendered.text;
    assert!(
        text.starts_with("\"\"\"doc\"\"\"\nfrom __future__ import annotations\nimport sys as "),
        "{text}"
    );
    assert!(
        text.contains("__pycc_fallback_builtins.exec(__pycc_fallback_builtins.compile(\"\\n\\n\\n\\n\\ndef lam(v):"),
        "{text}"
    );
    // `count` is read by nothing in fallback code; `lam` is fallback-bound.
    assert!(
        text.contains("print(__pycc_fallback_ns['lam'](count))"),
        "{text}"
    );
    assert!(text.contains("def helper(x):"), "{text}");
    // Every line after the preamble keeps its number.
    assert_eq!(text.lines().count(), PROGRAM.lines().count() + 1);
    let report = plan.report_lines();
    assert_eq!(report.len(), 1);
    assert!(report[0].starts_with("fallback: function `lam` at m.py:6-7 runs in CPython: C0001"));
    // A second failure in the same statement adds nothing.
    let at = rendered.ranges[3].start;
    assert!(!plan.absorb(Some(&rendered), &[failure_at(at)]));
}

#[test]
fn a_compiled_definition_fallback_code_cannot_see_falls_back_too() {
    let source = "class K:\n    pass\n\ndef _h():\n    return 1\n\ndef f():\n    return (lambda: (K, _h()))()\n";
    let mut plan = Plan::new(source, "m.py", NATIVE).expect("parses");
    plan.absorb(None, &[failure_at(source.find("lambda").unwrap() as u32)]);
    let report = plan.report_lines().join("\n");
    assert!(report.contains("class `K`"), "{report}");
    assert!(report.contains("function `_h`"), "{report}");
    let ext = Publishes {
        functions: true,
        classes: true,
    };
    let mut plan = Plan::new(source, "m.py", ext).expect("parses");
    plan.absorb(None, &[failure_at(source.find("lambda").unwrap() as u32)]);
    let report = plan.report_lines().join("\n");
    assert!(!report.contains("class `K`"), "{report}");
    assert!(report.contains("function `_h`"), "{report}");
}

#[test]
fn a_compiled_import_or_handler_binding_a_dict_global_falls_back() {
    let source = "import json\ntry:\n    pass\nexcept E as e:\n    pass\n\
                  def f():\n    return (lambda: (json, e))()\n";
    let mut plan = Plan::new(source, "m.py", NATIVE).expect("parses");
    plan.absorb(None, &[failure_at(source.find("lambda").unwrap() as u32)]);
    let report = plan.report_lines().join("\n");
    assert!(report.contains("statement at m.py:1 "), "{report}");
    assert!(report.contains("statements at m.py:2-5 "), "{report}");
}

/// A wildcard import's names are unknown, so fallback code reading a name
/// no statement binds takes the import with it.
#[test]
fn a_wildcard_import_falls_back_with_code_reading_an_unbound_name() {
    let source = "from itertools import *\nx = 1\ny = list(accumulate([x]))\n";
    let mut plan = Plan::new(source, "m.py", NATIVE).expect("parses");
    plan.absorb(None, &[failure_at(source.find("list").unwrap() as u32)]);
    let report = plan.report_lines().join("\n");
    assert!(report.contains("statement at m.py:1 "), "{report}");
    assert!(!report.contains("m.py:2"), "{report}");
}

#[test]
fn a_failure_no_statement_owns_falls_back_everything() {
    let mut plan = Plan::new("x = 1\ny = 2\n", "m.py", NATIVE).expect("parses");
    let failure = Failure {
        diagnostic: Diagnostic::error("T0021", "m", Span::new(0, 0)),
        origin: None,
    };
    assert!(plan.absorb(None, &[failure]));
    assert_eq!(plan.report_lines().len(), 2);
    let rendered = plan.render();
    assert!(
        rendered.text.contains("compile(\"x = 1\\ny = 2\\n\""),
        "{}",
        rendered.text
    );
    // Placed by the type checker's item origin instead of a span.
    let mut plan = Plan::new("x = 1\ny = f()\n", "m.py", NATIVE).expect("parses");
    let keyed = Failure {
        diagnostic: Diagnostic::error("T0021", "m", Span::new(0, 0)),
        origin: Some(6),
    };
    plan.absorb(None, &[keyed]);
    assert_eq!(plan.report_lines().len(), 1);
}

#[test]
fn only_syntax_and_artifact_refusals_cannot_fall_back() {
    let span = Span::new(0, 1);
    for code in ["L0001", "L0002", "I0402", "I0403", "I0405"] {
        assert!(
            !can_fall_back(&Diagnostic::error(code, "m", span)),
            "{code}"
        );
    }
    for code in ["C0001", "C0002", "T0021", "I0404", "E0100"] {
        assert!(can_fall_back(&Diagnostic::error(code, "m", span)), "{code}");
    }
    assert!(Plan::new("def f(:\n", "m.py", NATIVE).is_none());
}
