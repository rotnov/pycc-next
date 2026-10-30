//! Unit tests for the non-strict fallback plan (#12).

use super::names::{Rewriter, bound_names, module_references, opaque_bound_names};
use super::*;
use pycc_diag::Span;

const NATIVE: Publishes = Publishes {
    functions: true,
    classes: true,
    infers_unannotated: true,
    variables: false,
    foreign_relative_imports: false,
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
def helper(x: int) -> int:\n    return x + 1\n\n\
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
    assert!(text.contains("def helper(x: int) -> int:"), "{text}");
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
    let source = "class K:\n    pass\n\ndef _h():\n    return 1\n\ndef f():\n    return (lambda: (K, _h()))()\n\n\
                  class L:\n    def m(self):\n        return 1\n";
    // A native build publishes `K` (issue #8), but not `L`'s unannotated
    // method, and an `L` may reach the fallback code through any value.
    let mut plan = Plan::new(source, "m.py", NATIVE).expect("parses");
    plan.absorb(None, &[failure_at(source.find("lambda").unwrap() as u32)]);
    let report = plan.report_lines().join("\n");
    assert!(!report.contains("class `K`"), "{report}");
    assert!(report.contains("class `L` at m.py:10 runs in CPython"), "{report}");
    assert!(report.contains("function `_h`"), "{report}");
    let unpublished = Publishes { classes: false, ..NATIVE };
    let mut plan = Plan::new(source, "m.py", unpublished).expect("parses");
    plan.absorb(None, &[failure_at(source.find("lambda").unwrap() as u32)]);
    let report = plan.report_lines().join("\n");
    assert!(report.contains("class `K`"), "{report}");
    let ext = Publishes {
        functions: true,
        classes: true,
        infers_unannotated: false,
        variables: false,
        foreign_relative_imports: false,
    };
    let mut plan = Plan::new(source, "m.py", ext).expect("parses");
    plan.absorb(None, &[failure_at(source.find("lambda").unwrap() as u32)]);
    let report = plan.report_lines().join("\n");
    assert!(!report.contains("class `K`"), "{report}");
    assert!(!report.contains("class `L`"), "{report}");
    assert!(report.contains("function `_h`"), "{report}");
    // Without fallback code no class falls back.
    let mut plan = Plan::new(source, "m.py", NATIVE).expect("parses");
    plan.cascade();
    assert!(plan.report_lines().is_empty());
}

#[test]
fn a_native_build_publishes_a_class_only_with_members_python_code_can_use() {
    let unpublished = |class: &str| {
        let source = format!("class C:\n{class}\n");
        members::with_unpublished_members(&stmts(&source)).contains("C")
    };
    for published in [
        "    pass",
        "    \"doc\"\n    n = 1\n    m: int = -2\n    s = 'x'\n    t: str",
        "    def __init__(self, x: int) -> None:\n        self.x = x",
        "    def f(self, a: int, /, b: str) -> int:\n        return a",
        "    @staticmethod\n    def g(a: int) -> int:\n        return a",
        "    @classmethod\n    def h(cls) -> int:\n        return 1",
        "    @property\n    def p(self) -> int:\n        return 1",
        "    def __repr__(self) -> str:\n        return ''\n    def __lt__(self, o: 'C') -> bool:\n        return True",
    ] {
        assert!(!unpublished(published), "{published}");
    }
    for hidden in [
        "    n = [1]",
        "    if True:\n        pass",
        "    def f(self, a):\n        return a",
        "    def f(self, a: int):\n        return a",
        "    def f(self, a: int = 1) -> int:\n        return a",
        "    def f(self, *a: int) -> int:\n        return 1",
        "    def f(self, *, a: int) -> int:\n        return a",
        "    def f(self, **a: int) -> int:\n        return 1",
        "    def _f(self) -> int:\n        return 1",
        "    def __len__(self) -> int:\n        return 1",
        "    @functools.cache\n    def f(self) -> int:\n        return 1",
        "    @staticmethod\n    def g(a) -> int:\n        return 1",
        "    def __init__(self, x) -> None:\n        self.x = x",
    ] {
        assert!(unpublished(hidden), "{hidden}");
    }
    // The class statement itself, and a class built on one hiding members.
    let classes = members::with_unpublished_members(&stmts(
        "@dataclass\nclass D:\n    x: int\n\n\
         class E(Enum):\n    A = 1\n\n\
         class M(metaclass=Meta):\n    pass\n\n\
         class H:\n    def _f(self) -> int:\n        return 1\n\n\
         class S(H):\n    pass\n\n\
         class B:\n    pass\n\n\
         class Ok(B, object):\n    pass\n\n\
         class Err(ValueError):\n    pass\n\n\
         class Later(Undefined):\n    pass\n",
    ));
    assert_eq!(
        classes.into_iter().collect::<Vec<_>>(),
        ["D", "E", "H", "Later", "M", "S"]
    );
}

#[test]
fn a_compiled_handler_binding_a_dict_global_falls_back_and_an_import_is_mirrored() {
    let source = "import json\ntry:\n    pass\nexcept E as e:\n    pass\n\
                  def f():\n    return (lambda: (json, e))()\n";
    let mut plan = Plan::new(source, "m.py", NATIVE).expect("parses");
    plan.absorb(None, &[failure_at(source.find("lambda").unwrap() as u32)]);
    let report = plan.report_lines().join("\n");
    assert!(!report.contains("statement at m.py:1 "), "{report}");
    assert!(report.contains("statements at m.py:2-5 "), "{report}");
    // The import stays compiled and runs in CPython too, on its own line.
    let text = plan.render().text;
    assert!(
        text.contains(
            "import json; __pycc_fallback_builtins.exec(__pycc_fallback_builtins.compile(\"import json\""
        ),
        "{text}"
    );
    assert_eq!(text.lines().count(), source.lines().count() + 1, "{text}");
}

/// An alias of an attribute chain (`input = sys.stdin.readline`) that
/// fallback code reads is mirrored like the import it reads; an import of
/// the program's own module, or a name bound twice, is not.
#[test]
fn an_alias_and_its_import_are_mirrored_unless_rebound_or_local() {
    let source = "import sys\ninput = sys.stdin.readline\n\
                  def f():\n    return (lambda: input())()\n";
    let mut plan = Plan::new(source, "m.py", NATIVE).expect("parses");
    plan.absorb(None, &[failure_at(source.find("lambda").unwrap() as u32)]);
    assert_eq!(plan.report_lines().len(), 1, "{:?}", plan.report_lines());
    assert_eq!(plan.mirrored, BTreeSet::from([0, 1]));
    let rebound = "import sys\ninput = sys.stdin.readline\ninput = 3\n\
                   def f():\n    return (lambda: input())()\n";
    let mut plan = Plan::new(rebound, "m.py", NATIVE).expect("parses");
    plan.absorb(None, &[failure_at(rebound.find("lambda").unwrap() as u32)]);
    assert!(!plan.mirrored.contains(&1));
    let dir = pycc_scratch::ScratchDir::new("fallback_local_import").expect("scratch");
    std::fs::write(dir.join("helper.py"), "X = 1\n").expect("write");
    let local = "import helper\ndef f():\n    return (lambda: helper.X)()\n";
    let mut plan = Plan::new(local, "m.py", NATIVE).expect("parses");
    plan.set_entry_dir(Some(dir.join("")));
    plan.absorb(None, &[failure_at(local.find("lambda").unwrap() as u32)]);
    assert!(plan.mirrored.is_empty());
    assert!(plan.report_lines().join("\n").contains("statement at m.py:1 "));
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

fn builtin_failure(name: &str, start: u32) -> Failure {
    Failure {
        diagnostic: Diagnostic::error(
            "C0001",
            format!("call to builtin `{name}` is valid Python but not implemented yet"),
            Span::new(start, start + 1),
        ),
        origin: None,
    }
}

/// A builtin call pycc does not compile becomes a call through CPython's
/// `builtins` module; one that reads its caller's frame falls back.
#[test]
fn an_unsupported_builtin_call_goes_through_cpython_builtins() {
    let source = "a = 1\nb = min(a, 2)\nc = vars()\n";
    let mut plan = Plan::new(source, "m.py", NATIVE).expect("parses");
    let min = source.find("min").unwrap() as u32;
    assert!(plan.absorb(None, &[builtin_failure("min", min)]));
    assert!(plan.report_lines().is_empty());
    let rendered = plan.render();
    assert!(
        rendered.text.contains("b = __pycc_fallback_builtins.min(a, 2)"),
        "{}",
        rendered.text
    );
    // The same call failing again (in the rewritten text) is not new.
    let again = rendered.text.find("__pycc_fallback_builtins.min").unwrap() as u32;
    assert!(!plan.object_builtins.contains_key(&again));
    let vars = rendered.text.find("vars").unwrap() as u32;
    assert!(plan.absorb(Some(&rendered), &[builtin_failure("vars", vars)]));
    assert!(plan.report_lines()[0].contains("m.py:3 "), "{:?}", plan.report_lines());
}

/// A global fallback code only reads stays native in compiled code, with
/// every compiled store copied into the namespace; one fallback code binds,
/// or that a `for` target binds, lives in the namespace; a copy that fails
/// to compile demotes its global.
#[test]
fn a_global_fallback_code_only_reads_stays_native_with_copied_stores() {
    let source = "n = 1\nk = 0\nfor i in range(3):\n    n += i\n\
                  def f():\n    global k\n    k = (lambda: n + i)()\n";
    let mut plan = Plan::new(source, "m.py", NATIVE).expect("parses");
    plan.absorb(None, &[failure_at(source.find("lambda").unwrap() as u32)]);
    assert_eq!(plan.native_globals(), set(&["n"]));
    let rendered = plan.render();
    let text = &rendered.text;
    assert!(text.contains("n = 1; __pycc_fallback_ns['n'] = n\n"), "{text}");
    assert!(
        text.contains("    n += __pycc_fallback_ns['i']; __pycc_fallback_ns['n'] = n\n"),
        "{text}"
    );
    assert!(text.contains("__pycc_fallback_ns['k'] = 0"), "{text}");
    assert!(text.contains("for __pycc_fallback_ns['i'] in"), "{text}");
    let copy = text.find("; __pycc_fallback_ns['n'] = n").unwrap() as u32 + 2;
    let failure = Failure {
        diagnostic: Diagnostic::error("I0404", "item store", Span::new(copy, copy + 1)),
        origin: None,
    };
    assert!(plan.absorb(Some(&rendered), &[failure]));
    assert!(plan.native_globals().is_empty());
    assert!(plan.render().text.contains("__pycc_fallback_ns['n'] = 1"));
}

/// An unannotated public function fallback code calls is compiled with
/// object annotations instead of falling back.
#[test]
fn an_inferred_function_fallback_code_calls_is_compiled_with_objects() {
    let source = "def f(a, b: int):\n    return a\n\nx = (lambda: f(2, 1))()\n";
    let mut plan = Plan::new(source, "m.py", NATIVE).expect("parses");
    plan.absorb(None, &[failure_at(source.find("lambda").unwrap() as u32)]);
    assert_eq!(plan.report_lines().len(), 1, "{:?}", plan.report_lines());
    let text = plan.render().text;
    assert!(text.contains("def f(a: object, b: int) -> object:"), "{text}");
    // With a parameter default it falls back instead (#1194).
    let source = "def f(a, b=1):\n    return a\n\nx = (lambda: f(2))()\n";
    let mut plan = Plan::new(source, "m.py", NATIVE).expect("parses");
    plan.absorb(None, &[failure_at(source.find("lambda").unwrap() as u32)]);
    assert!(plan.report_lines().join("\n").contains("function `f`"));
}

/// How a statement reaches module globals by a string (issue #20): `eval`
/// reads them, `exec` and `globals()` may also bind them, and `locals()`,
/// `vars()` and `dir()` only reach them called with no argument at module
/// scope; a local shadowing the builtin reaches nothing.
#[test]
fn reaching_globals_by_a_string_is_found_where_it_is_the_module_scope() {
    use super::by_string::{Reach, reach};
    let cases = [
        ("def f():\n    return eval('c')\n", Reach::Reads),
        ("def f():\n    exec('c = 1')\n", Reach::Binds),
        ("x = globals()['c']\n", Reach::Binds),
        ("print(dir())\n", Reach::Reads),
        ("for k in list(locals()): pass\n", Reach::Binds),
        ("print(vars(o), dir(o))\n", Reach::No),
        ("def f():\n    return locals(), vars(), dir()\n", Reach::No),
        ("class K:\n    names = dir()\n", Reach::No),
        ("f = lambda: dir()\n", Reach::No),
        ("def f(d=dir()):\n    pass\n", Reach::Reads),
        ("def f():\n    eval = len\n    return eval('c')\n", Reach::No),
        // The module object, through which code may read and bind any
        // global; other uses of `__name__` do not reach it.
        ("def f():\n    return sys.modules[__name__].c\n", Reach::Binds),
        ("m = sys.modules['__main__']\n", Reach::Binds),
        ("def f():\n    import __main__\n", Reach::Binds),
        ("m = importlib.import_module(__name__)\n", Reach::Binds),
        ("m = __import__(__name__)\n", Reach::Binds),
        ("log = logging.getLogger(__name__)\n", Reach::No),
        ("if __name__ == '__main__':\n    main()\n", Reach::No),
    ];
    for (source, expected) in cases {
        let stmt = &stmts(source)[0];
        assert_eq!(reach(stmt, &module_references(stmt)), expected, "{source}");
    }
}

/// Fallback code calling `eval` reads every module global, which compiled
/// code keeps native and copies into the namespace; code calling `exec` may
/// bind every module variable, which then lives in the namespace (issue #20).
#[test]
fn fallback_code_reaching_globals_by_a_string_sees_every_global() {
    let source = "c = 1\nd = 'x'\ndef f():\n    return eval('c')\n";
    let mut plan = Plan::new(source, "m.py", NATIVE).expect("parses");
    plan.absorb(None, &[failure_at(source.find("eval").unwrap() as u32)]);
    assert_eq!(plan.native_globals(), set(&["c", "d"]));
    let source = "import math\nc = 1\nd = 'x'\ndef g() -> int:\n    return c\n\
                  def f():\n    exec('c = 2')\n";
    let mut plan = Plan::new(source, "m.py", NATIVE).expect("parses");
    plan.absorb(None, &[failure_at(source.find("exec").unwrap() as u32)]);
    assert!(plan.native_globals().is_empty());
    let text = plan.render().text;
    assert!(text.contains("return __pycc_fallback_ns['c']"), "{text}");
    // The import is read too, so it also runs in CPython; it is not a
    // variable the code may rebind.
    assert!(plan.mirrored.contains(&0), "{text}");
}
