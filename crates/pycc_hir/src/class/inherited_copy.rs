//! Receiver-exact inherited-method copies (#1337): the one canonical
//! spelling of a copy's name and the one canonical way to recover a copy's
//! origin from that name.
//!
//! `pycc_types`' copy pass (`inherited_copies`) clones an inherited method
//! body `D.m` into a new `HirItem::Function` whose receiver is retyped to a
//! subclass `C`, when the body's behaviour depends on the receiver's class
//! (D-254). Every later phase -- the checker's `super()` anchor, MIR's member
//! resolution and anchor, codegen's function-pointer slot timing, the `ext`
//! driver's publication -- must agree on which items are copies and what
//! they copy. They all ask this module, and nothing else parses a copy name
//! (a traceback frame name only strips the exported [`SUPER_TARGET_MARKER`]).
//!
//! **Two spellings.**
//! - The *primary* copy of the body `C` actually resolves `m` to (the first
//!   definer `D` of `m` in `C`'s MRO, `D != C`) is spelled receiver-style,
//!   exactly as an own definition on `C` would be: `C.m`, `C.p.setter`,
//!   `C.k.classmethod`. `C`'s own namespace does not bind `m` (otherwise `D`
//!   would not be the first definer), so the name is free, and the lexical
//!   `ext` classifiers, `mangle_ext_name`, and `source_frame_name` read it as
//!   an ordinary member of `C`.
//! - A *`super()`-target* copy -- a body `T.m` reached from a body compiled
//!   for `C` through `super()` while `T` is not `m`'s first definer for `C` --
//!   cannot use `C.m`: that name is `C`'s own `m` or the primary copy. It is
//!   spelled `C.m.0super_T`, followed by the same kind suffix a primary copy
//!   carries (`C.p.0super_T.setter`, `C.k.0super_T.classmethod`), so every
//!   kind round-trips through [`inherited_copy_origin`] (today `super()`
//!   reaches only plain methods). The third segment starts with a digit, so it
//!   can never be a Python identifier, a `static`/`classmethod`/`setter`
//!   suffix, or any other name the lowerer emits; both lexical `ext`
//!   classifiers refuse a third segment other than those suffixes, so such a
//!   copy is never exported (it is never the member `C` resolves).
//!
//! **No side table.** Identity is derived from the name plus the class
//! tables every consumer already holds, instead of a new `HirModule` field:
//! `HirModule` and `HirClassDef` are constructed literally in several
//! hundred places, and a derived identity cannot drift from the items it
//! describes. The copy pass asserts that no item that existed before it
//! classifies as a copy.

use crate::HirClassDef;

/// The kind of class member a method body implements, as far as copies are
/// concerned. Static methods have no receiver and are never copied.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CopiedMemberKind {
    /// A regular method, or a `@property` getter (both spelled `<C>.<m>`).
    Method,
    /// A `@<p>.setter` body (`<C>.<p>.setter`).
    Setter,
    /// A `@classmethod` body (`<C>.<k>.classmethod`).
    ClassMethod,
}

impl CopiedMemberKind {
    fn suffix(self) -> &'static str {
        match self {
            CopiedMemberKind::Method => "",
            CopiedMemberKind::Setter => ".setter",
            CopiedMemberKind::ClassMethod => ".classmethod",
        }
    }
}

/// What an inherited-method copy copies, recovered from its name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InheritedCopy {
    /// The class the copy is compiled for: its `self`/`cls` is typed as this
    /// class.
    pub receiver: String,
    /// The class that defines the copied body -- the `super()` anchor.
    pub origin_class: String,
    /// The copied body's own mangled item name (`D.m`, `D.p.setter`, ...).
    pub origin_name: String,
    /// The member name (`m`).
    pub member: String,
    /// The member kind the body implements.
    pub kind: CopiedMemberKind,
}

/// The third-segment marker of a `super()`-target copy (see the module
/// comment). Exported so a caller that only needs to strip the marker (a
/// traceback frame name) spells it from this one constant.
pub const SUPER_TARGET_MARKER: &str = "0super_";

/// Whether `class` binds `member` in its own namespace, as any member kind:
/// a method, property, static method, class method, or class attribute.
#[must_use]
pub fn binds_member(class: &HirClassDef, member: &str) -> bool {
    class.methods.iter().any(|(name, _)| name == member)
        || class.properties.iter().any(|p| p.name == member)
        || class.static_methods.iter().any(|(name, _)| name == member)
        || class.class_methods.iter().any(|(name, _)| name == member)
        || class.class_attrs.iter().any(|(name, _, _)| name == member)
}

/// Whether `mangled` is one of `class`'s own item names.
fn owns_item(class: &HirClassDef, mangled: &str) -> bool {
    class.methods.iter().any(|(_, m)| m == mangled)
        || class
            .properties
            .iter()
            .any(|p| p.getter == mangled || p.setter.as_deref() == Some(mangled))
        || class.static_methods.iter().any(|(_, m)| m == mangled)
        || class.class_methods.iter().any(|(_, m)| m == mangled)
}

/// The first class in `receiver`'s MRO whose own namespace binds `member`,
/// or `None` when no class in the MRO binds it.
///
/// `__init__` ranks constructors as instantiation does (#966): a D-225
/// implicit constructor stub loses to any declared `__init__` later in the
/// MRO, as CPython's `object.__init__` would, and wins only when every
/// constructor in the MRO is implicit.
pub fn first_definer<'c>(
    receiver: &'c HirClassDef,
    member: &str,
    class_of: &impl Fn(&str) -> Option<&'c HirClassDef>,
) -> Option<&'c HirClassDef> {
    let find = |skip_implicit: bool| {
        receiver
            .mro
            .iter()
            .filter_map(|name| class_of(name))
            .find(|class| {
                !(skip_implicit && class.implicit_object_init) && binds_member(class, member)
            })
    };
    find(member == "__init__").or_else(|| find(false))
}

/// Splits an origin item name `D.<rest>` into the member name and kind.
/// Returns `None` for a static method or a name that is not a method item
/// of `origin_class`.
fn split_origin(origin_class: &str, origin_name: &str) -> Option<(String, CopiedMemberKind)> {
    let rest = origin_name.strip_prefix(origin_class)?.strip_prefix('.')?;
    let mut segments = rest.split('.');
    let member = segments.next()?.to_string();
    let kind = match (segments.next(), segments.next()) {
        (None, _) => CopiedMemberKind::Method,
        (Some("setter"), None) => CopiedMemberKind::Setter,
        (Some("classmethod"), None) => CopiedMemberKind::ClassMethod,
        _ => return None,
    };
    Some((member, kind))
}

/// The name of the copy of `origin_name` (defined by `origin_class`)
/// compiled for `receiver`: the receiver-style spelling when `origin_class`
/// is the first definer of the member for `receiver`, the `super()`-target
/// spelling otherwise. `None` when `origin_name` is not a copyable item name
/// of `origin_class` (a static method, or not spelled `origin_class.<m>`).
pub fn inherited_copy_name<'c>(
    receiver: &'c HirClassDef,
    origin_class: &str,
    origin_name: &str,
    class_of: &impl Fn(&str) -> Option<&'c HirClassDef>,
) -> Option<String> {
    let (member, kind) = split_origin(origin_class, origin_name)?;
    let primary = first_definer(receiver, &member, class_of)
        .is_some_and(|definer| definer.name == origin_class && definer.name != receiver.name);
    Some(if primary {
        format!("{}.{member}{}", receiver.name, kind.suffix())
    } else {
        format!(
            "{}.{member}.{SUPER_TARGET_MARKER}{origin_class}{}",
            receiver.name,
            kind.suffix()
        )
    })
}

/// Recovers what `name` copies, or `None` when `name` is not an
/// inherited-method copy. See the module comment for the two spellings.
///
/// This is a pure function of the name and the class tables: it answers
/// "would this name be a copy", so a caller that must know whether the
/// copy *exists* also checks its own item or scope table.
pub fn inherited_copy_origin<'c>(
    name: &str,
    class_of: &impl Fn(&str) -> Option<&'c HirClassDef>,
) -> Option<InheritedCopy> {
    let (receiver_name, rest) = name.split_once('.')?;
    let receiver = class_of(receiver_name)?;
    let mut segments = rest.split('.');
    let member = segments.next()?;
    if let Some(origin_class) = segments
        .next()
        .and_then(|s| s.strip_prefix(SUPER_TARGET_MARKER))
    {
        let kind = match (segments.next(), segments.next()) {
            (None, _) => CopiedMemberKind::Method,
            (Some("setter"), None) => CopiedMemberKind::Setter,
            (Some("classmethod"), None) => CopiedMemberKind::ClassMethod,
            _ => return None,
        };
        let origin = class_of(origin_class)?;
        let origin_name = format!("{origin_class}.{member}{}", kind.suffix());
        if origin_class == receiver_name
            || !receiver.mro.iter().any(|c| c == origin_class)
            || !owns_item(origin, &origin_name)
        {
            return None;
        }
        return Some(InheritedCopy {
            receiver: receiver_name.to_string(),
            origin_class: origin_class.to_string(),
            origin_name,
            member: member.to_string(),
            kind,
        });
    }
    if binds_member(receiver, member) {
        return None;
    }
    let definer = first_definer(receiver, member, class_of)?;
    let origin_name = format!("{}.{rest}", definer.name);
    if !owns_item(definer, &origin_name) {
        return None;
    }
    let (member, kind) = split_origin(&definer.name, &origin_name)?;
    Some(InheritedCopy {
        receiver: receiver_name.to_string(),
        origin_class: definer.name.clone(),
        origin_name,
        member,
        kind,
    })
}

#[cfg(test)]
#[path = "inherited_copy_tests.rs"]
mod tests;
