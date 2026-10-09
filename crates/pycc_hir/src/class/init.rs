//! Constructor resolution and synthesis, extracted from
//! [`super::lower_class`] (D-185: decompose the part a change touches).
//!
//! Every class `lower_class` returns ends up with exactly one `__init__` in
//! its method table, and this module is where that guarantee is established.
//! A class that declares its own `__init__` keeps it; a class that declares
//! none inherits the nearest *real* one in its MRO (#432); a class with
//! neither gets an implicit zero-argument constructor synthesized here
//! (#912,
//! [D-225](../../../../docs/decisions/D-225-synthesize-an-implicit-zero-argument-constructor.md)),
//! matching CPython's inherited `object.__init__`.
//!
//! #966: because that implicit constructor lands in the class's *own*
//! method table, it would otherwise out-rank a real `__init__` declared by
//! a later base of a derived class. [`ensure_init`] therefore reports when
//! it synthesized one, `lower_class` records that on
//! [`HirClassDef::implicit_object_init`], and constructor resolution ranks
//! a flagged class last -- where CPython ranks `object.__init__`.
//!
//! [`synthesize_dataclass_init`] serves both callers: the dataclass path
//! passes the merged field list, and the implicit-constructor path passes an
//! empty one, which degenerates to `def __init__(self) -> None: pass`.

use super::HirClassDef;
use crate::{HirExpr, HirItem, HirStmt, Ty};

/// Does any class *above* `mro[0]` in the method resolution order declare an
/// `__init__`?
///
/// #432: a derived class without its own `__init__` inherits the base
/// class's. Answering "yes" here is what keeps this class from synthesizing
/// a second, competing constructor.
///
/// This asks only *whether* an ancestor provides one, not which -- picking
/// the winner is constructor resolution's job, and since #966 that ranking
/// skips a
/// [`HirClassDef::implicit_object_init`](super::HirClassDef::implicit_object_init)
/// ancestor rather than taking the first `__init__` it meets. An MRO entry
/// that is not a class defined in this program -- a synthetic builtin
/// exception base, say -- contributes no `__init__`.
fn inherits_init(mro: &[String], defined_classes: &[(String, HirClassDef)]) -> bool {
    mro.iter().skip(1).any(|mro_class| {
        defined_classes
            .iter()
            .find(|(name, _)| name == mro_class)
            .map(|(_, cd)| cd.methods.iter().any(|(mn, _)| mn == "__init__"))
            .unwrap_or(false)
    })
}

/// Give a class that declares no `__init__` of its own one to be constructed
/// through: either the inherited one, left implicit in the method table
/// exactly as #432 already did, or -- when nothing in the MRO provides one --
/// a freshly synthesized zero-argument constructor appended to `methods` and
/// `items`.
///
/// #912: before D-225 the second case was rejected outright with a `C0001`.
/// It is now the mechanism that makes `class C: pass` instantiable, so every
/// downstream pass may rely on a non-enum class having a resolvable
/// constructor.
///
/// #966: returns `true` exactly when it synthesized one, so `lower_class`
/// can record the provenance on
/// [`HirClassDef::implicit_object_init`](super::HirClassDef::implicit_object_init).
/// The synthesized constructor is indistinguishable by shape from a
/// user-written `def __init__(self) -> None: pass`, so this return value is
/// the only sound way to tell them apart downstream.
pub(super) fn ensure_init(
    class_name: &str,
    mro: &[String],
    defined_classes: &[(String, HirClassDef)],
    methods: &mut Vec<(String, String)>,
    items: &mut Vec<HirItem>,
) -> bool {
    if inherits_init(mro, defined_classes) {
        return false;
    }
    items.push(synthesize_dataclass_init(class_name, &[]));
    methods.push(("__init__".to_string(), format!("{class_name}.__init__")));
    true
}

/// Build `def __init__(self, f1: T1, ...) -> None: self.f1 = f1; ...`.
///
/// With an empty `fields` slice this is the #912 implicit constructor: one
/// `self` parameter and an empty body.
pub(super) fn synthesize_dataclass_init(class_name: &str, fields: &[(String, Ty)]) -> HirItem {
    let self_ty = Ty::Instance(Box::new(class_name.to_string()));
    let mut params: Vec<(String, Ty)> = vec![("self".to_string(), self_ty)];
    for (name, ty) in fields {
        params.push((name.clone(), ty.clone()));
    }
    let body: Vec<HirStmt> = fields
        .iter()
        .map(|(name, _)| HirStmt::AttrSet {
            base: HirExpr::Name("self".to_string()),
            attr: name.clone(),
            value: HirExpr::Name(name.clone()),
        })
        .collect();
    HirItem::Function {
        name: format!("{class_name}.__init__"),
        params,
        return_ty: Ty::None,
        body,
    }
}

#[cfg(test)]
mod tests {
    use crate::{HirItem, lower_checked};

    /// #912: `class A: pass` / `class B(A): pass` -- `A` synthesizes its own
    /// constructor, and `B` then *inherits* it rather than synthesizing a
    /// second one. This is the `inherits_init` true arm, and it is the
    /// property `an_init_less_chain_instantiates_through_the_inherited_constructor`
    /// exercises end to end.
    #[test]
    fn an_init_less_derived_class_inherits_the_synthesized_base_constructor() {
        // `A` also carries a plain method, so the module's item list holds a
        // non-constructor function too -- the filter below has to skip it.
        let module = crate::pycc_parser_test_helper::parse(
            "class A:\n    def foo(self) -> None:\n        return\nclass B(A):\n    pass\n",
        );
        let hir = lower_checked(&module).unwrap();
        let own_init = |class: &str| {
            hir.class_defs
                .iter()
                .find(|(name, _)| name == class)
                .map(|(_, cd)| cd.methods.iter().any(|(mn, _)| mn == "__init__"))
                .expect("both classes are lowered")
        };
        assert!(own_init("A"), "`A` synthesizes its own constructor");
        // #921: an ordinary class lowered through `lower_class` never
        // carries the enum provenance marker.
        assert!(
            hir.class_defs.iter().all(|(_, cd)| !cd.is_enum),
            "only `lower_enum_class` sets `is_enum`"
        );
        assert!(
            !own_init("B"),
            "`B` must reuse `A`'s inherited constructor, not synthesize its own \
             (the MRO resolves it at instantiation time, as #432 already did)"
        );
        let emitted: Vec<&String> = hir
            .items
            .iter()
            .filter_map(|item| match item {
                HirItem::Function { name, .. } if name.ends_with(".__init__") => Some(name),
                _ => None,
            })
            .collect();
        assert_eq!(
            emitted,
            vec!["A.__init__"],
            "exactly one constructor item is synthesized for the chain"
        );
    }

    /// #966: `implicit_object_init` records [`ensure_init`]'s own report,
    /// not the shape of the constructor it produced.
    ///
    /// Both arms are asserted in one test on purpose. `false` is also what
    /// all ~160 mechanical `HirClassDef` construction sites write, so a
    /// lone `false` assertion would pass just as well against a field that
    /// was never wired to `ensure_init` at all; pairing it with a class
    /// that must be `true` is what gives the `false` case discriminating
    /// power.
    #[test]
    fn ensure_init_s_report_is_what_reaches_implicit_object_init() {
        let module = crate::pycc_parser_test_helper::parse(
            "class Implicit:\n    pass\n\
             class Real:\n    def __init__(self) -> None:\n        self.z = 1\n\
             class Inheriting(Real):\n    pass\n\
             class Declaring:\n    def __init__(self) -> None:\n        self.q = 2\n",
        );
        let hir = lower_checked(&module).unwrap();
        let flag = |class: &str| {
            hir.class_defs
                .iter()
                .find(|(name, _)| name == class)
                .map(|(_, cd)| cd.implicit_object_init)
                .expect("every class in the program is lowered")
        };
        assert!(
            flag("Implicit"),
            "a class with no `__init__` anywhere in its MRO gets the D-225 \
             implicit constructor, and must be flagged so constructor \
             resolution can rank it last"
        );
        assert!(
            !flag("Inheriting"),
            "`ensure_init` synthesized nothing here -- `Real.__init__` is \
             inherited through the MRO, so `Inheriting` is not flagged"
        );
        assert!(
            !flag("Real") && !flag("Declaring"),
            "a class that declares its own `__init__` never reaches \
             `ensure_init`, so it is never flagged"
        );
    }

    /// A `@dataclass`'s generated constructor is a *real* one. It is built
    /// by [`synthesize_dataclass_init`], the same helper the implicit
    /// constructor uses, so flagging inside that helper rather than at
    /// [`ensure_init`]'s call site would wrongly rank a dataclass base
    /// last -- see `tests/issue_966_inherited_init_rank.rs`.
    ///
    /// Both field counts are pinned. A *zero*-field `@dataclass` is the one
    /// case that is byte-for-byte shape-identical to the implicit
    /// constructor -- both are `synthesize_dataclass_init(name, &[])` --
    /// so it is exactly where a shape-based rule would silently give the
    /// wrong answer. The `!is_dataclass` guard at `ensure_init`'s call site
    /// makes the field count irrelevant, and this test proves it.
    #[test]
    fn a_dataclass_s_generated_constructor_is_not_flagged_implicit() {
        for (label, body) in [("one field", "    x: int\n"), ("zero fields", "    pass\n")] {
            let module = crate::pycc_parser_test_helper::parse(&format!(
                "from dataclasses import dataclass\n@dataclass\nclass Point:\n{body}"
            ));
            let hir = lower_checked(&module).unwrap();
            let (_, point) = hir
                .class_defs
                .iter()
                .find(|(name, _)| name == "Point")
                .expect("the dataclass is lowered");
            assert!(
                point.methods.iter().any(|(mn, _)| mn == "__init__"),
                "the dataclass path synthesizes its own constructor ({label})"
            );
            assert!(
                !point.implicit_object_init,
                "a dataclass constructor is real and must keep ranking first ({label})"
            );
        }
    }

    /// The `unwrap_or(false)` arm of [`inherits_init`]'s MRO walk: an MRO
    /// entry naming no class in `defined_classes`.
    ///
    /// No successfully-lowered program reaches this arm. `validate_bases`
    /// (`super::mro`) rejects a direct base absent from `defined_classes`
    /// before `lower_class` ever calls [`ensure_init`], every deeper MRO
    /// entry is merged in by C3 from bases that passed that same check, and
    /// `crate::module` seeds all synthetic builtin exception classes into
    /// the class table before any user statement is lowered -- so a
    /// synthetic base is a *defined* class here, not a missing one. The arm
    /// is defensive, and this test reaches it the only way it can be
    /// reached: by calling `inherits_init` directly with a hand-built pair,
    /// the same technique `super::mro`'s own tests use for an analogous
    /// otherwise-unconstructible input.
    #[test]
    fn an_mro_entry_that_names_no_defined_class_contributes_no_init() {
        let mro = vec!["Derived".to_string(), "Vanished".to_string()];
        assert!(
            !super::inherits_init(&mro, &[]),
            "an MRO entry with no matching `defined_classes` entry contributes \
             no `__init__`, rather than panicking or reporting one"
        );
    }
}
