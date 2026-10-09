//! #1284 Part 1 (#1345): a class attribute bound to
//! `staticmethod(<foreign callable>)`, and the one MRO rule `pycc_types` and
//! `pycc_mir` both use to decide whether a read or call reaches it.
//!
//! The attribute has no storage (D-224, narrowed by D-256). A read or call
//! through it is rewritten, at the use site, into the reference itself --
//! [`ForeignCallableRef::read_expr`] or [`ForeignCallableRef::call_expr`] --
//! which then type-checks and lowers through the existing CPython-object
//! paths (`ObjAttrGet`, `ObjMethodCall`, `ObjCall`). Both crates build the
//! rewritten expression through these two constructors and pick the winning
//! class through [`class_namespace_winner`], so they cannot drift apart.

use super::{HirClassDef, ProtocolMember};
use crate::{ClassAttrValue, HirExpr, ImportBinding};
use pycc_ast::visitor::{self, Visitor};
use pycc_ast::{Expr, Stmt};

/// The reference inside `staticmethod(<ref>)`: a root name that is a
/// module-level foreign (CPython) import, followed by zero or more attribute
/// segments (`os.path.exists` is root `os`, path `["path", "exists"]`;
/// `from operator import add` then `staticmethod(add)` is root `add`, empty
/// path).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ForeignCallableRef {
    pub root: String,
    pub path: Vec<String>,
}

impl ForeignCallableRef {
    /// The reference as an expression: `Name(root)` wrapped in one
    /// `AttrGet` per path segment.
    pub fn read_expr(&self) -> HirExpr {
        Self::chain(&self.root, &self.path)
    }

    /// A call of the reference with `args`.
    ///
    /// An empty path is a direct call of the root name (`HirExpr::Call`,
    /// the #1313 borrowed-callee path). Otherwise it is a method call on the
    /// prefix. It is built as `HirExpr::MethodCall` directly, never as
    /// `ReceiverDispatchedCall`, so a last segment spelled `add`, `get`,
    /// `pop`, or `append` still reaches the object arm rather than #1188's
    /// container dispatch.
    pub fn call_expr(&self, args: Vec<HirExpr>) -> HirExpr {
        match self.path.split_last() {
            None => HirExpr::Call {
                callee: self.root.clone(),
                args,
            },
            Some((method, prefix)) => HirExpr::MethodCall {
                base: Box::new(Self::chain(&self.root, prefix)),
                method: method.clone(),
                args,
            },
        }
    }

    fn chain(root: &str, path: &[String]) -> HirExpr {
        path.iter()
            .fold(HirExpr::Name(root.to_string()), |base, attr| {
                HirExpr::AttrGet {
                    base: Box::new(base),
                    attr: attr.clone(),
                }
            })
    }
}

/// The class that wins a lookup of one name in a class-level namespace.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClassNamespaceWinner<'a> {
    /// The winner is a `staticmethod(<foreign ref>)` class attribute.
    ForeignStatic {
        owner: &'a str,
        target: &'a ForeignCallableRef,
    },
    /// The winner is anything else: a method, `@staticmethod`,
    /// `@classmethod`, `@property`, enum member, `Protocol` `def`, or a
    /// literal class attribute.
    Other { owner: &'a str },
}

/// The first class in `mro` (most-derived first) that binds `name` in any
/// class-level namespace, or `None` when no class does.
///
/// A class-level namespace is everything
/// [`declares_name_outside_class_attrs`](super::declares_name_outside_class_attrs)
/// covers plus `class_attrs`. Instance slots (`attrs`) are deliberately not
/// consulted: a class-name receiver has no instance `__dict__`, and an
/// instance receiver checks slots first through [`instance_foreign_static`].
/// A class `lookup` does not know is skipped.
pub fn class_namespace_winner<'a>(
    mro: &'a [String],
    lookup: impl Fn(&str) -> Option<&'a HirClassDef>,
    name: &str,
) -> Option<ClassNamespaceWinner<'a>> {
    for class_name in mro {
        let Some(class_def) = lookup(class_name) else {
            continue;
        };
        if let Some((_, _, value)) = class_def.class_attrs.iter().find(|(n, _, _)| n == name) {
            return Some(match value {
                ClassAttrValue::ForeignStatic(target) => ClassNamespaceWinner::ForeignStatic {
                    owner: &class_def.name,
                    target,
                },
                _ => ClassNamespaceWinner::Other {
                    owner: &class_def.name,
                },
            });
        }
        if super::declares_name_outside_class_attrs(class_def, name) {
            return Some(ClassNamespaceWinner::Other {
                owner: &class_def.name,
            });
        }
    }
    None
}

/// The foreign reference a **class-name** read or call `C.name` reaches, if
/// the winner of `name` in `C`'s MRO is a foreign `staticmethod` attribute.
pub fn class_name_foreign_static<'a>(
    mro: &'a [String],
    lookup: impl Fn(&str) -> Option<&'a HirClassDef>,
    name: &str,
) -> Option<&'a ForeignCallableRef> {
    match class_namespace_winner(mro, lookup, name) {
        Some(ClassNamespaceWinner::ForeignStatic { target, .. }) => Some(target),
        _ => None,
    }
}

/// Whether any class in `mro` has an instance slot named `name`.
pub fn mro_has_instance_slot<'a>(
    mro: &'a [String],
    lookup: impl Fn(&str) -> Option<&'a HirClassDef>,
    name: &str,
) -> bool {
    mro.iter()
        .filter_map(|class_name| lookup(class_name))
        .any(|class_def| class_def.attrs.iter().any(|(n, _)| n == name))
}

/// The foreign reference an **instance** read or call `x.name` reaches, if
/// no class in the MRO has an instance slot named `name` and the winner of
/// `name` is a foreign `staticmethod` attribute. A slot anywhere in the MRO
/// declines the foreign path, and #960's instance dispatch runs unchanged.
pub fn instance_foreign_static<'a>(
    mro: &'a [String],
    lookup: impl Fn(&str) -> Option<&'a HirClassDef> + Copy,
    name: &str,
) -> Option<&'a ForeignCallableRef> {
    if mro_has_instance_slot(mro, lookup, name) {
        return None;
    }
    class_name_foreign_static(mro, lookup, name)
}

/// An instance access whose class-namespace winner is a foreign
/// `staticmethod` attribute, with an instance slot of the same name
/// declining the foreign path and an `@property` of that name later in the
/// MRO. CPython reads the slot (the winner is a non-data descriptor, so the
/// instance `__dict__` wins), while the existing property-first instance
/// dispatch would reach the property; the access is refused instead.
pub fn slot_behind_foreign_static_meets_property<'a>(
    mro: &'a [String],
    lookup: impl Fn(&str) -> Option<&'a HirClassDef> + Copy,
    name: &str,
) -> bool {
    mro_has_instance_slot(mro, lookup, name)
        && class_name_foreign_static(mro, lookup, name).is_some()
        && mro
            .iter()
            .filter_map(|class_name| lookup(class_name))
            .any(|class_def| class_def.properties.iter().any(|p| p.name == name))
}

/// Rule 7: an instance **read** `x.name` whose winner is a method-kind
/// binding (a method, `@staticmethod`, `@classmethod`, or `Protocol` `def`)
/// while a foreign `staticmethod` attribute of that name sits later in the
/// MRO. Neither crate's existing class-attribute lookup is positional
/// against methods, so the read would reach the later attribute; it is
/// refused instead (#1350).
pub fn method_shadows_foreign_static<'a>(
    mro: &'a [String],
    lookup: impl Fn(&str) -> Option<&'a HirClassDef> + Copy,
    name: &str,
) -> bool {
    if mro_has_instance_slot(mro, lookup, name) {
        return false;
    }
    let Some(ClassNamespaceWinner::Other { owner }) = class_namespace_winner(mro, lookup, name)
    else {
        return false;
    };
    let method_kind = lookup(owner).is_some_and(|owner_def| {
        owner_def.methods.iter().any(|(n, _)| n == name)
            || owner_def.static_methods.iter().any(|(n, _)| n == name)
            || owner_def.class_methods.iter().any(|(n, _)| n == name)
            || owner_def
                .protocol_members
                .iter()
                .any(|member| matches!(member, ProtocolMember::Method { name: n, .. } if n == name))
    });
    method_kind
        && mro
            .iter()
            .skip_while(|class_name| class_name.as_str() != owner)
            .skip(1)
            .filter_map(|class_name| lookup(class_name))
            .any(|class_def| {
                class_def.class_attrs.iter().any(|(n, _, value)| {
                    n == name && matches!(value, ClassAttrValue::ForeignStatic(_))
                })
            })
}

/// What an instance access of one name resolves to, for comparing a class
/// against its subclasses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum InstanceWinner<'a> {
    Slot,
    Foreign(&'a str),
    Other(&'a str),
    Missing,
}

fn instance_winner<'a>(
    mro: &'a [String],
    lookup: impl Fn(&str) -> Option<&'a HirClassDef> + Copy,
    name: &str,
) -> InstanceWinner<'a> {
    if mro_has_instance_slot(mro, lookup, name) {
        return InstanceWinner::Slot;
    }
    match class_namespace_winner(mro, lookup, name) {
        Some(ClassNamespaceWinner::ForeignStatic { owner, .. }) => InstanceWinner::Foreign(owner),
        Some(ClassNamespaceWinner::Other { owner }) => InstanceWinner::Other(owner),
        None => InstanceWinner::Missing,
    }
}

/// The subclass-override refusal for an instance receiver (lifted by #1350
/// once #1337 lands): the name of a class whose MRO contains `class_name`
/// and whose instance winner for `name` differs from `class_name`'s, when at
/// least one of the two winners is a foreign `staticmethod` attribute.
///
/// `pycc` resolves `x.name` statically against `x`'s declared class, while
/// `x` may hold a subclass instance at run time. A differing winner would
/// then be a silent miscompile, so the access is refused. The check fires
/// only when a foreign attribute is involved, so no program that compiled
/// before #1345 changes behaviour. A name `class_name`'s own MRO never binds
/// yields `None`: that is the ordinary unknown-attribute refusal.
pub fn subclass_divergence<'a>(
    class_name: &str,
    all_classes: impl IntoIterator<Item = &'a HirClassDef>,
    lookup: impl Fn(&str) -> Option<&'a HirClassDef> + Copy,
    name: &str,
) -> Option<&'a str> {
    let own_mro = &lookup(class_name)?.mro;
    let own = instance_winner(own_mro, lookup, name);
    // A name the receiver's own class never binds is an ordinary unknown
    // attribute of the declared type, not a subclass-override hazard: the
    // override wording would point at a class-name call that does not exist.
    if own == InstanceWinner::Missing {
        return None;
    }
    // The alphabetically first diverging subclass, so the refusal names the
    // same class whatever order `all_classes` iterates in.
    all_classes
        .into_iter()
        .filter_map(|sub| {
            if sub.name == class_name || !sub.mro.iter().any(|c| c == class_name) {
                return None;
            }
            let theirs = instance_winner(&sub.mro, lookup, name);
            let involves_foreign = matches!(own, InstanceWinner::Foreign(_))
                || matches!(theirs, InstanceWinner::Foreign(_));
            (involves_foreign && theirs != own).then_some(sub.name.as_str())
        })
        .min()
}

/// Whether `body` binds `name` in its own scope: an assignment, annotated
/// assignment with a value, augmented assignment, `for`/`with` target,
/// walrus, `except ... as`, `match` capture (all through
/// `class::enum_call::module_bindings`), or a `def`, `class`, or
/// `import`/`from ... import` statement, at any depth of a compound
/// statement but never inside a nested `def` or `class` body.
///
/// Used for the module body (is `staticmethod` rebound?) and for the
/// statements of a class body that precede an attribute (is the reference's
/// root, or `staticmethod`, bound by the class itself?). Over-reporting only
/// refuses a program, so a `TYPE_CHECKING`-guarded `def` or `import` counts.
pub(crate) fn binds_name(body: &[Stmt], imports: &[ImportBinding], name: &str) -> bool {
    struct DefinitionScan<'n> {
        name: &'n str,
        found: bool,
    }
    impl<'a> Visitor<'a> for DefinitionScan<'_> {
        fn visit_stmt(&mut self, stmt: &'a Stmt) {
            match stmt {
                Stmt::FunctionDef(def) => self.found |= def.name.as_str() == self.name,
                Stmt::ClassDef(def) => self.found |= def.name.as_str() == self.name,
                Stmt::Import(import) => {
                    self.found |= import.names.iter().any(|alias| {
                        let local = alias.asname.as_ref().map_or_else(
                            || alias.name.as_str().split('.').next().unwrap_or_default(),
                            |asname| asname.as_str(),
                        );
                        local == self.name
                    });
                }
                Stmt::ImportFrom(import) => {
                    self.found |= import.names.iter().any(|alias| {
                        alias
                            .asname
                            .as_ref()
                            .map_or(alias.name.as_str(), |asname| asname.as_str())
                            == self.name
                    });
                }
                _ => visitor::walk_stmt(self, stmt),
            }
        }
        // Expressions bind nothing this scan looks for; the store-context
        // bindings they can make are `module_bindings`' job.
        fn visit_expr(&mut self, _expr: &'a Expr) {}
    }
    let mut scan = DefinitionScan { name, found: false };
    scan.visit_body(body);
    scan.found
        || super::enum_call::module_bindings(body, imports)
            .iter()
            .any(|bound| bound == name)
}

#[cfg(test)]
#[path = "foreign_static_tests.rs"]
pub(crate) mod tests;
