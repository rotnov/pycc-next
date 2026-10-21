//! Non-strict mode: `except` clauses whose type is a CPython object
//! (`except struct.error:`, `except mod.Error as e:`), which `pycc_hir`'s
//! handlers cannot name (they hold class names, matched by tag).
//!
//! From the first such clause on, the clauses become one clause catching
//! `Exception` into a fresh name, whose body tests the caught exception
//! against each clause's type in order, as CPython's dispatch does:
//!
//! ```python
//! try:                                  try:
//!     body                                  body
//! except ValueError:                    except ValueError:
//!     a                                     a
//! except struct.error as e:     ==>     except Exception as __pycc_exc_N:
//!     b                                     if __pycc_exc_match(__pycc_exc_N, struct.error):
//! except KeyError:                              e = __pycc_exc_object(__pycc_exc_N)
//!     c                                         b
//!                                           elif isinstance(__pycc_exc_N, KeyError):
//!                                               c
//!                                           else:
//!                                               raise
//! ```
//!
//! The type is evaluated when the clause is tried, as in CPython;
//! `__pycc_exc_match` refuses a type that is not an exception class (or a
//! tuple of them) with CPython's `TypeError`, and `e` is the CPython
//! exception object itself (`pycc_hir::builtins::EXC_OBJECT`). A clause
//! after the first object clause that binds a bare-name type with `as`
//! keeps the statement as written (and refused).

use super::build::{call_name, load, store};
use ruff_python_ast::{
    AtomicNodeIndex, ElifElseClause, ExceptHandler, ExceptHandlerExceptHandler, Expr, Identifier,
    Stmt, StmtIf, StmtRaise, StmtTry,
};
use ruff_text_size::TextRange;

/// The callee testing a caught exception against a CPython object.
const EXC_MATCH: &str = "__pycc_exc_match";
/// The callee converting a caught exception to its CPython object.
const EXC_OBJECT: &str = "__pycc_exc_object";

/// Whether `ty`, an `except` clause's type, is what a handler holds as
/// written: a bare name, or a tuple of them.
fn names_a_class(ty: &Expr, object_name: &dyn Fn(&str) -> bool) -> bool {
    let class_name = |e: &Expr| matches!(e, Expr::Name(name) if !object_name(name.id.as_str()));
    match ty {
        Expr::Tuple(tuple) => tuple.elts.iter().all(class_name),
        other => class_name(other),
    }
}

/// The test of an object clause of type `ty` against the caught exception
/// `caught`: `__pycc_exc_match(caught, ty)`, or for a tuple holding class
/// names too (`except (ValueError, zlib.error):`) the objects' match, then
/// `isinstance` with the names. Every object of the tuple is evaluated, as
/// CPython evaluates the whole tuple; a name evaluates to its class.
fn object_test(
    caught: &str,
    ty: &Expr,
    range: TextRange,
    object_name: &dyn Fn(&str) -> bool,
) -> Expr {
    let Expr::Tuple(tuple) = ty else {
        return call_name(EXC_MATCH, vec![load(caught, range), ty.clone()], range);
    };
    let (names, objects): (Vec<&Expr>, Vec<&Expr>) =
        tuple
            .elts
            .iter()
            .partition(|e| matches!(e, Expr::Name(name) if !object_name(name.id.as_str())));
    if names.is_empty() {
        return call_name(EXC_MATCH, vec![load(caught, range), ty.clone()], range);
    }
    let tuple_of = |elts: Vec<&Expr>| {
        Expr::Tuple(ruff_python_ast::ExprTuple {
            node_index: AtomicNodeIndex::default(),
            range,
            elts: elts.into_iter().cloned().collect(),
            ctx: ruff_python_ast::ExprContext::Load,
            parenthesized: true,
        })
    };
    let matched = call_name(EXC_MATCH, vec![load(caught, range), tuple_of(objects)], range);
    let named = call_name("isinstance", vec![load(caught, range), tuple_of(names)], range);
    Expr::BoolOp(ruff_python_ast::ExprBoolOp {
        node_index: AtomicNodeIndex::default(),
        range,
        op: ruff_python_ast::BoolOp::Or,
        values: vec![matched, named],
    })
}

/// `try_stmt` with its object clauses rewritten (module doc), or `None`
/// when it has none (or is a `try`/`except*`, or cannot be rewritten).
/// `object_name` answers whether a bare name is a CPython object (an
/// imported exception class, `except KeyNotFoundError:`), whose clause is
/// tested at run time like an attribute's.
#[must_use]
pub fn object_except_handlers(
    try_stmt: &StmtTry,
    object_name: &dyn Fn(&str) -> bool,
) -> Option<StmtTry> {
    if try_stmt.is_star {
        return None;
    }
    let clause = |h: &ExceptHandler| {
        let ExceptHandler::ExceptHandler(h) = h;
        h.clone()
    };
    let handlers: Vec<ExceptHandlerExceptHandler> = try_stmt.handlers.iter().map(clause).collect();
    let first = handlers
        .iter()
        .position(|h| h.type_.as_deref().is_some_and(|ty| !names_a_class(ty, object_name)))?;
    let range = handlers[first].range;
    let caught = format!("__pycc_exc_{}", u32::from(range.start()));
    let mut tests: Vec<(Option<Expr>, Vec<Stmt>)> = Vec::new();
    let mut catch_all = false;
    for h in &handlers[first..] {
        let Some(ty) = h.type_.as_deref() else {
            tests.push((None, h.body.to_vec()));
            catch_all = true;
            break;
        };
        if names_a_class(ty, object_name) {
            if h.name.is_some() {
                return None;
            }
            let test = call_name("isinstance", vec![load(&caught, h.range), ty.clone()], h.range);
            tests.push((Some(test), h.body.to_vec()));
            continue;
        }
        let test = object_test(&caught, ty, h.range, object_name);
        let mut body = Vec::with_capacity(h.body.len() + 1);
        if let Some(name) = &h.name {
            body.push(super::build::assign(
                store(name.as_str(), name.range),
                call_name(EXC_OBJECT, vec![load(&caught, name.range)], name.range),
                name.range,
            ));
        }
        body.extend(h.body.iter().cloned());
        tests.push((Some(test), body));
    }
    if !catch_all {
        tests.push((
            None,
            vec![Stmt::Raise(StmtRaise {
                node_index: AtomicNodeIndex::default(),
                range,
                exc: None,
                cause: None,
            })],
        ));
    }
    let mut tests = tests.into_iter();
    let (test, body) = tests.next().expect("at least the object clause");
    let dispatch = Stmt::If(StmtIf {
        node_index: AtomicNodeIndex::default(),
        range,
        test: Box::new(test.expect("the first clause has a type")),
        body: body.into(),
        elif_else_clauses: tests
            .map(|(test, body)| ElifElseClause {
                node_index: AtomicNodeIndex::default(),
                range,
                test,
                body: body.into(),
            })
            .collect(),
    });
    let mut rewritten = try_stmt.clone();
    rewritten.handlers = handlers[..first]
        .iter()
        .cloned()
        .map(ExceptHandler::ExceptHandler)
        .chain(std::iter::once(ExceptHandler::ExceptHandler(
            ExceptHandlerExceptHandler {
                node_index: AtomicNodeIndex::default(),
                range,
                type_: Some(Box::new(load("Exception", range))),
                name: Some(Identifier::new(caught, TextRange::empty(range.start()))),
                body: vec![dispatch].into(),
            },
        )))
        .collect();
    Some(rewritten)
}
