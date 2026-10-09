//! #974: the shared "is this name declared somewhere other than
//! `class_attrs`?" predicate used by a **class-name-qualified** attribute
//! read (`Derived.LIMIT`) in both `pycc_types` and `pycc_mir`.
//!
//! A class-name-qualified read resolves through the reading class's MRO,
//! most-derived first, and folds to a class attribute's constant only when
//! the *first* class in that MRO that declares the name at all declares it
//! as a class attribute. Walking `class_attrs` alone would be a
//! mis-compile: for
//!
//! ```python
//! class A:
//!     x: int = 2
//!
//!
//! class B(A):
//!     @staticmethod
//!     def x() -> int:
//!         return 1
//! ```
//!
//! `B`'s own `x` shadows `A`'s class attribute, exactly as it does in
//! CPython, so `B.x` must not fold to `2`. This predicate is what makes
//! the walk stop at `B`.
//!
//! Sharing one predicate across the two crates is deliberate. The type
//! checker's lookup and `pycc_mir`'s constant fold must agree on which
//! class wins, and #960 is the precedent for what happens when they do
//! not: the checker reports one type while the fold emits another class's
//! literal, so the program silently compiles to the wrong value (and
//! `pycc_codegen` aborts outright when the two declared types differ).
//! Both crates consume the same [`HirClassDef`], so the predicate is
//! genuinely shareable rather than duplicated. Only the predicate is
//! shared: `pycc_types` needs a `Ty` and `pycc_mir` needs a folded
//! literal, so each crate keeps its own MRO loop around this call.

use super::{HirClassDef, ProtocolMember};

/// #974: returns `true` when `class_def` declares `name` in one of the
/// class-level namespaces that a class-name-qualified read must treat as
/// *shadowing* an inherited class attribute.
///
/// The namespaces checked are `methods`, `static_methods`, `class_methods`,
/// `properties`, `enum_members`, and the [`ProtocolMember::Method`] half of
/// `protocol_members` -- everything a `ClassName.name` read could resolve
/// to in CPython that is not a class attribute. Within one
/// class these are disjoint from `class_attrs`:
/// `class::attrs::reject_class_attr_collisions` rejects a class that
/// declares the same name both ways with `C0001`, so the two checks a
/// caller performs per MRO class can never both succeed on the same class
/// and their relative order is immaterial. The shadowing this predicate
/// detects is therefore always cross-class, between a derived class and a
/// base.
///
/// Four namespaces are excluded on purpose:
///
/// - [`HirClassDef::attrs`], the instance slots. A class object has no
///   instance `__dict__`, so an instance slot declared by a derived class
///   does **not** shadow a base's class attribute for a
///   class-name-qualified read. This is the one place the class-name path
///   deliberately diverges from #960's instance-read precedence, and
///   CPython agrees: for `class A` contributing an instance slot `x`,
///   `class B` declaring `x: int = 2`, and `class C(A, B)`, CPython prints
///   `1` for `c.x` and `2` for `C.x`.
/// - [`ProtocolMember::Attribute`], for exactly the same reason as
///   `attrs`: a bare `x: int` in a protocol body is an interface
///   *requirement*, not a binding on the class object, and a protocol
///   class really can precede a class attribute's owner in a live MRO
///   (`class P(Protocol): x: int`, `class A: x: int = 2`,
///   `class C(P, A)` compiles today, and CPython prints `2` for `C.x`).
///   It is excluded because it is not a shadowing declaration, not
///   because the code path is unreachable. Its sibling
///   [`ProtocolMember::Method`] is **not** excluded: a `Protocol` class
///   executes its body like any other class, so `def x(self) -> int: ...`
///   really does bind `x` in `P.__dict__`, and CPython resolves
///   `C.x` to that function rather than continuing to a base's class
///   attribute. Folding the base's constant there would be a
///   mis-compile in both crates at once, which the shared predicate
///   cannot catch precisely because they agree.
/// - [`HirClassDef::abstract_methods`], which is redundant: an
///   `@abstractmethod` is also entered into `methods` by
///   `class::body::walk_class_body`, so it is already covered.
/// - [`HirClassDef::dataclass_fields`], which is a subset of `attrs` and
///   therefore excluded with it. This is sound only while a dataclass
///   field with a default value is rejected with `C0001`; if field
///   defaults ever land, a defaulted field becomes a real class-object
///   binding in CPython and this exclusion must be revisited.
pub fn declares_name_outside_class_attrs(class_def: &HirClassDef, name: &str) -> bool {
    class_def.methods.iter().any(|(n, _)| n == name)
        || class_def.static_methods.iter().any(|(n, _)| n == name)
        || class_def.class_methods.iter().any(|(n, _)| n == name)
        || class_def.properties.iter().any(|p| p.name == name)
        || class_def.enum_members.iter().any(|(n, _)| n == name)
        || class_def
            .protocol_members
            .iter()
            .any(|member| matches!(member, ProtocolMember::Method { name: n, .. } if n == name))
}
