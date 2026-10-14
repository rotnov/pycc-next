use super::*;
use ruff_python_ast::visitor::{self, Visitor};
use ruff_python_ast::{ExprContext, ModModule};
use std::collections::BTreeSet;

fn parse(source: &str) -> ModModule {
    ruff_python_parser::parse_module(source)
        .expect("test source parses")
        .into_syntax()
}

/// Every name the module mentions -- expression names, `def`/`class`
/// names, `global` names, `except ... as` names -- in source order.
fn names(module: &ModModule) -> Vec<String> {
    struct Names(Vec<String>);
    impl<'a> Visitor<'a> for Names {
        fn visit_stmt(&mut self, stmt: &'a Stmt) {
            match stmt {
                Stmt::FunctionDef(def) => self.0.push(def.name.to_string()),
                Stmt::ClassDef(class) => self.0.push(class.name.to_string()),
                Stmt::Global(global) => self.0.extend(global.names.iter().map(ToString::to_string)),
                _ => {}
            }
            visitor::walk_stmt(self, stmt);
        }
        fn visit_except_handler(&mut self, handler: &'a ruff_python_ast::ExceptHandler) {
            let ruff_python_ast::ExceptHandler::ExceptHandler(node) = handler;
            if let Some(name) = &node.name {
                self.0.push(name.to_string());
            }
            visitor::walk_except_handler(self, handler);
        }
        fn visit_expr(&mut self, expr: &'a Expr) {
            if let Expr::Name(name) = expr {
                self.0.push(name.id.to_string());
            }
            visitor::walk_expr(self, expr);
        }
    }
    let mut collected = Names(Vec::new());
    collected.visit_body(&module.body);
    collected.0
}

/// Renames every module-scope name in `globals` to `G_<name>`, and turns
/// `m.<attr>` into `M_<attr>`; refuses a bare `m`.
struct Test {
    globals: BTreeSet<&'static str>,
}

impl GlobalRewrite for Test {
    fn name(&self, name: &str, ctx: ExprContext) -> NameRewrite {
        if name == "m" && ctx != ExprContext::Invalid {
            return NameRewrite::Refuse {
                code: "C0001",
                message: "module as value".to_string(),
            };
        }
        if self.globals.contains(name) {
            NameRewrite::Rename(format!("G_{name}"))
        } else {
            NameRewrite::Keep
        }
    }

    fn chain(&self, root: &str, attrs: &[&str], _ctx: ExprContext) -> Option<ChainRewrite> {
        (root == "m").then(|| ChainRewrite::Name {
            consumed: 1,
            name: format!("M_{}", attrs[0]),
            cell: attrs[0] == "cell",
        })
    }
}

fn rewrite(source: &str, globals: &[&'static str]) -> (Vec<String>, Vec<RewriteError>) {
    let mut module = parse(source);
    let errors = rewrite_module_globals(
        &mut module,
        &Test {
            globals: globals.iter().copied().collect(),
        },
    );
    (names(&module), errors)
}

#[test]
fn a_qualified_spelling_round_trips() {
    assert_eq!(module_global("helper", "double"), "0m6helper_double");
    assert_eq!(module_global("pkg.sub", "X"), "0m3pkg3sub_X");
    assert_eq!(
        split_module_global("0m3pkg3sub_X"),
        Some(("pkg.sub".to_string(), "X"))
    );
    assert_eq!(
        split_module_global("0m3pkg3sub__private"),
        Some(("pkg.sub".to_string(), "_private"))
    );
    assert_eq!(split_module_global("0gen_C"), None);
    assert_eq!(split_module_global("0m_x"), None);
    assert_eq!(split_module_global("0m9short_x"), None);
    assert_eq!(split_module_global("plain"), None);
    assert_eq!(display_name("0m6helper_double"), "helper.double");
    assert_eq!(display_name("double"), "double");
    assert_eq!(plain_name("0m6helper_double"), "double");
    assert_eq!(plain_name("double"), "double");
    assert_eq!(module_key("pkg.sub"), "0m3pkg3sub_");
}

#[test]
fn demangling_rewrites_every_qualified_spelling_in_a_message() {
    assert_eq!(
        demangle_text("expected `0m8geometry_Point`, got `0m3pkg3sub_Q.x` (`a0m1x_y`)"),
        "expected `Point`, got `Q.x` (`a0m1x_y`)"
    );
    assert_eq!(demangle_text("0m is not 0m_ é"), "0m is not 0m_ é");
}

#[test]
fn module_bindings_exclude_imports_and_include_global_declarations() {
    let module = parse(
        "import os\nfrom m import a\nx = 1\ndef f():\n    global g\n    g = 2\nclass C:\n    y = 3\nfor i in []:\n    pass\nprint(__name__)\n",
    );
    let bindings = module_bindings(&module);
    assert_eq!(
        bindings
            .defined
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>(),
        ["C", "f", "g", "i", "x"]
    );
    assert_eq!(
        bindings
            .imported
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>(),
        ["a", "os"]
    );
    assert!(bindings.mentions_dunder_name);
}

#[test]
fn locals_parameters_and_class_attributes_keep_their_names() {
    let (names, errors) = rewrite(
        "x = 1\ndef f(x):\n    return x\ndef g():\n    x = 2\n    return x\nclass C:\n    x = 3\n    y = x\n    def m(self):\n        return x\nprint(x)\n",
        &["x", "f", "g", "C"],
    );
    assert!(errors.is_empty());
    assert_eq!(
        names,
        [
            "G_x", "G_f", "x", "G_g", "x", "x", "G_C", "x", "x", "y", "m", "G_x", "print", "G_x"
        ]
    );
}

#[test]
fn global_declarations_and_comprehension_scopes_follow_python() {
    let (names, errors) = rewrite(
        "n = 0\ndef bump():\n    global n\n    n += 1\nys = [n + k for k in range(n)]\nzs = [w := k for k in ys]\n",
        &["n", "bump", "ys", "zs", "w"],
    );
    assert!(errors.is_empty());
    assert_eq!(
        names,
        [
            "G_n", "G_bump", "G_n", "G_n", "range", "G_n", "k", "G_n", "k", "G_ys", "G_ys", "k",
            "k", "G_w", "G_zs"
        ]
    );
}

#[test]
fn nonlocal_lambda_and_except_names_resolve_by_scope() {
    let (names, errors) = rewrite(
        "def outer():\n    v = 1\n    def inner():\n        nonlocal v\n        v = 2\n    return lambda v: v\ntry:\n    pass\nexcept E as v:\n    pass\n",
        &["v", "outer"],
    );
    assert!(errors.is_empty());
    assert_eq!(names, ["G_outer", "v", "inner", "v", "v", "G_v", "E"]);
}

#[test]
fn an_attribute_chain_and_a_bare_module_name_go_through_the_rewrite() {
    let (names, errors) = rewrite(
        "print(m.f(1).g)\ndef h():\n    return m.K\nm\ndef local(m):\n    return m.f\n",
        &[],
    );
    assert_eq!(names, ["print", "M_f", "h", "M_K", "m", "local", "m"]);
    assert_eq!(errors.len(), 1);
    assert_eq!(errors[0].message, "module as value");
    assert_eq!(errors[0].range, 40..41);
}

#[test]
fn a_type_variable_string_and_a_nested_string_annotation_follow_the_rename() {
    let mut module = parse(
        "T = TypeVar(\"T\")\nclass P:\n    pass\ndef f(xs: list[\"P\"], k: Literal[\"P\"]) -> None:\n    pass\n",
    );
    let errors = rewrite_module_globals(
        &mut module,
        &Test {
            globals: ["T", "P", "f"].into_iter().collect(),
        },
    );
    assert!(errors.is_empty());
    let Stmt::Assign(assign) = &module.body[0] else {
        panic!("an assignment");
    };
    let Expr::Call(call) = assign.value.as_ref() else {
        panic!("a call");
    };
    let Expr::StringLiteral(literal) = &call.arguments.args[0] else {
        panic!("a string");
    };
    assert_eq!(literal.value.to_str(), "G_T");
    let all = names(&module);
    assert!(all.contains(&"G_P".to_string()), "{all:?}");
    let Stmt::FunctionDef(def) = &module.body[2] else {
        panic!("a def");
    };
    let annotation = def.parameters.args[1]
        .parameter
        .annotation
        .as_deref()
        .unwrap();
    let Expr::Subscript(subscript) = annotation else {
        panic!("a subscript");
    };
    assert!(matches!(subscript.slice.as_ref(), Expr::StringLiteral(_)));
}

#[test]
fn synthesized_statements_have_the_expected_shape() {
    let range = ruff_text_size::TextRange::default();
    let Stmt::ImportFrom(import) =
        synthesized_import("0m6helper_", &["0m6helper_f".to_string()], range)
    else {
        panic!("an import");
    };
    assert_eq!(import.module.unwrap().as_str(), "0m6helper_");
    assert_eq!(import.names[0].name.as_str(), "0m6helper_f");
    let Stmt::Assign(assign) = snapshot_assign("x", "0m1m_x", false, range) else {
        panic!("an assignment");
    };
    assert!(matches!(assign.value.as_ref(), Expr::Name(name) if name.id.as_str() == "0m1m_x"));
    let Stmt::Assign(assign) = snapshot_assign("x", "0m1m_x", true, range) else {
        panic!("an assignment");
    };
    assert!(matches!(assign.value.as_ref(), Expr::Subscript(_)));
    let Stmt::Assign(assign) = string_assign("__name__", "pkg.sub", range) else {
        panic!("an assignment");
    };
    assert!(
        matches!(assign.value.as_ref(), Expr::StringLiteral(s) if s.value.to_str() == "pkg.sub")
    );
}
