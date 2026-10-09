//! The early-exit continuation shape's admission test, shared by the
//! checker's check phase (`pycc_types::narrow::apply_post_if_narrowing`),
//! its constraint solver (`pycc_types::constraints::object_narrow`) and
//! MIR lowering (`pycc_mir`'s `apply_post_if_narrowing`), so the three
//! cannot disagree on whether the rest of a block reads a name narrowed.

use crate::{HirStmt, definitely_terminates, killed_names};

/// True when `if <test>: <body> else: <orelse>`, whose test narrows `name`
/// on its `orelse` side, leaves `name` narrowed for the statements after
/// the `if`.
///
/// Two conditions must both hold:
/// - `body` definitely terminates ([`definitely_terminates`]), so the
///   continuation is reached only through `orelse`;
/// - `orelse` rebinds `name` nowhere ([`killed_names`], which walks nested
///   `elif` chains, loops, `try` and `match` arms and walrus targets).
///
/// The second condition fixes a #1476 review finding. In
/// `if not isinstance(o, int): return 0` followed by `else: o = 1.5`, the
/// one surviving path rebinds `o` to a `float`. Narrowing it to `int` again
/// after the `if` would make every later read unbox a `float` as an
/// `int`, which raises `TypeError` where CPython computes with the float.
/// The same holds for `if x is None: return` followed by `else: x = None`.
pub fn continuation_narrows(body: &[HirStmt], orelse: &[HirStmt], name: &str) -> bool {
    definitely_terminates(body) && !killed_names(orelse).contains(name)
}

#[cfg(test)]
mod tests;
