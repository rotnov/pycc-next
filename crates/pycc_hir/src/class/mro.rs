//! Base-class resolution and C3 method-resolution-order computation
//! (`validate_bases`, `resolve_mro`, and the `compute_c3_mro` algorithm they
//! sit on).
//!
//! Extracted verbatim from `crates/pycc_hir/src/class.rs` per AGENTS.md's
//! file-decomposition rule and D-185's per-file tracking issue (#548): this
//! is one cohesion-driven seam of that 5,332-line file, not a rewrite. Every
//! diagnostic message, every check, and the C3 algorithm itself are
//! unchanged -- the only edits are the ones the module boundary forces
//! (`lower_class`'s two inline blocks became two named functions taking the
//! class-header `range` explicitly, and the circular-inheritance check spells
//! its `Vec<String>` membership test as `iter().any(...)` now that
//! `class_name` arrives as a `&str` rather than an owned `String`).
//! Part 1 of #1283 (#1318) then added a check and a message that are not
//! boundary edits: an unresolved base naming a subclassable builtin type
//! (`is_subclassable_builtin_type_name`, guarded by `module_rebinds`) now
//! reports `builtin_base_message` instead of `unknown_base_message`.
//!
//! The seam is "which bases are legal, and what order do they linearize
//! into". Everything upstream of it stays in `class.rs`: parsing a class
//! header's base expressions into names (including the duplicate-base
//! rejection), the `Enum`/`Protocol`/`ABC` marker-base consumption, and the
//! generic-class-with-bases gate -- that last one is a PEP 695 type-parameter
//! restriction that happens to read `bases.is_empty()`, not a statement about
//! base resolution, so it deliberately stays with the other `type_param`
//! checks.

use crate::{HirClassDef, HirItem, ImportBinding, Ty, unsupported};
use pycc_diag::Diagnostic;

/// The CPython builtin types a user class may subclass that this version
/// reports as "not supported yet" rather than as an unknown class (Part 1
/// of #1283, #1318): `class fzset(frozenset):` is valid Python.
///
/// Enumerated rather than derived from `pycc_types`'s builtin name list,
/// which mixes functions with types. Deliberately excluded, so they keep the
/// unknown-class message: `object` (`class C(object)` means `class C:`, so
/// the honest fix there is to accept it), the four types CPython itself
/// refuses as a base with `TypeError: type 'bool' is not an acceptable base
/// type` (`bool`, `range`, `slice`, `memoryview`), and every other builtin.
/// `tests/issue_1283_builtin_base.rs` checks the enumeration and the four
/// refusals against the oracle interpreter.
fn is_subclassable_builtin_type_name(name: &str) -> bool {
    matches!(
        name,
        "int"
            | "float"
            | "complex"
            | "str"
            | "bytes"
            | "bytearray"
            | "list"
            | "tuple"
            | "dict"
            | "set"
            | "frozenset"
    )
}

/// Whether an item lowered earlier in this module binds `name`, so a base
/// spelled `name` no longer means the builtin: a type alias (`type list =
/// int`, or the legacy `TypeAlias` spelling), an import (`import list` of a
/// foreign module), a top-level `def`, or any other top-level binding
/// (`frozenset = 1`). Such a base keeps the unknown-class message instead of
/// being reported as a builtin type it is not.
fn module_rebinds(
    name: &str,
    aliases: &[(String, Ty)],
    imports: &[ImportBinding],
    module_items: &[HirItem],
) -> bool {
    aliases.iter().any(|(alias, _)| alias == name)
        || imports
            .iter()
            .any(|binding| crate::import_local_name(binding) == name)
        || module_items
            .iter()
            .any(|item| matches!(item, HirItem::Function { name: defined, .. } if defined == name))
        || crate::top_level_bound_names(module_items).contains(name)
}

/// Validates every direct base of the class being lowered against the
/// classes already defined earlier in the same module (#432).
///
/// Rejects, in order, an unknown base (a name not defined earlier in the
/// module, or a builtin type such as `frozenset`, which gets its own
/// "not supported yet" wording), an enum base class (#941), a PEP 695 generic base class, and
/// circular inheritance (the base already lists this class in its own MRO).
/// Each rejection is a `C0001` capability diagnostic spanning the class
/// header's `range`.
///
/// The enum-base rejection keys on `HirClassDef::is_enum` (the #921
/// provenance flag), so a `StrEnum`-derived base and a member-less
/// docstring-only enum (#744) are caught alike, and a class whose base was
/// itself rejected for extending an enum never gets far enough to be
/// checked. Without it, D-225's `ensure_init` synthesizes an empty
/// constructor for the subclass and instantiating it aborts the compiled
/// program at run time (`value`/`name` slots never filled). The wording
/// follows `docs/DIAGNOSTICS.md`'s C0001 convention: a base *with* members
/// names the `TypeError` CPython itself raises for the same program, while
/// a member-less base -- which CPython does allow extending -- is reported
/// as not supported yet.
///
/// `defined_classes` maps each already-defined class name to its own
/// `HirClassDef`, in source order. `aliases`, `imports` and `module_items`
/// are the module's earlier type aliases, import bindings and lowered
/// items; they only choose the wording of the unknown-base rejection (see
/// [`module_rebinds`]) and never resolve a base.
pub(super) fn validate_bases(
    class_name: &str,
    bases: &[String],
    defined_classes: &[(String, HirClassDef)],
    aliases: &[(String, Ty)],
    imports: &[ImportBinding],
    module_items: &[HirItem],
    range: std::ops::Range<u32>,
) -> Result<(), Diagnostic> {
    // #432: validate each base class against the already-defined classes.
    for base_name in bases {
        let Some(base_def) = defined_classes.iter().find(|(name, _)| name == base_name) else {
            // Both messages are built in `module` so #867's cascade
            // classifier can parse them back (D-219). A builtin type the
            // module has not rebound is valid Python this version cannot
            // compile yet, not an unknown name (Part 1 of #1283).
            let message = if is_subclassable_builtin_type_name(base_name)
                && !module_rebinds(base_name, aliases, imports, module_items)
            {
                crate::module::builtin_base_message(class_name, base_name)
            } else {
                crate::module::unknown_base_message(class_name, base_name)
            };
            return Err(unsupported(message, range.clone()));
        };
        // #941: an enum class is not a real base. CPython refuses to extend
        // an enum that has members; a member-less enum is extensible in
        // CPython but `lower_enum_class` only fires on a direct
        // `Enum`/`StrEnum` marker base, so that shape is unsupported here.
        if base_def.1.is_enum {
            let message = if base_def.1.enum_members.is_empty() {
                format!(
                    "class `{class_name}` cannot inherit from member-less enum class \
                     `{base_name}` -- extending an enum class that has no members is not \
                     supported yet"
                )
            } else {
                format!(
                    "class `{class_name}` cannot inherit from enum class `{base_name}` -- \
                     CPython raises `TypeError: <enum '{class_name}'> cannot extend <enum \
                     '{base_name}'>` because an enum class with members cannot be extended"
                )
            };
            return Err(unsupported(message, range.clone()));
        }
        // Generic classes (from #387) with a type_param cannot be used as
        // base classes yet.
        if base_def.1.type_param.is_some() {
            return Err(unsupported(
                format!(
                    "class `{class_name}` cannot inherit from generic class `{base_name}` -- \
                     generic classes as bases are not supported yet"
                ),
                range.clone(),
            ));
        }
        // Reject circular inheritance: if the base class already lists this
        // class in its own MRO, then inheriting from it would create a
        // cycle.
        if base_def.1.mro.iter().any(|name| name == class_name) {
            return Err(unsupported(
                format!(
                    "class `{class_name}` cannot inherit from `{base_name}` -- circular \
                     inheritance is not supported"
                ),
                range.clone(),
            ));
        }
    }
    Ok(())
}

/// #432/#969: Computes the flat attribute-slot layout shared by `pycc_hir`'s
/// layout gate and `pycc_mir`'s `AttrGet`/`AttrSet` slot resolution.
///
/// `mro_defs` is the class's MRO, most derived first, already resolved to
/// the corresponding [`HirClassDef`]s. Each class contributes its own
/// declared attributes that no more-derived class already contributed; the
/// walk runs most-base-first so a base class's attributes always occupy the
/// same low slot indices in every class that inherits them (an inherited
/// method is lowered once, against its *own* class's layout). A second,
/// most-derived-first pass overrides the slot type for a re-declared
/// attribute, matching CPython's MRO-based attribute resolution.
///
/// The two crates must agree on what counts as a slot -- merged `@dataclass`
/// fields, an exception class's (empty) attribute list -- so the single
/// definition lives here, beside the MRO that indexes it, and `pycc_mir`
/// delegates to it (`pycc_mir::class::mro_attrs`).
pub fn flat_attr_layout(mro_defs: &[&HirClassDef]) -> Vec<(String, Ty)> {
    let mut result: Vec<(String, Ty)> = Vec::new();
    let mut slot_index: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    // Pass 1: assign slots in most-base-first order (reverse MRO).
    for mro_def in mro_defs.iter().rev() {
        for (name, ty) in &mro_def.attrs {
            if !slot_index.contains_key(name) {
                slot_index.insert(name.clone(), result.len());
                result.push((name.clone(), ty.clone()));
            }
        }
    }
    // Pass 2: override types for re-declared attrs (most-derived wins).
    // `overridden.insert` returns true the first time pass 2 sees an attr,
    // and pass 1 already assigned every attr a slot, so the index is always
    // present.
    let mut overridden: std::collections::HashSet<String> = std::collections::HashSet::new();
    for mro_def in mro_defs {
        for (name, ty) in &mro_def.attrs {
            if overridden.insert(name.clone()) {
                let idx = slot_index[name];
                result[idx].1 = ty.clone();
            }
        }
    }
    result
}

/// Resolves `class_def`'s MRO to the `HirClassDef`s that carry the slots,
/// most derived first. The class being lowered is not yet in
/// `defined_classes`, so its own MRO head resolves to `class_def` itself.
fn resolve_mro_defs<'a>(
    class_def: &'a HirClassDef,
    defined_classes: &'a [(String, HirClassDef)],
) -> Vec<&'a HirClassDef> {
    class_def
        .mro
        .iter()
        .filter_map(|mro_class| {
            if *mro_class == class_def.name {
                Some(class_def)
            } else {
                defined_classes
                    .iter()
                    .find(|(name, _)| name == mro_class)
                    .map(|(_, def)| def)
            }
        })
        .collect()
}

/// #969: Rejects a multiple-inheritance shape whose base instance layouts
/// cannot all be embedded in the derived class's flat slot layout.
///
/// D-154 lowers each method once, against the slot indices of its *own*
/// class's layout, and those indices have to keep meaning the same
/// attribute in every class that inherits the method. That holds exactly
/// when every ancestor's layout is a name-wise prefix of the derived
/// layout. With a single base it always holds (the derived layout is the
/// base's layout plus the derived class's own new attributes), but two
/// bases that each declare instance attributes have disjoint layouts and no
/// flat ordering can make both a prefix -- one of them is re-based and its
/// methods then read and write the wrong slots (silently wrong values, or
/// an abort on a slot that was never written).
///
/// Rejecting is a deliberate narrowing of accepted surface rather than a
/// layout redesign; see the decision entry cited from `docs/TYPE_SYSTEM.md`.
/// Bases that declare *no* instance attributes (a methods-only mixin, a
/// class-attribute-only base, a builtin exception class) contribute no
/// slots and stay accepted, as do bases whose attributes all share names
/// (they share the one slot).
///
/// The comparison is name-wise only, which is sufficient because D-210's
/// `T0052` (`pycc_types`) independently rejects two classes in one MRO that
/// declare the same attribute name with differing types: equal names there
/// imply equal slot types, so a name-wise prefix is a full layout prefix.
/// Relaxing `T0052` would require revisiting that reasoning here. Because
/// this gate runs during HIR lowering, ahead of type checking, a program
/// that violates both is reported as `C0001` rather than `T0052`.
pub(super) fn validate_mro_slot_layout(
    class_def: &HirClassDef,
    defined_classes: &[(String, HirClassDef)],
    range: std::ops::Range<u32>,
) -> Result<(), Diagnostic> {
    // A single base can never violate the prefix property, and a base-less
    // class has no inherited method to mis-address.
    if class_def.bases.len() < 2 {
        return Ok(());
    }
    let derived = flat_attr_layout(&resolve_mro_defs(class_def, defined_classes));
    let class_name = &class_def.name;
    for ancestor in class_def
        .mro
        .iter()
        .skip(1)
        .filter_map(|name| defined_classes.iter().find(|(n, _)| n == name))
        .map(|(_, def)| def)
    {
        // C3 guarantees the ancestor's own MRO is a sub-sequence of this
        // class's, so its layout can never be *longer* than the derived
        // one -- comparing the names pairwise is exactly the prefix test.
        // Checking each ancestor against the derived layout alone, rather
        // than every pair of ancestors, is sufficient: two sequences that
        // are each a prefix of one common sequence are themselves
        // prefix-comparable, so the linear check is equivalent to the
        // quadratic one.
        let ancestor_layout = flat_attr_layout(&resolve_mro_defs(ancestor, defined_classes));
        if !ancestor_layout
            .iter()
            .zip(derived.iter())
            .all(|((ancestor_attr, _), (derived_attr, _))| ancestor_attr == derived_attr)
        {
            let ancestor_name = &ancestor.name;
            return Err(unsupported(
                format!(
                    "class `{class_name}` inherits instance attributes from more than one \
                     class in its method resolution order -- multiple inheritance where two \
                     classes in the method resolution order each declare their own instance \
                     attributes is not supported yet: \
                     `{ancestor_name}`'s instance layout is not a prefix of `{class_name}`'s, \
                     so `{ancestor_name}`'s own methods would address the wrong attribute slots"
                ),
                range,
            ));
        }
    }
    Ok(())
}

/// Computes the class's C3 linearization, mapping an impossible merge onto
/// the `C0001` "inconsistent method resolution order" diagnostic (#432).
///
/// A `None` from `compute_c3_mro` means the inheritance order is
/// inconsistent (a C3 conflict), which is rejected as a circular/inconsistent
/// inheritance error.
pub(super) fn resolve_mro(
    class_name: &str,
    bases: &[String],
    defined_classes: &[(String, HirClassDef)],
    range: std::ops::Range<u32>,
) -> Result<Vec<String>, Diagnostic> {
    compute_c3_mro(class_name, bases, defined_classes).ok_or_else(|| {
        unsupported(
            format!(
                "class `{class_name}` has an inconsistent method resolution order (MRO) -- \
                 the C3 linearization of its base classes is impossible"
            ),
            range,
        )
    })
}

/// Computes the C3 linearization (MRO) for a class with the given name and
/// direct bases, using the already-defined classes' own MROs. This is the
/// standard C3 algorithm from <https://en.wikipedia.org/wiki/C3_linearization>:
///
/// `L[C] = C + merge(L[B1], L[B2], ..., [B1, B2, ...])`
///
/// where `merge` repeatedly takes the head of the first non-empty list that
/// does not appear in the tail of any other list, and appends it to the
/// result. If no such element exists, the linearization is impossible (a
/// conflicting inheritance order) and `None` is returned.
///
/// `defined_classes` maps each already-defined class name to its own
/// `HirClassDef` (which carries its own `mro`). Every base in `bases` must
/// already be present in `defined_classes` -- `lower_class` validates this
/// before calling this function.
fn compute_c3_mro(
    class_name: &str,
    bases: &[String],
    defined_classes: &[(String, HirClassDef)],
) -> Option<Vec<String>> {
    if bases.is_empty() {
        return Some(vec![class_name.to_string()]);
    }
    // Collect each base's own MRO (already computed, since bases must be
    // defined before the derived class).
    let mut sequences: Vec<Vec<String>> = Vec::with_capacity(bases.len() + 1);
    for base_name in bases {
        let base_def = defined_classes
            .iter()
            .find(|(name, _)| name == base_name)
            .map(|(_, def)| def)
            .expect("base class must be defined before the derived class");
        sequences.push(base_def.mro.clone());
    }
    // The last sequence is the list of base names themselves.
    sequences.push(bases.to_vec());
    let mut result = vec![class_name.to_string()];
    loop {
        // Remove empty sequences.
        sequences.retain(|s| !s.is_empty());
        if sequences.is_empty() {
            return Some(result);
        }
        // Find the first head that does not appear in the tail of any
        // other sequence.
        let mut chosen: Option<String> = None;
        for seq in &sequences {
            let candidate = &seq[0];
            let in_tail = sequences
                .iter()
                .any(|s| s.iter().skip(1).any(|elem| elem == candidate));
            if !in_tail {
                chosen = Some(candidate.clone());
                break;
            }
        }
        let Some(candidate) = chosen else {
            // C3 linearization is impossible -- a conflicting inheritance
            // order. This is a "circular or inconsistent MRO" error.
            return None;
        };
        result.push(candidate.clone());
        // Remove the chosen element from the head of every sequence that
        // starts with it.
        for seq in &mut sequences {
            if seq[0] == candidate {
                seq.remove(0);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::flat_attr_layout;
    use crate::HirClassDef;
    use crate::class::tests::lower_ok;
    use crate::lower_checked;

    // -- #969: flat attribute-slot layout and the layout-prefix gate --------

    /// Collects the `HirClassDef`s of `names`, in order, out of a lowered
    /// module, so a test can call `flat_attr_layout` on a real MRO slice.
    fn defs_of<'a>(hir: &'a crate::HirModule, names: &[&str]) -> Vec<&'a HirClassDef> {
        names
            .iter()
            .map(|name| {
                &hir.class_defs
                    .iter()
                    .find(|(class_name, _)| class_name == name)
                    .expect("test fixture should define the class")
                    .1
            })
            .collect()
    }

    fn layout_names(layout: &[(String, crate::Ty)]) -> Vec<String> {
        layout.iter().map(|(name, _)| name.clone()).collect()
    }

    #[test]
    fn flat_attr_layout_assigns_slots_most_base_first() {
        let hir = lower_ok(
            "class A:\n    def __init__(self) -> None:\n        self.a = 1\nclass B(A):\n    def __init__(self) -> None:\n        self.a = 1\n        self.b = 2\n",
        );
        // `B`'s MRO is `[B, A]`; the walk is most-base-first, so `A`'s `a`
        // takes slot 0 and `B`'s own `b` follows it.
        assert_eq!(
            layout_names(&flat_attr_layout(&defs_of(&hir, &["B", "A"]))),
            vec!["a".to_string(), "b".to_string()]
        );
        // The base's own layout is exactly the derived layout's prefix.
        assert_eq!(
            layout_names(&flat_attr_layout(&defs_of(&hir, &["A"]))),
            vec!["a".to_string()]
        );
    }

    #[test]
    fn flat_attr_layout_deduplicates_a_redeclared_attribute() {
        // Pass 1's `slot_index` skip: a name declared by two classes in one
        // MRO gets one slot, not two. This is why two bases declaring the
        // *same* attribute name never violate the #969 prefix predicate.
        let hir = lower_ok(
            "class A:\n    def __init__(self) -> None:\n        self.x = 1\nclass B(A):\n    def __init__(self) -> None:\n        self.x = 2\n",
        );
        assert_eq!(
            layout_names(&flat_attr_layout(&defs_of(&hir, &["B", "A"]))),
            vec!["x".to_string()]
        );
    }

    #[test]
    fn flat_attr_layout_lets_the_most_derived_declaration_win_on_type() {
        // Pass 2: the slot keeps its most-base-first *index* but takes the
        // most-derived declaration's *type*.
        let hir = lower_ok(
            "class A:\n    def __init__(self) -> None:\n        self.x = 1\nclass B(A):\n    def __init__(self) -> None:\n        self.x = 1.5\n",
        );
        let layout = flat_attr_layout(&defs_of(&hir, &["B", "A"]));
        assert_eq!(layout.len(), 1);
        assert_eq!(layout[0].0, "x");
        assert_eq!(layout[0].1, crate::Ty::Float);
        // The base alone still sees its own declaration.
        let base = flat_attr_layout(&defs_of(&hir, &["A"]));
        assert_eq!(base[0].1, crate::Ty::Int);
    }

    #[test]
    fn a_single_base_never_trips_the_layout_gate() {
        // `bases.len() < 2` short-circuit: an ancestor's layout is a prefix
        // of its descendant's by construction in a single-inheritance chain,
        // however many levels declare attributes.
        let hir = lower_ok(
            "class A:\n    def __init__(self) -> None:\n        self.a = 1\nclass B(A):\n    def __init__(self) -> None:\n        self.a = 1\n        self.b = 2\nclass C(B):\n    def __init__(self) -> None:\n        self.a = 1\n        self.b = 2\n        self.c = 3\n",
        );
        assert_eq!(
            layout_names(&flat_attr_layout(&defs_of(&hir, &["C", "B", "A"]))),
            vec!["a".to_string(), "b".to_string(), "c".to_string()]
        );
    }

    #[test]
    fn two_bases_declaring_the_same_attribute_name_pass_the_layout_gate() {
        // Every layout in the MRO is `[used]`, so every prefix test is an
        // equality. This is `tests/fixtures/pep_3135_super.py`'s shape.
        let hir = lower_ok(
            "class Slow:\n    def __init__(self) -> None:\n        self.used = 1\nclass Fast:\n    def __init__(self) -> None:\n        self.used = 2\nclass Mixed(Slow, Fast):\n    def __init__(self) -> None:\n        self.used = 3\n",
        );
        assert_eq!(
            layout_names(&flat_attr_layout(&defs_of(
                &hir,
                &["Mixed", "Slow", "Fast"]
            ))),
            vec!["used".to_string()]
        );
    }

    #[test]
    fn a_methods_only_second_base_passes_the_layout_gate() {
        // An empty layout is a prefix of everything, so a mixin that
        // declares no instance attribute is always accepted.
        let hir = lower_ok(
            "class A:\n    def __init__(self) -> None:\n        self.a = 1\nclass Mixin:\n    def f(self) -> int:\n        return 1\nclass C(A, Mixin):\n    def __init__(self) -> None:\n        self.a = 1\n",
        );
        assert!(flat_attr_layout(&defs_of(&hir, &["Mixin"])).is_empty());
        assert_eq!(
            layout_names(&flat_attr_layout(&defs_of(&hir, &["C", "A", "Mixin"]))),
            vec!["a".to_string()]
        );
    }

    #[test]
    fn two_bases_with_diverging_layouts_are_rejected() {
        // The gate itself: `C`'s layout is `[b, a]` (most-base-first over
        // `[C, A, B]`) while `A`'s own is `[a]`, so `A`'s already-lowered
        // methods would address slot 0 -- which now belongs to `b`.
        let module = crate::pycc_parser_test_helper::parse(
            "class A:\n    def __init__(self) -> None:\n        self.a = 1\nclass B:\n    def __init__(self) -> None:\n        self.b = 2\nclass C(A, B):\n    def __init__(self) -> None:\n        self.a = 1\n        self.b = 2\n",
        );
        let diagnostic = lower_checked(&module).unwrap_err();
        assert_eq!(diagnostic.code, "C0001");
        assert!(
            diagnostic
                .message
                .contains("`A`'s instance layout is not a prefix of `C`'s"),
            "unexpected message: {}",
            diagnostic.message
        );
    }

    // -- #432: inheritance, C3 MRO, @override, inherited __init__ -----------

    #[test]
    fn single_inheritance_produces_correct_mro() {
        let hir = lower_ok(
            "class A:\n    def __init__(self) -> None:\n        return\n    def f(self) -> int:\n        return 1\nclass B(A):\n    def __init__(self) -> None:\n        return\n",
        );
        let (_, b_def) = &hir.class_defs[1];
        assert_eq!(b_def.bases, vec!["A".to_string()]);
        assert_eq!(b_def.mro, vec!["B".to_string(), "A".to_string()]);
    }

    #[test]
    fn multiple_inheritance_produces_c3_mro() {
        let hir = lower_ok(
            "class A:\n    def __init__(self) -> None:\n        return\nclass B:\n    def __init__(self) -> None:\n        return\nclass C(A, B):\n    def __init__(self) -> None:\n        return\n",
        );
        let (_, c_def) = &hir.class_defs[2];
        assert_eq!(c_def.bases, vec!["A".to_string(), "B".to_string()]);
        assert_eq!(
            c_def.mro,
            vec!["C".to_string(), "A".to_string(), "B".to_string()]
        );
    }

    #[test]
    fn diamond_inheritance_produces_correct_c3_mro() {
        let hir = lower_ok(
            "class A:\n    def __init__(self) -> None:\n        return\nclass B(A):\n    def __init__(self) -> None:\n        return\nclass C(A):\n    def __init__(self) -> None:\n        return\nclass D(B, C):\n    def __init__(self) -> None:\n        return\n",
        );
        let (_, d_def) = &hir.class_defs[3];
        // C3: D, B, C, A
        assert_eq!(
            d_def.mro,
            vec![
                "D".to_string(),
                "B".to_string(),
                "C".to_string(),
                "A".to_string()
            ]
        );
    }

    #[test]
    fn circular_inheritance_is_rejected() {
        let module = crate::pycc_parser_test_helper::parse(
            "class A(B):\n    def __init__(self) -> None:\n        return\nclass B(A):\n    def __init__(self) -> None:\n        return\n",
        );
        // The first class `A(B)` is rejected because `B` is not yet defined.
        let diagnostic = lower_checked(&module).unwrap_err();
        assert_eq!(diagnostic.code, "C0001");
        assert!(
            diagnostic
                .message
                .contains("inherits from unknown class `B`"),
            "unexpected message: {}",
            diagnostic.message
        );
    }

    #[test]
    fn inheriting_from_a_generic_class_is_rejected() {
        let module = crate::pycc_parser_test_helper::parse(
            "class A[T]:\n    def __init__(self, x: T) -> None:\n        self.x = x\nclass B(A):\n    def __init__(self) -> None:\n        return\n",
        );
        let diagnostic = lower_checked(&module).unwrap_err();
        assert_eq!(diagnostic.code, "C0001");
        assert!(
            diagnostic
                .message
                .contains("cannot inherit from generic class `A`"),
            "unexpected message: {}",
            diagnostic.message
        );
    }

    #[test]
    fn a_class_with_an_unknown_base_is_unsupported() {
        // #432: `class C(Base):` where `Base` is not defined earlier in the
        // module is rejected with C0001 (unknown base class).
        let module = crate::pycc_parser_test_helper::parse(
            "class C(Base):\n    def __init__(self) -> None:\n        return\n",
        );
        let diagnostic = lower_checked(&module).unwrap_err();
        assert_eq!(diagnostic.code, "C0001");
        assert!(
            diagnostic
                .message
                .contains("inherits from unknown class `Base`"),
            "unexpected message: {}",
            diagnostic.message
        );
    }

    #[test]
    fn an_inconsistent_c3_mro_is_rejected() {
        // #432: a classic C3 linearization conflict. `C(A, B)` gives MRO
        // [C, A, B] and `D(B, A)` gives MRO [D, B, A]. `E(C, D)` then has
        // no valid C3 merge: after choosing C and D, the remaining
        // sequences [A, B] and [B, A] have no head that does not appear in
        // the other's tail, so `compute_c3_mro` returns `None`.
        let module = crate::pycc_parser_test_helper::parse(
            "class A:\n    def __init__(self) -> None:\n        return\nclass B:\n    def __init__(self) -> None:\n        return\nclass C(A, B):\n    def __init__(self) -> None:\n        return\nclass D(B, A):\n    def __init__(self) -> None:\n        return\nclass E(C, D):\n    def __init__(self) -> None:\n        return\n",
        );
        let diagnostic = lower_checked(&module).unwrap_err();
        assert_eq!(diagnostic.code, "C0001");
        assert!(
            diagnostic
                .message
                .contains("inconsistent method resolution order (MRO)"),
            "unexpected message: {}",
            diagnostic.message
        );
    }

    #[test]
    fn circular_inheritance_in_mro_is_rejected() {
        // #432: circular inheritance (A's MRO contains B, and B inherits
        // from A) is impossible through normal source-order processing --
        // a base class must be defined before the derived class that
        // inherits from it, so A's MRO can never contain B before B is
        // even defined. This test bypasses `module::lower_all` (which processes
        // classes in source order) and calls `lower_class` directly with a
        // hand-built `defined_classes` whose "A" entry already lists "B"
        // in its MRO, exercising the defensive circular-inheritance check
        // that is otherwise unreachable from any real source program.
        //
        // The `find_map` includes a leading non-class statement so its
        // `_ => None` arm is exercised (not just the `ClassDef` arm),
        // matching this file's own established coverage-gate convention
        // (see e.g. `a_subclass_clone_body_is_substituted` above).
        let module = crate::pycc_parser_test_helper::parse(
            "def _dummy() -> None:\n    return\nclass B(A):\n    def __init__(self) -> None:\n        return\n",
        );
        let def = module
            .body
            .iter()
            .find_map(|stmt| match stmt {
                pycc_ast::Stmt::ClassDef(def) => Some(def),
                _ => None,
            })
            .expect("test fixture must contain a class definition");
        let fake_a = HirClassDef {
            class_attrs: Vec::new(),
            exception_type_tag: None,
            name: "A".to_string(),
            bases: Vec::new(),
            mro: vec!["A".to_string(), "B".to_string()],
            attrs: vec![],
            methods: vec![("__init__".to_string(), "A.__init__".to_string())],
            type_param: None,
            properties: Vec::new(),
            static_methods: Vec::new(),
            class_methods: Vec::new(),
            is_enum: false,
            implicit_object_init: false,
            method_defaults: Vec::new(),
            enum_members: Vec::new(),
            is_dataclass: false,
            dataclass_fields: Vec::new(),
            is_protocol: false,
            runtime_checkable: false,
            protocol_members: Vec::new(),
            abstract_methods: Vec::new(),
            is_abstract: false,
        };
        let defined_classes = vec![("A".to_string(), fake_a)];
        let diagnostic = crate::class::lower_class(
            def,
            &[],
            &defined_classes,
            &[],
            &[],
            &[],
            &[],
            &crate::expr::keyword_bind::SignatureTable::default(),
            false,
        )
        .unwrap_err();
        assert_eq!(diagnostic.code, "C0001");
        assert!(
            diagnostic
                .message
                .contains("circular inheritance is not supported"),
            "unexpected message: {}",
            diagnostic.message
        );
    }

    // -- #941: an enum class cannot be used as a base --------------------------

    /// Asserts the `C0001` `validate_bases` reports for `source` carries
    /// `expected` and spans the offending class header (`header`).
    fn assert_enum_base_rejected(source: &str, header: &str, expected: &str) {
        let module = crate::pycc_parser_test_helper::parse(source);
        let diagnostic = lower_checked(&module).unwrap_err();
        assert_eq!(diagnostic.code, "C0001");
        assert!(
            diagnostic.message.contains(expected),
            "unexpected message: {}",
            diagnostic.message
        );
        let start = source
            .find(header)
            .expect("the fixture contains the class header");
        let span = diagnostic
            .span
            .expect("`validate_bases` spans the class header");
        assert_eq!(
            span.start as usize, start,
            "the diagnostic must start at the class header, got: {span:?}"
        );
    }

    #[test]
    fn inheriting_from_an_enum_with_members_names_cpythons_type_error() {
        // The issue's own shape: `Foo(Color)` where `Color` has members.
        // CPython 3.14 raises `TypeError: <enum 'Foo'> cannot extend
        // <enum 'Color'>` for the same class statement.
        assert_enum_base_rejected(
            "class Color(Enum):\n    RED = 1\nclass Foo(Color):\n    pass\n",
            "class Foo(Color)",
            "class `Foo` cannot inherit from enum class `Color` -- CPython raises `TypeError: \
             <enum 'Foo'> cannot extend <enum 'Color'>`",
        );
    }

    #[test]
    fn inheriting_from_a_member_less_enum_is_not_supported_yet() {
        // `class Base(Enum): pass` is already rejected by `lower_enum_class`
        // (an enum body must contain only member assignments), so the only
        // reachable member-less enum is the docstring-only one (#744).
        // CPython allows extending it, which is why the wording is "not
        // supported yet" rather than CPython's own error.
        assert_enum_base_rejected(
            "class Base(Enum):\n    \"A base.\"\nclass Color(Base):\n    RED = 1\n",
            "class Color(Base)",
            "class `Color` cannot inherit from member-less enum class `Base` -- extending an \
             enum class that has no members is not supported yet",
        );
    }

    #[test]
    fn inheriting_from_a_str_enum_is_rejected_too() {
        // `StrEnum` shares `lower_enum_class` (#892) and therefore the
        // `is_enum` marker the check keys on.
        assert_enum_base_rejected(
            "class Kind(StrEnum):\n    AXIAL = \"axial\"\nclass Sub(Kind):\n    pass\n",
            "class Sub(Kind)",
            "class `Sub` cannot inherit from enum class `Kind`",
        );
    }

    #[test]
    fn a_grand_subclass_of_an_enum_is_rejected_at_the_first_subclass() {
        // `Bar(Foo)` never gets checked: lowering stops at `Foo(Color)`,
        // the first class that names the enum as a base.
        assert_enum_base_rejected(
            "class Color(Enum):\n    RED = 1\nclass Foo(Color):\n    pass\nclass Bar(Foo):\n    pass\n",
            "class Foo(Color)",
            "class `Foo` cannot inherit from enum class `Color`",
        );
    }

    #[test]
    fn an_enum_base_listed_after_an_ordinary_base_is_rejected() {
        // The check runs per base, so the enum need not be the first one.
        assert_enum_base_rejected(
            "class A:\n    def __init__(self) -> None:\n        return\nclass Color(Enum):\n    RED = 1\nclass Foo(A, Color):\n    pass\n",
            "class Foo(A, Color)",
            "class `Foo` cannot inherit from enum class `Color`",
        );
    }

    #[test]
    fn a_non_enum_base_beside_an_enum_class_still_lowers() {
        // The negative: an ordinary base is unaffected by the new check
        // even when an enum class is defined in the same module.
        let hir = lower_ok(
            "class Color(Enum):\n    RED = 1\nclass A:\n    def __init__(self) -> None:\n        return\nclass B(A):\n    pass\n",
        );
        let (_, b_def) = &hir.class_defs[2];
        assert_eq!(b_def.bases, vec!["A".to_string()]);
        assert!(!b_def.is_enum);
    }
    // -- Part 1 of #1283: a builtin-type base -----------------------------------

    /// The `C0001` message `lower_checked` reports for `class C(<base>): pass`.
    fn base_rejection(base: &str) -> String {
        let source = format!("class C({base}):\n    pass\n");
        let module = crate::pycc_parser_test_helper::parse(&source);
        let diagnostic = lower_checked(&module).unwrap_err();
        assert_eq!(diagnostic.code, "C0001");
        diagnostic.message
    }

    #[test]
    fn every_subclassable_builtin_type_gets_the_builtin_base_message() {
        for base in [
            "int",
            "float",
            "complex",
            "str",
            "bytes",
            "bytearray",
            "list",
            "tuple",
            "dict",
            "set",
            "frozenset",
        ] {
            assert_eq!(
                base_rejection(base),
                crate::module::builtin_base_message("C", base)
            );
        }
    }

    #[test]
    fn object_refused_builtin_types_and_functions_keep_the_unknown_class_message() {
        // `object` is excluded because the honest fix is to accept it;
        // `bool`, `range`, `slice` and `memoryview` are the four types CPython
        // refuses as a base; `len` is a builtin function, not a type.
        for base in ["object", "bool", "range", "slice", "memoryview", "len"] {
            assert_eq!(
                base_rejection(base),
                crate::module::unknown_base_message("C", base)
            );
        }
    }
}
