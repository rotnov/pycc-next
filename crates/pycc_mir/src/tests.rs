//! Unit tests for the crate-root HIR-to-MIR lowering surface.
//!
//! Split into cohesion-driven submodules (issue #546), each grouped around the
//! production seam it exercises. Every submodule is still a descendant of the
//! crate root, so its `use crate::*` reaches exactly the private items the
//! single flat `mod tests` reached through `use super::*`.

mod buffer;
mod builtin;
mod class_attr;
mod class_dunder;
mod class_getitem;
mod class_init_rank;
mod class_mro;
mod class_predicate;
mod class_static;
mod class_super;
mod collection;
mod comprehension;
mod exception;
mod expr;
mod expr_ty;
mod foreign_static;
mod frozenset;
mod hash;
mod import;
mod instance_hash;
mod matching;
mod narrow;
mod obj_bind;
mod obj_call;
mod obj_keyword_call;
mod obj_raise;
// Part 1 of #1255: a comprehension over a CPython object.
mod object_comprehension;
// #1475: a native value boxed into an `object` binding.
mod object_box;
mod protocol;
mod receiver_class_call;
mod scope;
mod set_ops;
mod slice;
mod stmt;
mod try_narrow;
mod unpack;
