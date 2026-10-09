//! `return NotImplemented` in a comparison method of an `ext` module
//! (#1418, D-258's #1418 amendment).
//!
//! CPython's rich-comparison protocol lets `__eq__`, `__ne__`, `__lt__`,
//! `__le__`, `__gt__` and `__ge__` return the `NotImplemented` singleton to
//! say "this operand pair is not mine". A method that does so returns a
//! CPython object, whatever its annotation says: `def __eq__(self, other)
//! -> bool` really returns `bool` *or* `NotImplemented`. pycc admits exactly
//! that narrow form, and only in a module compiled into an `ext` artifact,
//! where the CPython object is a type pycc has (D-258 rule 1):
//!
//! - [`refusal`] is the gate. Run over every top-level item of an `ext`
//!   module before it is lowered, it refuses (`C0001`) every other spelling
//!   of the name: in a module-level function, an ordinary method, a nested
//!   `def`, `lambda` or class, an assignment, an argument, or any operand.
//! - The statement lowering turns each admitted `return NotImplemented`
//!   into [`crate::HirExpr::NotImplemented`] (see
//!   [`lower_return_value`]), and `class::method::lower_method` then widens
//!   the method's return type to `Ty::Object` when
//!   [`body_returns_not_implemented`] finds one, overriding the written
//!   annotation (`-> bool` included), as CPython's own behaviour does.
//!
//! A `native` build never reaches either half: [`refusal`] and
//! [`lower_return_value`] are both gated on the `ext` mode, so the name
//! keeps lowering to an ordinary `HirExpr::Name` and `pycc_types` keeps its
//! `T0021` "name `NotImplemented` is not defined" byte for byte (D-258
//! rule 6).
//!
//! The gate reads the name only where it is an expression. A binding that
//! carries no expression -- a parameter, `def`, `class` or `import ... as`
//! named `NotImplemented` -- is not refused, so an admitted
//! `return NotImplemented` under such a shadowing binding still yields the
//! builtin singleton where CPython would return the shadowing value.

use crate::{HirExpr, HirStmt, Ty};
use pycc_ast::visitor::{self, Visitor};
use pycc_ast::{Expr, Stmt};
use pycc_diag::Diagnostic;

/// The builtin name this module admits.
const NOT_IMPLEMENTED: &str = "NotImplemented";

/// The six rich-comparison methods whose `return NotImplemented` is
/// admitted, in CPython's `tp_richcompare` operator order.
pub(crate) const COMPARISON_DUNDERS: [&str; 6] =
    ["__lt__", "__le__", "__eq__", "__ne__", "__gt__", "__ge__"];

/// The `help` a `T0022` return mismatch carries when a method that returns
/// `NotImplemented` also returns a native value the object cannot hold
/// (`pycc_types` attaches it in both of its return checks). Since #1475 a
/// scalar, `None` or class-instance value is boxed into the object, so
/// only a container, an `Optional` and the like still reach it.
pub const WIDENED_RETURN_HELP: &str = "this method returns `NotImplemented`, so pycc types its \
     return as the CPython object, as CPython does, whatever its annotation says; return a \
     CPython object, an `int`, `float`, `bool`, `str`, `None` or class-instance value on every \
     path -- boxing a container or `Optional` value into the object is not implemented yet \
     (#1387)";

/// Whether `name` is one of [`COMPARISON_DUNDERS`].
pub(crate) fn is_comparison_dunder(name: &str) -> bool {
    COMPARISON_DUNDERS.contains(&name)
}

/// Whether `expr` is a bare read of the name `NotImplemented`.
fn is_not_implemented(expr: &Expr) -> bool {
    matches!(expr, Expr::Name(name) if name.id.as_str() == NOT_IMPLEMENTED)
}

/// The lowered value of a `return` statement's `value` when it is the
/// admitted `return NotImplemented` of an `ext` module, or `None` when the
/// ordinary expression lowering applies.
///
/// It needs no position check of its own: [`refusal`] has already refused
/// every `NotImplemented` of an `ext` module that is not an admitted
/// `return NotImplemented`.
pub(crate) fn lower_return_value(value: &Expr, aliases: &[(String, Ty)]) -> Option<HirExpr> {
    (crate::func::is_ext_module(aliases) && is_not_implemented(value))
        .then_some(HirExpr::NotImplemented)
}

/// The `C0001` for the first `NotImplemented` in the top-level item `stmt`
/// of an `ext` module that is not an admitted `return NotImplemented`, or
/// `None` when every one is admitted.
///
/// Admitted means: the value of a `return` statement, exactly the name,
/// anywhere in the body of an undecorated, non-`async` `def` of one of
/// [`COMPARISON_DUNDERS`] written directly in a class body, including
/// inside `if`/`elif`/`else`, `try`/`except`/`else`/`finally`, `while`,
/// `for`, `with` and `match` blocks. A nested `def`, `lambda` or class
/// starts a new scope and admits nothing, as does a method's own header
/// (its decorators, defaults and annotations).
pub(crate) fn refusal(stmt: &Stmt) -> Option<Diagnostic> {
    let mut scan = Scan {
        admit: false,
        class_body: false,
        refused: None,
    };
    scan.visit_stmt(stmt);
    scan.refused.map(|range| {
        crate::unsupported(
            "`NotImplemented` is not supported here yet: an `--ext` module admits it only as \
             `return NotImplemented` in a class's `__eq__`, `__ne__`, `__lt__`, `__le__`, \
             `__gt__` or `__ge__` method (#1418)",
            range,
        )
    })
}

/// The walk behind [`refusal`]. Like `exception::ReferenceScan`, it is the
/// generic ruff visitor, so every position a name can occupy is reached by
/// construction.
struct Scan {
    /// Inside the body of an admitted comparison method.
    admit: bool,
    /// The statement about to be visited sits directly in a class body.
    class_body: bool,
    /// The first refused occurrence.
    refused: Option<std::ops::Range<u32>>,
}

impl<'a> Visitor<'a> for Scan {
    fn visit_stmt(&mut self, stmt: &'a Stmt) {
        if self.refused.is_some() {
            return;
        }
        let in_class_body = std::mem::replace(&mut self.class_body, false);
        match stmt {
            Stmt::Return(ret)
                if self.admit && ret.value.as_deref().is_some_and(is_not_implemented) => {}
            Stmt::FunctionDef(def) => {
                let admit = in_class_body
                    && def.decorator_list.is_empty()
                    && !def.is_async
                    && is_comparison_dunder(def.name.as_str());
                let outer = std::mem::replace(&mut self.admit, false);
                for decorator in &def.decorator_list {
                    self.visit_decorator(decorator);
                }
                if let Some(type_params) = &def.type_params {
                    self.visit_type_params(type_params);
                }
                self.visit_parameters(&def.parameters);
                if let Some(returns) = &def.returns {
                    self.visit_annotation(returns);
                }
                self.admit = admit;
                self.visit_body(&def.body);
                self.admit = outer;
            }
            Stmt::ClassDef(class) => {
                let outer = std::mem::replace(&mut self.admit, false);
                for decorator in &class.decorator_list {
                    self.visit_decorator(decorator);
                }
                if let Some(type_params) = &class.type_params {
                    self.visit_type_params(type_params);
                }
                if let Some(arguments) = &class.arguments {
                    self.visit_arguments(arguments);
                }
                for member in &class.body {
                    self.class_body = true;
                    self.visit_stmt(member);
                }
                self.class_body = false;
                self.admit = outer;
            }
            _ => visitor::walk_stmt(self, stmt),
        }
    }

    fn visit_expr(&mut self, expr: &'a Expr) {
        if self.refused.is_some() {
            return;
        }
        if is_not_implemented(expr) {
            self.refused = Some(pycc_ast::expr_range(expr));
            return;
        }
        visitor::walk_expr(self, expr);
    }
}

/// Whether `body` contains a `return NotImplemented` at any nesting depth,
/// that is, whether a comparison method returns the CPython object
/// (#1418).
///
/// Only the lowering of an admitted `return NotImplemented` builds
/// [`HirExpr::NotImplemented`], so this is true only for such a method's
/// body. `class::method::lower_method` reads it to widen the method's
/// return type, and `pycc_types` to attach [`WIDENED_RETURN_HELP`]. As in
/// `buffer_store::body_returns_inside_finally`, HIR has no nested-function
/// statement, so the walk cannot descend into one.
pub fn body_returns_not_implemented(body: &[HirStmt]) -> bool {
    body.iter().any(stmt_returns_not_implemented)
}

fn stmt_returns_not_implemented(stmt: &HirStmt) -> bool {
    match stmt {
        HirStmt::Return(value) => matches!(value, Some(HirExpr::NotImplemented)),
        HirStmt::Try {
            body,
            handlers,
            orelse,
            finalbody,
        }
        | HirStmt::TryStar {
            body,
            handlers,
            orelse,
            finalbody,
        } => {
            body_returns_not_implemented(body)
                || handlers
                    .iter()
                    .any(|handler| body_returns_not_implemented(&handler.body))
                || body_returns_not_implemented(orelse)
                || body_returns_not_implemented(finalbody)
        }
        HirStmt::If { body, orelse, .. } => {
            body_returns_not_implemented(body) || body_returns_not_implemented(orelse)
        }
        HirStmt::While { body, .. }
        | HirStmt::ForRange { body, .. }
        | HirStmt::ForList { body, .. }
        | HirStmt::ForObject { body, .. } => body_returns_not_implemented(body),
        HirStmt::Match { cases, .. } => cases
            .iter()
            .any(|case| body_returns_not_implemented(&case.body)),
        HirStmt::ExprStmt(_)
        | HirStmt::Assign { .. }
        | HirStmt::AnnAssign { .. }
        | HirStmt::DictSet { .. }
        | HirStmt::ListCompAssign { .. }
        | HirStmt::DictCompAssign { .. }
        | HirStmt::SetCompAssign { .. }
        | HirStmt::AttrSet { .. }
        | HirStmt::Delete { .. }
        | HirStmt::DeleteSlice { .. }
        | HirStmt::DeleteAttr { .. }
        | HirStmt::ForeignImport { .. }
        | HirStmt::Raise { .. } => false,
    }
}

#[cfg(test)]
#[path = "not_implemented_tests.rs"]
mod tests;
