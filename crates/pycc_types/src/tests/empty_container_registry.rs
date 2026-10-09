//! The registry invariant `annotated_function_environment` must uphold
//! (#1021 review round 12).
//!
//! The #1021 empty-container pre-pass is the one caller that builds an
//! `Environment` from a module whose signatures are not all concrete. Its
//! constructor ends in `bind_classes`, which records *every* class member
//! unconditionally, while the class resolvers panic when a table entry has
//! no ordinary-function registration. Skipping `Ty::Infer` signatures
//! therefore aborted the process on a program that merely fails to infer.
//!
//! These tests pin both halves: the structural invariant across every
//! mangled-name-bearing table `HirClassDef` carries, and the soundness
//! reason a prune of those tables is *not* the fix -- dropping an
//! unannotated override lets the MRO walk resolve the call to a base
//! class's return type, which would be wrong rather than missed.

use super::*;

use crate::annotated_function_environment;

/// Lowers source to HIR without type-checking it, so a module carrying
/// unannotated private members reaches the pre-pass's own constructor.
fn lower(source: &str) -> HirModule {
    let module = pycc_parser::parse(source).expect("test fixture must parse");
    pycc_hir::lower_checked(&module).expect("test fixture must lower")
}

/// Every mangled name any bound class's tables carry must resolve through
/// `lookup_function`. This is the invariant the class resolvers assert with
/// a panic. The walk below enumerates by hand the five mangled-name-bearing
/// tables `HirClassDef` carries today, so a sixth field added later is not
/// covered automatically; what the hardcoded expected-name list does catch is
/// a change in *what* those five tables record for a given class body.
#[test]
fn the_partial_registry_resolves_every_mangled_name_its_class_tables_carry() {
    let hir = lower(
        "class Base:\n\
         \x20   def _inherited(self):\n\
         \x20       return 1\n\
         class A(Base):\n\
         \x20   def _method(self):\n\
         \x20       return 1\n\
         \x20   @property\n\
         \x20   def _getter(self):\n\
         \x20       return 2\n\
         \x20   @_getter.setter\n\
         \x20   def _getter(self, value):\n\
         \x20       self._slot = value\n\
         \x20   @staticmethod\n\
         \x20   def _static():\n\
         \x20       return 3\n\
         \x20   @classmethod\n\
         \x20   def _classmethod(cls):\n\
         \x20       return 4\n\
         def main() -> None:\n\
         \x20   print(0)\n\
         main()\n",
    );

    let env = annotated_function_environment(&hir);
    let mut seen: Vec<String> = Vec::new();
    for (class_name, class_def) in env.classes.iter() {
        let mut mangled_names: Vec<&str> = Vec::new();
        mangled_names.extend(class_def.methods.iter().map(|(_, m)| m.as_str()));
        mangled_names.extend(class_def.static_methods.iter().map(|(_, m)| m.as_str()));
        mangled_names.extend(class_def.class_methods.iter().map(|(_, m)| m.as_str()));
        for prop in &class_def.properties {
            mangled_names.push(prop.getter.as_str());
            if let Some(setter) = &prop.setter {
                mangled_names.push(setter.as_str());
            }
        }
        for mangled in mangled_names {
            assert!(
                env.lookup_function(mangled).is_some(),
                "`{mangled}` is in class `{class_name}`'s own tables but is not \
                 registered as an ordinary function",
            );
            seen.push(mangled.to_string());
        }
    }
    seen.sort();
    seen.dedup();
    assert_eq!(
        seen,
        vec![
            "A._classmethod.classmethod",
            "A._getter",
            "A._getter.setter",
            "A._method",
            "A._static.static",
            "Base.__init__",
            "Base._inherited",
        ],
        "the fixture must exercise every mangled-name-bearing table",
    );
}

/// An unannotated signature is registered with its `Ty::Infer` rather than
/// skipped, so the override stays authoritative and the call resolves to
/// `Ty::Infer` -- which the pre-pass's `concrete` guard then rejects. Were
/// the entry pruned instead, the MRO walk would reach `Base._one` and the
/// container would be resolved to the base class's `str`.
#[test]
fn an_unannotated_override_is_registered_rather_than_left_to_the_base_class() {
    let hir = lower(
        "class Base:\n\
         \x20   def _one(self) -> str:\n\
         \x20       return \"a\"\n\
         class A(Base):\n\
         \x20   def _one(self):\n\
         \x20       return 1\n\
         def main() -> None:\n\
         \x20   print(0)\n\
         main()\n",
    );

    let env = annotated_function_environment(&hir);
    let (params, return_ty) = env
        .lookup_function("A._one")
        .expect("the unannotated override must be registered");
    assert_eq!(
        return_ty,
        &Ty::Infer,
        "the override keeps its own unresolved return type",
    );
    assert_eq!(params.len(), 1, "`self` is the only parameter");
    assert_eq!(
        env.lookup_function("Base._one").map(|(_, ty)| ty),
        Some(&Ty::Str),
        "the annotated base method is unaffected",
    );
}

/// A partially annotated signature can satisfy `is_generic_signature` and
/// still carry `Ty::Infer`: a private method of a PEP 695 generic class may
/// return `T` while one of its parameters stays unannotated. Such an entry is
/// registered in `functions`, because the class tables demand it, but it is
/// deliberately kept out of `generics` -- `instantiate_generic_call` cannot
/// substitute over a parameter whose type is not known yet.
#[test]
fn a_partially_annotated_generic_method_is_registered_but_not_treated_as_generic() {
    let hir = lower(
        "class Box[T]:\n\
         \x20   def __init__(self, v: T) -> None:\n\
         \x20       self.v = v\n\
         \x20   def _get(self, k) -> T:\n\
         \x20       return self.v\n\
         def main() -> None:\n\
         \x20   print(0)\n\
         main()\n",
    );

    let env = annotated_function_environment(&hir);
    assert!(
        env.lookup_function("Box._get").is_some(),
        "the class table's entry must still resolve",
    );
    assert!(
        env.generics.get("Box._get").is_none(),
        "an `Ty::Infer`-carrying signature never enters the generics table",
    );
    assert!(
        env.generics.get("Box.__init__").is_some(),
        "the fully annotated generic method is unaffected",
    );
}
