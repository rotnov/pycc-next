//! Annotated attribute targets in function bodies (v0.8, #891):
//! `obj.x: T = value` becomes `_pycc_N: T = value` followed by
//! `obj.x = _pycc_N`.
//!
//! In a function CPython never evaluates the annotation of an attribute
//! target (only a module or class body does, PEP 526), so the statement is
//! the store of `value` -- evaluated first, then `obj` -- with `T` a promise
//! about it. The temporary carries that promise: it is checked and
//! converted as a local annotated `T` is (strict mode refuses a value that
//! does not match; non-strict mode erases an unmodeled `T` to `object`),
//! and the store then gives the attribute the temporary's type. Before
//! this, only `obj.x: list[int] = []` and `obj.x: dict[str, int] = {}`
//! compiled, and any other annotated attribute store in a method sent its
//! class to CPython.
//!
//! Non-strict mode has no temporary: the store is the plain `obj.x =
//! value`, as if unannotated, because non-strict code does not refuse a
//! value its annotation does not describe (CPython does not either), and
//! the attribute's representation is decided as any unannotated one's is
//! -- a CPython object when another module of the package reads it, say
//! (`pycc_hir`'s `external_reads`), which a native value typed by the
//! annotation could not be stored into.
//!
//! An annotated attribute target without a value, or outside a function
//! (module or class level, where the annotation is evaluated), is left to
//! the frontend, which refuses it.

use super::Fresh;
use super::build::{ann_assign, assign, load, store};
use ruff_python_ast::{Expr, Stmt};

/// Rewrites every function body of `body` (module doc).
pub(crate) fn split_attribute_annotations(body: &mut [Stmt], fresh: &Fresh) {
    for stmt in body {
        match stmt {
            Stmt::FunctionDef(def) => in_function(&mut def.body, fresh),
            Stmt::ClassDef(class) => split_attribute_annotations(&mut class.body, fresh),
            Stmt::If(s) => {
                split_attribute_annotations(&mut s.body, fresh);
                for clause in &mut s.elif_else_clauses {
                    split_attribute_annotations(&mut clause.body, fresh);
                }
            }
            Stmt::While(s) => {
                split_attribute_annotations(&mut s.body, fresh);
                split_attribute_annotations(&mut s.orelse, fresh);
            }
            Stmt::For(s) => {
                split_attribute_annotations(&mut s.body, fresh);
                split_attribute_annotations(&mut s.orelse, fresh);
            }
            Stmt::With(s) => split_attribute_annotations(&mut s.body, fresh),
            Stmt::Try(s) => {
                split_attribute_annotations(&mut s.body, fresh);
                for handler in &mut s.handlers {
                    let ruff_python_ast::ExceptHandler::ExceptHandler(h) = handler;
                    split_attribute_annotations(&mut h.body, fresh);
                }
                split_attribute_annotations(&mut s.orelse, fresh);
                split_attribute_annotations(&mut s.finalbody, fresh);
            }
            Stmt::Match(s) => {
                for case in &mut s.cases {
                    split_attribute_annotations(&mut case.body, fresh);
                }
            }
            _ => {}
        }
    }
}

/// Rewrites the statements of a function body, recursing into its blocks
/// and nested definitions.
fn in_function(body: &mut thin_vec::ThinVec<Stmt>, fresh: &Fresh) {
    let old = std::mem::take(body);
    for mut stmt in old {
        match &mut stmt {
            Stmt::AnnAssign(ann) if matches!(*ann.target, Expr::Attribute(_)) && ann.value.is_some() => {
                let range = ann.range;
                let value = *ann.value.take().expect("checked above");
                let annotation = std::mem::replace(&mut *ann.annotation, load("object", range));
                let target = std::mem::replace(&mut *ann.target, load("object", range));
                if super::dataclasses::is_non_strict() {
                    body.push(assign(target, value, range));
                } else {
                    let temp = fresh.name();
                    body.push(ann_assign(store(&temp, range), annotation, value, range));
                    body.push(assign(target, load(&temp, range), range));
                }
                continue;
            }
            Stmt::FunctionDef(def) => in_function(&mut def.body, fresh),
            Stmt::ClassDef(class) => split_attribute_annotations(&mut class.body, fresh),
            Stmt::If(s) => {
                in_function(&mut s.body, fresh);
                for clause in &mut s.elif_else_clauses {
                    in_function(&mut clause.body, fresh);
                }
            }
            Stmt::While(s) => {
                in_function(&mut s.body, fresh);
                in_function(&mut s.orelse, fresh);
            }
            Stmt::For(s) => {
                in_function(&mut s.body, fresh);
                in_function(&mut s.orelse, fresh);
            }
            Stmt::With(s) => in_function(&mut s.body, fresh),
            Stmt::Try(s) => {
                in_function(&mut s.body, fresh);
                for handler in &mut s.handlers {
                    let ruff_python_ast::ExceptHandler::ExceptHandler(h) = handler;
                    in_function(&mut h.body, fresh);
                }
                in_function(&mut s.orelse, fresh);
                in_function(&mut s.finalbody, fresh);
            }
            Stmt::Match(s) => {
                for case in &mut s.cases {
                    in_function(&mut case.body, fresh);
                }
            }
            _ => {}
        }
        body.push(stmt);
    }
}

