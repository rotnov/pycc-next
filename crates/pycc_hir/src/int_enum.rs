//! `IntEnum` members used as `int` (an `int` subclass in CPython).
//!
//! An `IntEnum` member is a compiled enum instance (`enum_class`), so a
//! position that needs an `int` -- an arithmetic, bitwise or unary operand,
//! a comparison against a number, a sequence index, the argument of
//! `int()`/`float()`/`abs()`/`range()`/..., a format spec -- reads its
//! `value` instead: `pycc_types` types the member as `int` there
//! ([`as_int`]) and `pycc_mir` lowers it to `member.value`.
//!
//! The typing rules have no class table at hand (`numeric_result_type`
//! types a pair of `Ty`s), so the checker records the module's `IntEnum`
//! class names here once per check ([`set_int_enums`]).

use crate::{HirClassDef, Ty};
use std::cell::RefCell;
use std::collections::HashSet;

thread_local! {
    static INT_ENUMS: RefCell<HashSet<String>> = RefCell::new(HashSet::new());
}

/// Whether `class` is an `IntEnum` class: the one enum marker whose class
/// `enum_methods` gives the ordering dunders.
pub fn is_int_enum_class(class: &HirClassDef) -> bool {
    class.is_enum && class.methods.iter().any(|(name, _)| name == "__lt__")
}

/// Records the `IntEnum` classes among `classes` for [`is_int_enum`] (and
/// forgets those of the module checked before).
pub fn set_int_enums<'a>(classes: impl IntoIterator<Item = &'a HirClassDef>) {
    let names: HashSet<String> = classes
        .into_iter()
        .filter(|class| is_int_enum_class(class))
        .map(|class| class.name.clone())
        .collect();
    INT_ENUMS.with(|set| *set.borrow_mut() = names);
}

/// Whether `ty` is an instance of a recorded `IntEnum` class.
pub fn is_int_enum(ty: &Ty) -> bool {
    match ty {
        Ty::Instance(class) => INT_ENUMS.with(|set| set.borrow().contains(class.as_str())),
        _ => false,
    }
}

/// `ty` with an `IntEnum` member read as its `int` value.
pub fn as_int(ty: &Ty) -> Ty {
    if is_int_enum(ty) { Ty::Int } else { ty.clone() }
}

/// The builtins whose arguments read an `IntEnum` member as its `int`
/// value (`print`, `str`, `repr`, `hash`, `isinstance`, ... keep the
/// member).
pub fn reads_int_args(callee: &str) -> bool {
    matches!(
        callee,
        "int"
            | "float"
            | "abs"
            | "range"
            | "chr"
            | "hex"
            | "oct"
            | "bin"
            | "divmod"
            | "pow"
            | "round"
            | "math.sqrt"
            | "math.factorial"
            | "math.comb"
            | "math.perm"
            | "math.gcd"
            | "math.isqrt"
    )
}
