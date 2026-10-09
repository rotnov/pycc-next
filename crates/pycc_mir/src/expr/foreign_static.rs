//! Use-site lowering of a class attribute bound to
//! `staticmethod(<foreign callable>)` (Part 1 of #1284, D-256).
//!
//! Such an attribute has no runtime storage and no MIR node of its own:
//! every read `C.name` / `x.name` is rewritten to the recorded foreign
//! reference chain, and every call `C.name(args)` / `x.name(args)` to a
//! call of that chain, then lowered like any other expression. The winner
//! is chosen by `pycc_hir`'s shared MRO helpers, the same ones
//! `pycc_types` consults, so the checker's verdict and this rewrite never
//! disagree on which class-level binding a name reaches.

use super::HirClassDef;
use pycc_hir::{ForeignCallableRef, class_name_foreign_static, instance_foreign_static};
use std::collections::HashMap;

/// The foreign target a read or call of `attr` through the class object
/// `class_def` reaches, or `None` when the first class-level binding of
/// `attr` in its MRO is anything else.
pub(super) fn class_name_target<'a>(
    class_def: &'a HirClassDef,
    classes: &'a HashMap<String, HirClassDef>,
    attr: &str,
) -> Option<&'a ForeignCallableRef> {
    class_name_foreign_static(&class_def.mro, |name: &str| classes.get(name), attr)
}

/// The foreign target a read or call of `attr` through an *instance* of
/// `class_def` reaches: as [`class_name_target`], except that an instance
/// slot of that name anywhere in the MRO shadows the class attribute and
/// yields `None`.
pub(super) fn instance_target<'a>(
    class_def: &'a HirClassDef,
    classes: &'a HashMap<String, HirClassDef>,
    attr: &str,
) -> Option<&'a ForeignCallableRef> {
    instance_foreign_static(&class_def.mro, |name: &str| classes.get(name), attr)
}
