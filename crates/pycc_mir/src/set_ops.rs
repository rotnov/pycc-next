//! How codegen hashes and compares the elements of a set of user-class
//! instances (#1343, Part 1 of #1336).
//!
//! `pycc_codegen` has no class table, so lowering resolves each verdict from
//! `pycc_hir::resolve_instance_hash`/`resolve_instance_eq` and carries it on
//! [`crate::MirExpr::SetLiteral`] and [`crate::MirExpr::SetAdd`]. `None` in
//! those `ops` fields means a `set[int]`, whose runtime insert dedups by
//! value on its own.

use crate::Ty;

/// The hash and equality a set insertion uses for its instance element.
#[derive(Debug, Clone, PartialEq)]
pub struct SetElementOps {
    pub hash: SetHashOp,
    pub eq: SetEqOp,
}

/// How an element's hash is computed, once per insertion as in CPython.
#[derive(Debug, Clone, PartialEq)]
pub enum SetHashOp {
    /// `object.__hash__`: the pointer's identity hash.
    Identity,
    /// A call to the mangled user `__hash__`. `ret` is its declared return,
    /// `int` or `bool`: a direct call returns an encoded `i64` word or a raw
    /// `i1`, and codegen reduces the two differently.
    Method { callee: String, ret: Ty },
}

/// How a new element is compared with a stored one whose hash is equal.
#[derive(Debug, Clone, PartialEq)]
pub enum SetEqOp {
    /// `object.__eq__`: identity.
    Identity,
    /// A call to the mangled user `__eq__`, with the stored element as
    /// `self` and the new one as `other`, CPython's own argument order.
    Method { callee: String },
}

impl SetElementOps {
    /// Whether an insertion calls user code, and so can raise.
    pub fn calls_user_code(&self) -> bool {
        matches!(self.hash, SetHashOp::Method { .. }) || matches!(self.eq, SetEqOp::Method { .. })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_user_method_makes_an_insertion_call_user_code() {
        let method_hash = SetHashOp::Method {
            callee: "R.__hash__".to_string(),
            ret: Ty::Int,
        };
        let method_eq = SetEqOp::Method {
            callee: "R.__eq__".to_string(),
        };
        let ops = |hash: &SetHashOp, eq: &SetEqOp| SetElementOps {
            hash: hash.clone(),
            eq: eq.clone(),
        };
        assert!(!ops(&SetHashOp::Identity, &SetEqOp::Identity).calls_user_code());
        assert!(ops(&method_hash, &SetEqOp::Identity).calls_user_code());
        assert!(ops(&SetHashOp::Identity, &method_eq).calls_user_code());
        assert!(ops(&method_hash, &method_eq).calls_user_code());
    }
}
