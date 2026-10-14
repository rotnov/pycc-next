//! `Color(value)`, the value lookup of a top-level enum class (a class whose
//! one base is `Enum`, `IntEnum` or `StrEnum`), becomes a call of the static
//! method `Color.__pycc_lookup__(value)` HIR lowering synthesizes for every
//! enum class (`pycc_hir::class::enum_methods`). A call with another
//! argument shape is left as written, and HIR refuses it.

use ruff_python_ast::visitor::transformer::{self, Transformer};
use ruff_python_ast::{Expr, Stmt};
use ruff_text_size::Ranged;
use std::collections::HashSet;

/// The static method an enum value lookup calls (`enum_methods::LOOKUP`).
const LOOKUP: &str = "__pycc_lookup__";

fn is_enum_base(expr: &Expr) -> bool {
    let name = match expr {
        Expr::Name(n) => n.id.as_str(),
        Expr::Attribute(a) if matches!(&*a.value, Expr::Name(n) if n.id.as_str() == "enum") => {
            a.attr.as_str()
        }
        _ => return false,
    };
    matches!(name, "Enum" | "IntEnum" | "StrEnum")
}

/// Rewrites every enum value lookup in `body` (module doc).
pub(super) fn enum_lookups(body: &mut [Stmt]) {
    let enums: HashSet<String> = body
        .iter()
        .filter_map(|stmt| match stmt {
            Stmt::ClassDef(class)
                if class
                    .arguments
                    .as_ref()
                    .is_some_and(|a| a.keywords.is_empty() && matches!(a.args.as_ref(), [b] if is_enum_base(b))) =>
            {
                Some(class.name.to_string())
            }
            _ => None,
        })
        .collect();
    if enums.is_empty() {
        return;
    }
    let rewrite = Rewrite { enums };
    for stmt in body.iter_mut() {
        rewrite.visit_stmt(stmt);
    }
}

struct Rewrite {
    enums: HashSet<String>,
}

impl Transformer for Rewrite {
    fn visit_expr(&self, expr: &mut Expr) {
        transformer::walk_expr(self, expr);
        let Expr::Call(call) = expr else {
            return;
        };
        let Expr::Name(name) = &*call.func else {
            return;
        };
        if !self.enums.contains(name.id.as_str())
            || call.arguments.args.len() != 1
            || !call.arguments.keywords.is_empty()
            || matches!(call.arguments.args[0], Expr::Starred(_))
        {
            return;
        }
        let range = call.func.as_ref().range();
        let callee = super::build::attribute((*call.func).clone(), LOOKUP, range);
        call.func = Box::new(callee);
    }
}
