//! Part 1 of #1367: a name a foreign import binds is spellable in an
//! annotation, as the opaque `Ty::Object` (`docs/TYPE_SYSTEM.md`, the
//! `object` row). A subscript on it is erased and its arguments are never
//! resolved, and in a native module `object` itself stays unspellable
//! (Part 3, #1387); an `--ext` module spells it under D-258 (#1397,
//! `annotations_ext.rs`).

use super::from_foreign::{lower_foreign, lower_relative, only_error};
use super::*;
use crate::{HirItem, HirStmt, Ty};

fn lower_ok(source: &str, foreign: &[&str]) -> LoweredModule {
    lower_foreign(source, foreign)
        .unwrap_or_else(|diagnostics| panic!("{source:?} must lower: {diagnostics:#?}"))
}

/// The `(params, return_ty)` of the module-level function `name`.
fn signature(module: &LoweredModule, name: &str) -> (Vec<(String, Ty)>, Ty) {
    module
        .hir
        .items
        .iter()
        .find_map(|item| match item {
            HirItem::Function {
                name: n,
                params,
                return_ty,
                ..
            } if n == name => Some((params.clone(), return_ty.clone())),
            _ => None,
        })
        .unwrap_or_else(|| panic!("no function `{name}`"))
}

fn attrs(module: &LoweredModule, class: &str) -> Vec<(String, Ty)> {
    module
        .hir
        .class_defs
        .iter()
        .find(|(name, _)| name == class)
        .map(|(_, def)| def.attrs.clone())
        .unwrap_or_else(|| panic!("no class `{class}`"))
}

fn object_params(names: &[&str]) -> Vec<(String, Ty)> {
    names
        .iter()
        .map(|name| ((*name).to_string(), Ty::Object))
        .collect()
}

#[test]
fn a_foreign_class_annotates_a_parameter_a_return_and_a_local() {
    let module = lower_ok(
        "from fractions import Fraction\n\
         def _pick(f: Fraction) -> Fraction:\n    x: Fraction = f\n    return x\n",
        &["fractions"],
    );
    let (params, return_ty) = signature(&module, "_pick");
    assert_eq!(params, object_params(&["f"]));
    assert_eq!(return_ty, Ty::Object);
    let HirItem::Function { body, .. } = &module.hir.items[0] else {
        panic!("the function is the first item");
    };
    assert!(
        matches!(&body[0], HirStmt::AnnAssign { target, annotation: Ty::Object, .. } if target == "x"),
        "{body:#?}"
    );
}

#[test]
fn a_foreign_class_annotates_a_module_level_assignment() {
    let module = lower_ok(
        "from fractions import Fraction\nx: Fraction = Fraction(1, 2)\n",
        &["fractions"],
    );
    assert!(
        module.hir.items.iter().any(|item| matches!(
            item,
            HirItem::TopLevelStmt(HirStmt::AnnAssign { target, annotation: Ty::Object, .. })
                if target == "x"
        )),
        "{:#?}",
        module.hir.items
    );
}

/// A subscript on a foreign class is erased without resolving its
/// arguments: `Queue[Nope]` names nothing, and CPython 3.14 never
/// evaluates it (PEP 649).
#[test]
fn a_subscripted_foreign_class_erases_its_arguments() {
    let module = lower_ok(
        "from queue import Queue\n\
         def _a(q: Queue[int]) -> int:\n    return 1\n\
         def _b(q: Queue[Nope]) -> int:\n    return 1\n",
        &["queue"],
    );
    assert_eq!(signature(&module, "_a").0, object_params(&["q"]));
    assert_eq!(signature(&module, "_b").0, object_params(&["q"]));
}

/// The annotation resolves through the import that precedes it in source
/// order, as every other alias-table name does.
#[test]
fn an_annotation_above_its_import_keeps_its_c0001() {
    let diagnostic = only_error(lower_foreign(
        "def _f(t: Fraction) -> int:\n    return 1\nfrom fractions import Fraction\n",
        &["fractions"],
    ));
    assert_eq!(diagnostic.code, "C0001");
    assert_eq!(
        diagnostic.message,
        "type annotation `Fraction` is not supported yet"
    );
}

/// The import's alias entry never reaches `HirModule::type_aliases`, so the
/// name neither re-exports nor leaks; a D-135 alias built from it is an
/// ordinary alias of `object`.
#[test]
fn the_foreign_name_is_not_a_type_alias_but_an_alias_of_it_is() {
    let module = lower_ok(
        "from fractions import Fraction\ntype F = Fraction\n\
         def _f(t: F) -> F:\n    return t\n",
        &["fractions"],
    );
    assert_eq!(module.hir.type_aliases, vec![("F".to_string(), Ty::Object)]);
    assert_eq!(
        signature(&module, "_f"),
        (object_params(&["t"]), Ty::Object)
    );
}

/// A plain `import X` binds a foreign name too, and CPython accepts the
/// module object as an annotation, so pycc does as well.
#[test]
fn a_plain_foreign_import_s_name_annotates_as_object() {
    let module = lower_ok(
        "import fractions\ndef _f(t: fractions) -> int:\n    return 1\n",
        &["fractions"],
    );
    assert_eq!(signature(&module, "_f").0, object_params(&["t"]));
}

/// A block-scoped `import X` never passes through the module-level import
/// arm, so its name stays unspellable as an annotation (CPython accepts it;
/// out of scope for Part 1).
#[test]
fn a_block_scoped_import_s_name_keeps_its_c0001() {
    let diagnostic = only_error(lower_foreign(
        "try:\n    import fractions\nexcept ImportError:\n    pass\n\
         def _f(t: fractions) -> int:\n    return 1\n",
        &["fractions"],
    ));
    assert_eq!(
        diagnostic.message,
        "type annotation `fractions` is not supported yet"
    );
}

/// #1366 and #1138's channels bind foreign names as well: a sibling's class
/// and a sibling's `TypeVar` both resolve to `object`.
#[test]
fn the_relative_and_dotted_channels_annotate_as_object() {
    let module = lower_relative(
        "from .sib import Token, StateT\nfrom json.decoder import JSONDecoder\n\
         def _f(t: Token, s: StateT, d: JSONDecoder) -> StateT:\n    return s\n",
        &["json.decoder"],
    )
    .expect("must lower");
    assert_eq!(
        signature(&module, "_f"),
        (object_params(&["t", "s", "d"]), Ty::Object)
    );
}

#[test]
fn a_declared_foreign_attribute_and_an_undeclared_one_get_object_slots() {
    let module = lower_ok(
        "from fractions import Fraction\nfrom queue import Queue\n\
         class _Box:\n    f: Fraction\n    q: Queue[int]\n\
         \x20   def __init__(self, f: Fraction, q: Queue[int]) -> None:\n\
         \x20       self.f = f\n        self.q = q\n        self.g = f\n",
        &["fractions", "queue"],
    );
    assert_eq!(attrs(&module, "_Box"), object_params(&["f", "q", "g"]));
}

/// #1388: a declared foreign attribute is established from any expression,
/// a foreign call or a read through a slot `__init__` has not assigned yet
/// alike -- the latter raises `AttributeError` at run time, as in CPython.
#[test]
fn a_declared_foreign_attribute_is_established_from_any_expression() {
    for rhs in ["Fraction(n, 4)", "self.b.limit_denominator(3)"] {
        let source = format!(
            "from fractions import Fraction\n\
             class _Box:\n    f: Fraction\n\
             \x20   def __init__(self, n: int, b: Fraction) -> None:\n\
             \x20       self.f = {rhs}\n        self.b = b\n"
        );
        let module = lower_ok(&source, &["fractions"]);
        assert_eq!(attrs(&module, "_Box"), object_params(&["f", "b"]), "{rhs}");
    }
}

/// The positions Part 1 does not widen keep their refusals, so a later
/// widening is deliberate: a dataclass field, a class constant, a protocol
/// member (which resolves without the alias table), and `object` itself.
#[test]
fn the_positions_part_1_does_not_widen_keep_their_refusals() {
    for (body, needle) in [
        (
            "from dataclasses import dataclass\n@dataclass\nclass _D:\n    f: Fraction\n",
            "dataclass field `f` has type `object`",
        ),
        (
            "class _C:\n    f: Fraction = Fraction(1, 2)\n",
            "class attribute `f` has type `object`",
        ),
        (
            "from typing import Protocol\nclass _P(Protocol):\n    f: Fraction\n",
            "type annotation `Fraction` is not supported yet",
        ),
        (
            "def _f(t: object) -> int:\n    return 1\n",
            "type annotation `object` is not supported yet",
        ),
    ] {
        let source = format!("from fractions import Fraction\n{body}");
        let diagnostic = only_error(lower_foreign(&source, &["fractions"]));
        assert_eq!(diagnostic.code, "C0001", "{body}");
        assert!(
            diagnostic.message.starts_with(needle),
            "{body}: {}",
            diagnostic.message
        );
    }
}

/// The alias entry the import records is not a type alias the source wrote,
/// so a class of the same name keeps the import-collision message.
#[test]
fn a_class_named_like_a_foreign_import_collides_with_the_import() {
    let diagnostic = only_error(lower_foreign(
        "from fractions import Fraction\nclass Fraction:\n    pass\n",
        &["fractions"],
    ));
    assert_eq!(
        diagnostic.message,
        "class `Fraction` collides with an import of the same name already defined in this \
         module"
    );
}

/// A second foreign import of the same name records no second alias entry.
#[test]
fn a_repeated_foreign_import_records_one_alias_entry() {
    let module = lower_ok(
        "from fractions import Fraction\nfrom fractions import Fraction\n\
         def _f(t: Fraction) -> int:\n    return 1\n",
        &["fractions"],
    );
    assert_eq!(signature(&module, "_f").0, object_params(&["t"]));
}

/// A type alias spelled before a foreign import of the same name stays the
/// name's alias-table entry, but the module-wide foreign-shadow check that
/// runs after the per-item loop refuses the rebinding (`C0001`), so the
/// lowering it typed with the stale alias is discarded rather than returned.
#[test]
fn a_type_alias_before_a_foreign_import_of_its_name_is_refused() {
    let diagnostic = only_error(lower_foreign(
        "type Fraction = int\n\
         from fractions import Fraction\n\
         def _f(t: Fraction) -> int:\n    return 1\n",
        &["fractions"],
    ));
    assert_eq!(diagnostic.code, "C0001");
    assert!(
        diagnostic
            .message
            .contains("shadowing a foreign import is not supported yet"),
        "{}",
        diagnostic.message
    );
}

/// A foreign name nested in a legacy `typing` container (#1378) resolves
/// as an element through the alias table, so `Dict[str, C]` is a
/// `dict[str, object]` and meets the same element-type refusal
/// (`T0036`) as the lowercase `dict[str, C]` spelling, rather than an
/// erasure of the container itself.
#[test]
fn a_foreign_name_inside_a_legacy_typing_container_is_an_object_element() {
    let diagnostic = only_error(lower_foreign(
        "from typing import Dict\n\
         from fractions import Fraction\n\
         def _a(d: Dict[str, Fraction]) -> int:\n    return 1\n",
        &["fractions"],
    ));
    assert_eq!(diagnostic.code, "T0036");
    assert!(
        diagnostic
            .message
            .starts_with("dict[str, object] is not compiled yet"),
        "{}",
        diagnostic.message
    );
}

/// A foreign import whose local name is a legacy `typing` container
/// spelling (`Dict`) is refused at the import itself, so no alias entry
/// can shadow the container: the annotation keeps meeting the legacy
/// container's own arity check.
#[test]
fn a_foreign_import_named_like_a_typing_container_is_refused_at_the_import() {
    let diagnostics = lower_foreign(
        "from somelib import Dict\n\
         def _a(d: Dict[int]) -> int:\n    return 1\n",
        &["somelib"],
    )
    .expect_err("fixture must fail to lower");
    let codes: Vec<&str> = diagnostics.iter().map(|d| d.code).collect();
    assert_eq!(codes, ["C0001", "T0053"], "{diagnostics:#?}");
    assert!(
        diagnostics[0]
            .message
            .contains("a name pycc resolves by its spelling"),
        "{}",
        diagnostics[0].message
    );
}
