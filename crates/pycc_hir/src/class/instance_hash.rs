//! #1335 (Part 1 of #1332): which hash a user-class instance has.
//!
//! [`resolve_instance_hash`] is the one statement of the rule, shared by
//! `pycc_types` (the signature check and every refusal) and `pycc_mir` (the
//! lowering), so the two cannot disagree about a class. `docs/TYPE_SYSTEM.md`'s
//! `hash()` section is the contract.
//!
//! **The per-class verdict** follows CPython's `type_new`: a class whose own
//! dict binds `__eq__` but not `__hash__` gets `__hash__ = None`, and
//! `object.__hash__` (the identity hash) is inherited otherwise. The walk goes
//! down the class's MRO and stops at the first class whose dict binds either
//! name, in any of the five tables a class body can bind a name in -- CPython
//! checks key presence, whatever the value's kind.
//!
//! **Subclass agreement.** Static dispatch is not exact for a value whose
//! static class has subclasses: an inherited method's body is compiled once
//! against the base class, and `self` there can be a subclass instance (the
//! pre-existing `self.m()` miscompile tracked by #1337), as can an
//! `except Base as e:` binding. A class's verdict is therefore admitted only
//! when every class deriving from it agrees with it.

use super::HirClassDef;
use crate::exception::is_builtin_exception_class;
use std::collections::HashMap;

/// The two dunder names the walk looks for.
const HASH: &str = "__hash__";
const EQ: &str = "__eq__";

/// How `hash()` of an instance of a class resolves.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InstanceHash {
    /// `object.__hash__`: the instance's address, rotated as CPython's
    /// `_Py_HashPointer` does.
    Identity,
    /// A user `def __hash__(self)`, by its mangled function name.
    Method(String),
    /// `class` binds `__eq__` without `__hash__`, so CPython sets
    /// `__hash__ = None` and `hash()` raises `TypeError`.
    Unhashable { class: String },
    /// Valid Python that pycc does not compile yet (`C0001`).
    Unsupported(HashRefusal),
}

/// The lowering an admitted [`InstanceHash`] selects.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InstanceHashLowering {
    /// The identity hash of the instance pointer.
    Identity,
    /// A call to the mangled `__hash__` method with the instance.
    Method(String),
}

impl InstanceHash {
    /// The lowering of a verdict `pycc_types` admits, or `None` for a
    /// verdict it refuses before MIR runs.
    #[must_use]
    pub fn lowerable(&self) -> Option<InstanceHashLowering> {
        match self {
            InstanceHash::Identity => Some(InstanceHashLowering::Identity),
            InstanceHash::Method(mangled) => Some(InstanceHashLowering::Method(mangled.clone())),
            InstanceHash::Unhashable { .. } | InstanceHash::Unsupported(_) => None,
        }
    }
}

/// Why an instance's hash is not compiled yet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HashRefusal {
    /// `class` is a `@dataclass` or `@dataclass_transform()` class with no
    /// `__hash__` of its own. pycc marks both the same way, and CPython
    /// treats them differently: `@dataclass` sets `__hash__ = None`, while
    /// `@dataclass_transform()` synthesizes nothing and keeps the identity
    /// hash.
    Dataclass { class: String },
    /// `class` binds `__hash__` as something other than a plain instance
    /// method: a property, a static or class method, or a class attribute.
    NotAMethod { class: String },
    /// `class`, in the MRO, is an enum class (`Enum.__hash__` hashes the
    /// member name, a randomized `str` hash).
    Enum { class: String },
    /// `class`, in the MRO, is an exception class, whose runtime value is
    /// not a plain instance object.
    Exception { class: String },
    /// `class`, in the MRO, is a protocol class.
    Protocol { class: String },
    /// `class`, in the MRO, is a generic class, whose monomorphized name
    /// differs from its checked one.
    Generic { class: String },
    /// `subclass` derives from the hashed class and hashes differently, so
    /// static dispatch on the declared class could pick the wrong hash.
    SubclassDiffers { subclass: String },
}

impl HashRefusal {
    /// The `help` line of the `C0001` this refusal becomes.
    #[must_use]
    pub fn help(&self) -> String {
        match self {
            HashRefusal::Dataclass { class } => format!(
                "`{class}` has no `__hash__` of its own, and pycc cannot yet tell `@dataclass` \
                 (unhashable: CPython raises `TypeError`) from `@dataclass_transform()` \
                 (identity hash); define `__hash__` explicitly"
            ),
            HashRefusal::NotAMethod { class } => format!(
                "`{class}` binds `__hash__` as something other than a plain `def __hash__(self)` \
                 method, which pycc does not compile yet"
            ),
            HashRefusal::Enum { class } => {
                format!("`hash()` of an enum member (`{class}`) is not supported yet")
            }
            HashRefusal::Exception { class } => {
                format!("`hash()` of an exception instance (`{class}`) is not supported yet")
            }
            HashRefusal::Protocol { class } => {
                format!("`hash()` through a protocol class (`{class}`) is not supported yet")
            }
            HashRefusal::Generic { class } => {
                format!("`hash()` of a generic class instance (`{class}`) is not supported yet")
            }
            HashRefusal::SubclassDiffers { subclass } => format!(
                "subclass `{subclass}` hashes differently, and pycc dispatches statically on the \
                 declared class (#1337), so it could pick the wrong `__hash__`"
            ),
        }
    }
}

/// The class definition `name`, which is always registered: the C3 MRO is
/// built from already-lowered class definitions. `.expect`'s panic path
/// lives in libcore, outside this crate's instrumented regions.
fn class_def<'c>(classes: &'c HashMap<String, HirClassDef>, name: &str) -> &'c HirClassDef {
    classes.get(name).expect("MRO class must be registered")
}

/// The refusal for a class kind whose hash pycc does not compile yet, if
/// any class of `def`'s MRO is one.
fn precheck(def: &HirClassDef, classes: &HashMap<String, HirClassDef>) -> Option<HashRefusal> {
    def.mro.iter().find_map(|name| {
        let mro_def = class_def(classes, name);
        let class = name.clone();
        if mro_def.is_protocol {
            Some(HashRefusal::Protocol { class })
        } else if mro_def.is_enum {
            Some(HashRefusal::Enum { class })
        } else if mro_def.exception_type_tag.is_some() || is_builtin_exception_class(name) {
            // The seven basic builtin exceptions are seeded with tag
            // `None`, so the name check is what catches them (and, as a
            // safe over-refusal, a user class shadowing such a name).
            Some(HashRefusal::Exception { class })
        } else if mro_def.type_param.is_some() {
            Some(HashRefusal::Generic { class })
        } else {
            None
        }
    })
}

/// Whether `def`'s own dict binds `name`, in any of the five tables.
fn binds(def: &HirClassDef, name: &str) -> bool {
    def.methods.iter().any(|(n, _)| n == name)
        || def.properties.iter().any(|p| p.name == name)
        || def.static_methods.iter().any(|(n, _)| n == name)
        || def.class_methods.iter().any(|(n, _)| n == name)
        || def.class_attrs.iter().any(|(n, _, _)| n == name)
}

/// The verdict of `class` alone, ignoring its subclasses.
fn own(class: &str, classes: &HashMap<String, HirClassDef>) -> InstanceHash {
    let def = class_def(classes, class);
    if let Some(refusal) = precheck(def, classes) {
        return InstanceHash::Unsupported(refusal);
    }
    for name in &def.mro {
        let mro_def = class_def(classes, name);
        let class = name.clone();
        if binds(mro_def, HASH) {
            return match mro_def.methods.iter().find(|(n, _)| n == HASH) {
                Some((_, mangled)) => InstanceHash::Method(mangled.clone()),
                None => InstanceHash::Unsupported(HashRefusal::NotAMethod { class }),
            };
        }
        if mro_def.is_dataclass {
            return InstanceHash::Unsupported(HashRefusal::Dataclass { class });
        }
        if binds(mro_def, EQ) {
            return InstanceHash::Unhashable { class };
        }
    }
    InstanceHash::Identity
}

/// Whether two verdicts hash alike: the same variant and, for a method, the
/// same function. Two `Unhashable` verdicts agree whichever class binds
/// `__eq__`, since both raise the same `TypeError`.
fn agrees(a: &InstanceHash, b: &InstanceHash) -> bool {
    matches!(
        (a, b),
        (
            InstanceHash::Unhashable { .. },
            InstanceHash::Unhashable { .. }
        )
    ) || a == b
}

/// Every class deriving from `class`, sorted, so the subclass a refusal
/// names does not depend on the table's iteration order.
fn sorted_subclasses<'c>(
    class: &str,
    classes: &'c HashMap<String, HirClassDef>,
) -> Vec<&'c String> {
    let mut subclasses: Vec<&String> = classes
        .iter()
        .filter(|(name, def)| name.as_str() != class && def.mro.iter().any(|m| m == class))
        .map(|(name, _)| name)
        .collect();
    subclasses.sort();
    subclasses
}

/// How `hash()` of an instance whose static class is `class` resolves,
/// against the class table `classes`. See this module's documentation.
///
/// A class whose own verdict is already a refusal keeps that refusal's
/// reason; otherwise the first disagreeing subclass, by name, is named.
///
/// # Panics
/// When `class` or a class of an MRO is not in `classes`; every caller
/// passes a class the type checker resolved from the same table.
#[must_use]
pub fn resolve_instance_hash(class: &str, classes: &HashMap<String, HirClassDef>) -> InstanceHash {
    let verdict = own(class, classes);
    if matches!(verdict, InstanceHash::Unsupported(_)) {
        return verdict;
    }
    let subclasses = sorted_subclasses(class, classes);
    match subclasses
        .into_iter()
        .find(|subclass| !agrees(&own(subclass, classes), &verdict))
    {
        Some(subclass) => InstanceHash::Unsupported(HashRefusal::SubclassDiffers {
            subclass: subclass.clone(),
        }),
        None => verdict,
    }
}

/// #1343 (Part 1 of #1336): how two instances of a class compare when a set
/// probes for an equal element. `docs/TYPE_SYSTEM.md`'s set section is the
/// contract.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InstanceEq {
    /// No class of the MRO binds `__eq__`: `object.__eq__`, which is
    /// identity.
    Identity,
    /// A user `def __eq__(self, other)`, by its mangled function name.
    Method(String),
    /// Valid Python that pycc does not compile yet (`C0001`).
    Unsupported(EqRefusal),
}

/// Why a set of a class with a user `__eq__` is not compiled yet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EqRefusal {
    /// `class` binds `__eq__` as something other than a plain instance
    /// method: a property, a static or class method, or a class attribute.
    NotAMethod { class: String },
    /// `subclass` derives from the element class. CPython's
    /// `do_richcompare` calls a strict subclass's reflected `__eq__` first,
    /// and a subclass may override `__eq__`; static dispatch on the declared
    /// class reproduces neither.
    Subclassed { subclass: String },
}

impl EqRefusal {
    /// The `help` line of the `C0001` this refusal becomes.
    #[must_use]
    pub fn help(&self) -> String {
        match self {
            EqRefusal::NotAMethod { class } => format!(
                "`{class}` binds `__eq__` as something other than a plain \
                 `def __eq__(self, other)` method, which pycc does not compile as a set \
                 element yet"
            ),
            EqRefusal::Subclassed { subclass } => format!(
                "subclass `{subclass}` derives from the element class, and a set compares \
                 through a user `__eq__` by static dispatch on the declared class, which \
                 cannot reproduce CPython's subclass-first reflected comparison"
            ),
        }
    }
}

/// How two instances whose static class is `class` compare inside a set,
/// against the class table `classes`: the first class of the MRO binding
/// `__eq__` decides, and a user method is admitted only when no class
/// derives from `class` (the first by sorted name is named otherwise).
///
/// The protocol, enum, exception, and generic refusals are the hash side's
/// ([`resolve_instance_hash`]), which a set element resolves first.
///
/// # Panics
/// When `class` or a class of an MRO is not in `classes`.
#[must_use]
pub fn resolve_instance_eq(class: &str, classes: &HashMap<String, HirClassDef>) -> InstanceEq {
    let def = class_def(classes, class);
    let Some(owner) = def
        .mro
        .iter()
        .find(|name| binds(class_def(classes, name), EQ))
    else {
        return InstanceEq::Identity;
    };
    let Some((_, mangled)) = class_def(classes, owner)
        .methods
        .iter()
        .find(|(n, _)| n == EQ)
    else {
        return InstanceEq::Unsupported(EqRefusal::NotAMethod {
            class: owner.clone(),
        });
    };
    let subclasses = sorted_subclasses(class, classes);
    match subclasses.first() {
        Some(subclass) => InstanceEq::Unsupported(EqRefusal::Subclassed {
            subclass: (*subclass).clone(),
        }),
        None => InstanceEq::Method(mangled.clone()),
    }
}

#[cfg(test)]
#[path = "instance_hash_tests.rs"]
mod tests;
