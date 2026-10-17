//! Non-strict mode: an augmented assignment to an attribute or item of a
//! computed expression (`ns['C'].count += 1`, `rows[i + 1][j] -= x`), as
//! the statements binding each computed operand to a name first:
//!
//! ```python
//! base(...).attr += v      ==>    t = base(...)
//!                                 t.attr += v
//! a[f(i)] *= v             ==>    t = a; k = f(i)
//!                                 t[k] *= v
//! ```
//!
//! CPython evaluates the target's base, then its index, then the value,
//! once each; so do the statements.

use super::build::{assign, load, store};
use ruff_python_ast::{Expr, ExprContext, Stmt, StmtAugAssign};
use ruff_text_size::Ranged;

/// The statements `aug` means, or `None` when its target is a name, or an
/// attribute or item of a name or attribute chain with a name, literal or
/// slice index (which compile as written).
pub fn desugar_computed_aug_target(aug: &StmtAugAssign) -> Option<Vec<Stmt>> {
    let range = aug.range;
    let n = range.start().to_u32();
    let simple = |e: &Expr| {
        matches!(
            e,
            Expr::Name(_) | Expr::NumberLiteral(_) | Expr::StringLiteral(_) | Expr::Slice(_)
        )
    };
    // A name or an attribute chain on one (`self.items`) is a base the
    // statement compiles with as written.
    fn chain(e: &Expr) -> bool {
        match e {
            Expr::Name(_) => true,
            Expr::Attribute(attr) => chain(&attr.value),
            _ => false,
        }
    }
    let mut out = Vec::new();
    let target = match &*aug.target {
        Expr::Attribute(attr) if !chain(&attr.value) => {
            let base = format!("__pycc_augbase{n}");
            out.push(assign(store(&base, range), (*attr.value).clone(), range));
            let mut attr = attr.clone();
            attr.value = Box::new(load(&base, attr.value.range()));
            Expr::Attribute(attr)
        }
        Expr::Subscript(sub) if !simple(&sub.slice) || !chain(&sub.value) => {
            let mut sub = sub.clone();
            if !chain(&sub.value) {
                let base = format!("__pycc_augbase{n}");
                out.push(assign(store(&base, range), (*sub.value).clone(), range));
                sub.value = Box::new(load(&base, sub.value.range()));
            }
            if !simple(&sub.slice) {
                let key = format!("__pycc_augkey{n}");
                out.push(assign(store(&key, range), (*sub.slice).clone(), range));
                sub.slice = Box::new(load(&key, sub.slice.range()));
            }
            sub.ctx = ExprContext::Store;
            Expr::Subscript(sub)
        }
        _ => return None,
    };
    let mut aug = aug.clone();
    aug.target = Box::new(target);
    out.push(Stmt::AugAssign(aug));
    Some(out)
}
