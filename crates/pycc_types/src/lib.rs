
/// The checker's hash maps and sets: keyed by names, built and probed
/// many times per check, and a non-strict build checks a program many
/// times over (v0.5 accept 4). FxHash is several times cheaper than the
/// standard library's DoS-resistant SipHash for such short keys, and a
/// compiler's input is not adversarial in that sense. Nothing depends on
/// iteration order: it was already random per process.
pub(crate) mod fx {
    pub(crate) type HashMap<K, V> = std::collections::HashMap<K, V, rustc_hash::FxBuildHasher>;
    pub(crate) type HashSet<K> = std::collections::HashSet<K, rustc_hash::FxBuildHasher>;
}
mod binop;
mod boolop;
mod buffer;
mod builtin_call;
mod class;
mod dict_key;
mod closure;
mod compare_chain;
mod comprehension;
mod constraints;
mod del_stmt;
mod empty_container;
mod enum_lower;
mod env;
mod exception;
mod expr;
mod foreign;
mod frozenset;
mod hash;
mod if_exp;
mod inherited_copies;
mod instance_order;
mod int_builtins;
mod list_ops;
mod local_binder;
mod locate;
mod loop_flow;
mod module;
mod monomorphize;
mod narrow;
mod ndarray;
mod non_strict;
mod not_implemented;
mod object_box;
mod int_inf;
mod object_none;
mod redeclaration;
mod round;
mod set_element;
mod solver;
mod std_receiver;
mod string_conversion;
#[cfg(test)]
mod tests;
mod unop;
mod unpack;

pub(crate) use local_binder::{bind_local_types_in_body, bind_local_types_in_stmt};
use pycc_hir::{block_always_returns, loop_body};

pub use buffer::{
    function_local_producer_spellings, imported_producer_spellings, is_buffer_producer_spelling,
};
use comprehension::{CompElts, CompView, check_comp_assign};
pub(crate) use enum_lower::{
    check_enum_loop_body_function, check_enum_loop_body_module, enum_member_attr_type,
    unroll_enum_loops,
};
pub(crate) use env::BindingState;
pub use env::Environment;
use exception::{
    check_raise_stmt, check_try_star_stmt, check_try_stmt, is_unshadowed_builtin_exception,
};
pub use expr::infer_expr;
pub(crate) use expr::{infer_expr_in, reject_memoryview_declaration, reject_memoryview_read};
pub(crate) use redeclaration::{
    check_incompatible_attribute_redeclarations, check_incompatible_redefinitions,
};

pub(crate) use constraints::*;
pub use locate::with_item_spans;
pub(crate) use module::module_function_local_names;
pub use module::{
    DiagnosticKey, KeyedDiagnostics, check, check_all, check_all_keyed, check_and_resolve,
    check_and_resolve_all, check_and_resolve_all_keyed,
};
#[cfg(test)]
pub(crate) use module::{check_with_signatures, checked_function_signatures};
pub use monomorphize::*;
pub use non_strict::{non_strict_checked, set_non_strict};

use pycc_diag::{Diagnostic, Span};
#[cfg(test)]
use pycc_hir::BinOpKind;
#[cfg(test)]
use pycc_hir::CmpOpKind;
#[cfg(test)]
use pycc_hir::PropertyDef;
pub use pycc_hir::Ty;
#[cfg(test)]
use pycc_hir::UnaryOpKind;
use pycc_hir::{
    CompIter, FStringPart, HirClassDef, HirExpr, HirItem, HirMatchCase, HirModule, HirPattern,
    HirStmt,
};
use crate::fx::{HashMap, HashSet};
#[cfg(test)]
use std::sync::Arc;

// The crate root's former body, split by concern (mechanical move). Each
// module re-exports into the root so `crate::x` and `super::x` paths keep
// resolving.
mod assign_check;
mod generator;
mod generic_reject;
mod local_names;
mod match_check;
mod name_diagnostics;
mod stmt_check;
mod stmt_check_fn;
use assign_check::*;
pub use generic_reject::check_generic_function;
use generic_reject::*;
use local_names::*;
use match_check::*;
use name_diagnostics::*;
pub use stmt_check::check_stmt;
use stmt_check::*;
pub use stmt_check_fn::check_function;
use stmt_check_fn::*;

/// `inferred` as stored into an `annotation` slot: an `IntEnum` member
/// stored in an `int` name is its value (`pycc_hir::int_enum`).
pub(crate) fn int_enum_slot(inferred: &Ty, annotation: &Ty) -> Ty {
    if *annotation == Ty::Int {
        pycc_hir::int_enum::as_int(inferred)
    } else {
        inferred.clone()
    }
}

/// Whether `container` holds `int`s (list and set elements, dict keys): an
/// `IntEnum` member tested for membership in it is its value.
pub(crate) fn int_enum_needle(container: &Ty) -> bool {
    match container {
        Ty::List(elem) | Ty::Set(elem) | Ty::FrozenSet(elem) => **elem == Ty::Int,
        Ty::Dict(kv) => kv.0 == Ty::Int,
        _ => false,
    }
}
