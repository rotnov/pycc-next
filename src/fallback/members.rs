//! Classes an embedded executable would publish without some of their
//! members (issue #8).
//!
//! An embedded build with fallback code publishes its compiled classes in
//! `__main__` (`crate::build_pipeline`), so an instance reaching fallback
//! code through any value is a published type's object, as in an `--ext`
//! module. What it publishes is what the `--ext` boundary carries: a method
//! whose every parameter and return is annotated (a native build infers an
//! unannotated one's types from its compiled call sites, which a Python
//! caller need not match), not private, not decorated other than as a
//! `staticmethod`/`classmethod`/`property`, with no parameter default (the
//! published callable applies none, #1194), and among the dunders only the
//! constructor, `__repr__`/`__str__` and the comparison and hash slots; and
//! class attributes that are literals; and the class itself undecorated, with
//! no keyword and only bases that publish every member too. A class with any
//! other member keeps
//! running in CPython once the module has fallback code (`cascade`), since
//! fallback code holding one of its instances could not use that member.

use pycc_ast::{Expr, Stmt, StmtClassDef, StmtFunctionDef};
use std::collections::BTreeSet;

/// The dunders a published class carries (`ext_build`'s constructor,
/// `tp_repr`/`tp_str` and the comparison and hash slots).
const PUBLISHED_DUNDERS: &[&str] = &[
    "__init__", "__repr__", "__str__", "__eq__", "__ne__", "__lt__", "__le__", "__gt__", "__ge__",
    "__hash__",
];

/// The module-level classes with a member the embedded artifact would not
/// publish (module doc).
pub(super) fn with_unpublished_members(body: &[Stmt]) -> BTreeSet<String> {
    let mut classes: BTreeSet<&str> = BTreeSet::new();
    let mut unpublished = BTreeSet::new();
    for stmt in body {
        let Stmt::ClassDef(class) = stmt else {
            continue;
        };
        let name = class.name.as_str();
        if !all_published(class, &classes, &unpublished) {
            unpublished.insert(name.to_string());
        }
        classes.insert(name);
    }
    unpublished
}

/// Whether Python code can use every member of `class`: no decorator (a
/// `@dataclass` makes CPython's class a dataclass, which the published type
/// is not), no keyword (`metaclass=`), each base `object`, a builtin
/// exception or an earlier class of the module (`classes`) publishing every
/// member, and every member of its own body published.
fn all_published(class: &StmtClassDef, classes: &BTreeSet<&str>, unpublished: &BTreeSet<String>) -> bool {
    let bases_published = class.bases().iter().all(|expr| match expr {
        Expr::Name(base) => {
            let base = base.id.as_str();
            base == "object"
                || pycc_hir::BUILTIN_EXCEPTION_CLASSES.contains(&base)
                || classes.contains(base) && !unpublished.contains(base)
                // v0.10: a class another module of the package compiled
                // and publishes whole (a class subclassed in its package
                // runs in CPython there otherwise), read natively.
                || crate::package_build::sibling_base(expr)
        }
        _ => false,
    });
    class.decorator_list.is_empty()
        && class.keywords().is_empty()
        && bases_published
        && class.body.iter().all(|stmt| match stmt {
        Stmt::FunctionDef(def) => method_published(def),
        Stmt::Pass(_) => true,
        Stmt::Expr(expr) => matches!(&*expr.value, Expr::StringLiteral(_) | Expr::EllipsisLiteral(_)),
        Stmt::AnnAssign(assign) => assign.value.as_deref().is_none_or(is_literal),
        Stmt::Assign(assign) => is_literal(&assign.value),
        _ => false,
    })
}

fn method_published(def: &StmtFunctionDef) -> bool {
    let name = def.name.as_str();
    let dunder = name.starts_with("__") && name.ends_with("__");
    if dunder && !PUBLISHED_DUNDERS.contains(&name) || !dunder && name.starts_with('_') {
        return false;
    }
    let mut receiver = true;
    for decorator in &def.decorator_list {
        match &decorator.expression {
            Expr::Name(n) if n.id.as_str() == "staticmethod" => receiver = false,
            Expr::Name(n) if matches!(n.id.as_str(), "classmethod" | "property") => {}
            _ => return false,
        }
    }
    let p = &def.parameters;
    if p.vararg.is_some() || p.kwarg.is_some() || !p.kwonlyargs.is_empty() {
        return false;
    }
    let skip = usize::from(receiver);
    let params_annotated = p
        .posonlyargs
        .iter()
        .chain(&p.args)
        .skip(skip)
        .all(|param| param.parameter.annotation.is_some() && param.default.is_none());
    params_annotated && (def.returns.is_some() || name == "__init__")
}

pub(super) fn is_literal(expr: &Expr) -> bool {
    match expr {
        Expr::NumberLiteral(_)
        | Expr::StringLiteral(_)
        | Expr::BooleanLiteral(_)
        | Expr::NoneLiteral(_)
        | Expr::BytesLiteral(_) => true,
        Expr::UnaryOp(op) => matches!(&*op.operand, Expr::NumberLiteral(_)),
        _ => false,
    }
}

/// The module-level classes with a class attribute bound to `None` or a
/// `bytes` literal that does not become an instance slot
/// (`pycc_ast::instance_default_classes`): Python code may read it through
/// the class (published with the class since v0.9,
/// `crate::ext_build::attach_class_bases`), but the published instance has
/// no `__dict__` to shadow it with an instance attribute, which such a
/// default usually exists for (`spec.template = ...` with `template = None`
/// in the class body).
pub(super) fn with_instance_defaults(body: &[Stmt]) -> BTreeSet<String> {
    // The classes `pycc_ast`'s instance-defaults pass makes the defaults
    // instance slots of are published whole.
    let rewritten = pycc_ast::instance_default_classes(body);
    body.iter()
        .filter(|stmt| !matches!(stmt, Stmt::ClassDef(class) if rewritten.contains(class.name.as_str())))
        .filter_map(|stmt| match stmt {
            Stmt::ClassDef(class)
                if class.body.iter().any(|member| {
                    matches!(member, Stmt::Assign(assign)
                        if matches!(&*assign.value, Expr::NoneLiteral(_) | Expr::BytesLiteral(_)))
                }) =>
            {
                Some(class.name.to_string())
            }
            _ => None,
        })
        .collect()
}
