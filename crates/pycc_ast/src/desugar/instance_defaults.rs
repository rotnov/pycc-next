//! Non-strict mode: a class-level `None` default its instances shadow.
//!
//! `template = None` in a class body is, in practice, the default of an
//! attribute instances set later (`spec.template = "..."`): CPython reads the
//! class's value until an instance stores its own. A compiled instance has
//! no `__dict__`, so the attribute becomes an instance slot instead: each
//! `name = None` of the class body is removed from it and stored as
//! `self.name = None` first thing in the class's `__init__` (one taking
//! no arguments is added to a class without one). The class Python code
//! sees keeps the attribute on the class too, read through the slot's
//! descriptor when the instance has stored nothing
//! (`crate::ext_build::attach_class_bases`, the shim's slot defaults).
//!
//! Only a plain top-level class is rewritten: no decorator, keyword or type
//! parameter, every base `object` or a class of the module (and a class
//! with a base must have its own `__init__`), no other class of the module
//! subclasses it (a subclass's `__init__` need not run this one), and each
//! default is bound once in the class body, to a name no member reads or
//! defines, that is neither private nor a dunder.

use super::build::{ann_assign, attribute, load};
use ruff_python_ast::{
    AtomicNodeIndex, Expr, ExprContext, Identifier, Parameter, ParameterWithDefault, Parameters,
    Stmt, StmtClassDef, StmtFunctionDef,
};
use std::collections::HashSet;

/// Rewrites the instance defaults of every class of `body` (module doc).
pub(crate) fn instance_defaults(body: &mut [Stmt]) {
    if !super::dataclasses::is_non_strict() {
        return;
    }
    let classes: HashSet<String> = body
        .iter()
        .filter_map(|stmt| match stmt {
            Stmt::ClassDef(class) => Some(class.name.to_string()),
            _ => None,
        })
        .collect();
    let subclassed: HashSet<String> = body
        .iter()
        .filter_map(|stmt| match stmt {
            Stmt::ClassDef(class) => Some(class),
            _ => None,
        })
        .flat_map(|class| class.bases().iter())
        .filter_map(|base| match base {
            Expr::Name(name) => Some(name.id.to_string()),
            _ => None,
        })
        .collect();
    let written = class_attribute_writes(body);
    let mut rewritten: Vec<(String, Vec<String>)> = Vec::new();
    for stmt in body.iter_mut() {
        if let Stmt::ClassDef(class) = stmt
            && !subclassed.contains(class.name.as_str())
        {
            let defaults = rewrite(class, &classes, &written);
            if !defaults.is_empty() {
                rewritten.push((class.name.to_string(), defaults));
            }
        }
    }
    if rewritten.is_empty() {
        return;
    }
    // `C.name` read anywhere in the module is the default itself.
    use ruff_python_ast::visitor::transformer::{self, Transformer};
    struct Reads<'a>(&'a [(String, Vec<String>)]);
    impl Transformer for Reads<'_> {
        fn visit_expr(&self, expr: &mut Expr) {
            if let Expr::Attribute(attr) = expr
                && attr.ctx == ExprContext::Load
                && let Expr::Name(base) = &*attr.value
                && self.0.iter().any(|(class, defaults)| {
                    class == base.id.as_str() && defaults.iter().any(|d| d == attr.attr.as_str())
                })
            {
                *expr = Expr::NoneLiteral(ruff_python_ast::ExprNoneLiteral {
                    node_index: AtomicNodeIndex::default(),
                    range: attr.range,
                });
                return;
            }
            transformer::walk_expr(self, expr);
        }
    }
    let reads = Reads(&rewritten);
    for stmt in body.iter_mut() {
        reads.visit_stmt(stmt);
    }
}

/// `(class, attribute)` for every `C.attr` the module stores into or
/// deletes, at any depth: such an attribute is a class variable, not an
/// instance default.
fn class_attribute_writes(body: &[Stmt]) -> HashSet<(String, String)> {
    use ruff_python_ast::visitor::{self, Visitor};
    struct Writes(HashSet<(String, String)>);
    impl<'a> Visitor<'a> for Writes {
        fn visit_expr(&mut self, expr: &'a Expr) {
            if let Expr::Attribute(attr) = expr
                && attr.ctx != ExprContext::Load
                && let Expr::Name(base) = &*attr.value
            {
                self.0.insert((base.id.to_string(), attr.attr.to_string()));
            }
            visitor::walk_expr(self, expr);
        }
    }
    let mut writes = Writes(HashSet::new());
    for stmt in body {
        writes.visit_stmt(stmt);
    }
    writes.0
}

/// The classes of `body` [`instance_defaults`] rewrites (in non-strict
/// mode): those whose class-level `None` defaults become instance slots.
pub fn rewritten_classes(body: &[Stmt]) -> HashSet<String> {
    let mut copy: Vec<Stmt> = body.to_vec();
    let before: Vec<Stmt> = copy.clone();
    let forced = super::dataclasses::is_non_strict();
    if !forced {
        return HashSet::new();
    }
    instance_defaults(&mut copy);
    copy.iter()
        .zip(&before)
        .filter_map(|(after, before)| match (after, before) {
            (Stmt::ClassDef(a), Stmt::ClassDef(b)) if a.body.len() != b.body.len() || a != b => {
                Some(a.name.to_string())
            }
            _ => None,
        })
        .collect()
}

/// Rewrites `class`'s instance defaults, returning their names.
fn rewrite(
    class: &mut StmtClassDef,
    classes: &HashSet<String>,
    written: &HashSet<(String, String)>,
) -> Vec<String> {
    // A class method or a decorated method may read a default through
    // `cls`, which only the class variable answers.
    let decorated = class.body.iter().any(|stmt| {
        matches!(stmt, Stmt::FunctionDef(def) if !def.decorator_list.is_empty())
    });
    if !class.decorator_list.is_empty()
        || class.type_params.is_some()
        || !class.keywords().is_empty()
        || decorated
    {
        return Vec::new();
    }
    let bases_plain = class.bases().iter().all(|base| {
        matches!(base, Expr::Name(name)
            if name.id.as_str() == "object" || classes.contains(name.id.as_str()))
    });
    let has_base = class
        .bases()
        .iter()
        .any(|base| !matches!(base, Expr::Name(name) if name.id.as_str() == "object"));
    let init = class.body.iter().position(|stmt| {
        matches!(stmt, Stmt::FunctionDef(def)
            if def.name.as_str() == "__init__"
                && def.decorator_list.is_empty()
                && !def.parameters.args.is_empty()
                && def.parameters.posonlyargs.is_empty())
    });
    if !bases_plain || has_base && init.is_none() {
        return Vec::new();
    }
    // The names the class body itself binds (not its methods' locals).
    let mut counts: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    for stmt in &class.body {
        let bound: Vec<String> = match stmt {
            Stmt::Assign(assign) => assign
                .targets
                .iter()
                .filter_map(|target| match target {
                    Expr::Name(name) => Some(name.id.to_string()),
                    _ => None,
                })
                .collect(),
            Stmt::AnnAssign(assign) => match &*assign.target {
                Expr::Name(name) => vec![name.id.to_string()],
                _ => Vec::new(),
            },
            Stmt::AugAssign(assign) => match &*assign.target {
                Expr::Name(name) => vec![name.id.to_string()],
                _ => Vec::new(),
            },
            Stmt::FunctionDef(def) => vec![def.name.to_string()],
            Stmt::ClassDef(nested) => vec![nested.name.to_string()],
            _ => Vec::new(),
        };
        for name in bound {
            *counts.entry(name).or_insert(0) += 1;
        }
    }
    let read = member_reads(class);
    let defaults: Vec<String> = class
        .body
        .iter()
        .filter_map(|stmt| match stmt {
            Stmt::Assign(assign) => match (assign.targets.as_slice(), assign.value.as_ref()) {
                ([Expr::Name(target)], Expr::NoneLiteral(_)) => Some(target.id.to_string()),
                _ => None,
            },
            _ => None,
        })
        .filter(|name| {
            !name.starts_with('_')
                && counts.get(name).copied().unwrap_or(0) == 1
                && !read.contains(name)
                && !written.contains(&(class.name.to_string(), name.clone()))
        })
        .collect();
    if defaults.is_empty() {
        return defaults;
    }
    let range = class.range;
    class.body.retain(|stmt| {
        !matches!(stmt, Stmt::Assign(assign)
            if matches!(assign.targets.as_slice(), [Expr::Name(target)]
                if defaults.iter().any(|d| d == target.id.as_str())))
    });
    let init = class.body.iter().position(|stmt| {
        matches!(stmt, Stmt::FunctionDef(def) if def.name.as_str() == "__init__")
    });
    match init {
        Some(at) => {
            let Stmt::FunctionDef(def) = &mut class.body[at] else {
                return defaults;
            };
            let receiver = def.parameters.args[0].parameter.name.to_string();
            let stores = stores(&receiver, &defaults, range);
            let docstring = usize::from(matches!(def.body.first(), Some(Stmt::Expr(e))
                if matches!(&*e.value, Expr::StringLiteral(_))));
            def.body.splice(docstring..docstring, stores);
        }
        None => {
            let at = usize::from(matches!(class.body.first(), Some(Stmt::Expr(e))
                if matches!(&*e.value, Expr::StringLiteral(_))));
            class.body.insert(at, init_def(&defaults, range));
        }
    }
    defaults
}

/// `receiver.name = None` for each default.
fn stores(receiver: &str, defaults: &[String], range: ruff_text_size::TextRange) -> Vec<Stmt> {
    defaults
        .iter()
        .map(|name| {
            let mut target = attribute(load(receiver, range), name, range);
            if let Expr::Attribute(attr) = &mut target {
                attr.ctx = ExprContext::Store;
            }
            // Annotated `object`: the slot holds whatever the program stores
            // later, as the class-level `None` did.
            ann_assign(
                target,
                load("object", range),
                Expr::NoneLiteral(ruff_python_ast::ExprNoneLiteral {
                    node_index: AtomicNodeIndex::default(),
                    range,
                }),
                range,
            )
        })
        .collect()
}

/// `def __init__(self) -> None:` storing every default.
fn init_def(defaults: &[String], range: ruff_text_size::TextRange) -> Stmt {
    let parameter = |name: &str| ParameterWithDefault {
        node_index: AtomicNodeIndex::default(),
        range,
        parameter: Parameter {
            node_index: AtomicNodeIndex::default(),
            range,
            name: Identifier::new(name, range),
            annotation: None,
        },
        default: None,
    };
    Stmt::FunctionDef(StmtFunctionDef {
        node_index: AtomicNodeIndex::default(),
        range,
        is_async: false,
        decorator_list: Vec::new().into(),
        name: Identifier::new("__init__", range),
        type_params: None,
        parameters: Box::new(Parameters {
            range,
            node_index: AtomicNodeIndex::default(),
            posonlyargs: Vec::new().into(),
            args: vec![parameter("self")].into(),
            vararg: None,
            kwonlyargs: Vec::new().into(),
            kwarg: None,
        }),
        returns: Some(Box::new(Expr::NoneLiteral(ruff_python_ast::ExprNoneLiteral {
            node_index: AtomicNodeIndex::default(),
            range,
        }))),
        body: stores("self", defaults, range).into(),
    })
}

/// The names the class body's members read as bare names or define (a
/// method, a nested class), which a default must not be.
fn member_reads(class: &StmtClassDef) -> HashSet<String> {
    use ruff_python_ast::visitor::{self, Visitor};
    struct Names(HashSet<String>);
    impl<'a> Visitor<'a> for Names {
        fn visit_expr(&mut self, expr: &'a Expr) {
            if let Expr::Name(name) = expr
                && name.ctx == ExprContext::Load
            {
                self.0.insert(name.id.to_string());
            }
            visitor::walk_expr(self, expr);
        }
    }
    let mut names = Names(HashSet::new());
    // Only the class body's own statements: a method's bare names never
    // resolve to the class's attributes. A method's decorators and
    // defaults are evaluated in the class body, though.
    for stmt in &class.body {
        match stmt {
            Stmt::FunctionDef(def) => {
                names.0.insert(def.name.to_string());
                for decorator in &def.decorator_list {
                    names.visit_expr(&decorator.expression);
                }
                for parameter in def.parameters.iter_non_variadic_params() {
                    if let Some(default) = &parameter.default {
                        names.visit_expr(default);
                    }
                }
            }
            Stmt::ClassDef(nested) => {
                names.0.insert(nested.name.to_string());
            }
            Stmt::Assign(_) => {}
            other => names.visit_stmt(other),
        }
    }
    for stmt in &class.body {
        if let Stmt::Assign(assign) = stmt {
            names.visit_expr(&assign.value);
        }
    }
    names.0
}
