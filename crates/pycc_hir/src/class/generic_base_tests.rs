//! Unit tests for `class/generic_base.rs` and `module/type_var.rs` (Part 1
//! of #886, #1394): a `Generic[...]` base whose arguments are type variables
//! is erased, and every other shape keeps an explicit `C0001`.

use crate::pycc_parser_test_helper::parse;
use crate::{
    HirClassDef, HirItem, LoweredModule, ResolvedImport, ResolvedImports, Ty, lower_checked,
    lower_module, project_import_requests,
};
use pycc_diag::Diagnostic;

const TYPING: &str = "from typing import Generic, TypeVar\nT = TypeVar(\"T\")\n";

fn lower_ok(source: &str) -> crate::HirModule {
    lower_checked(&parse(source)).expect("fixture should lower")
}

fn lower_err(source: &str) -> Diagnostic {
    lower_checked(&parse(source)).expect_err("fixture should be refused")
}

/// Lowers `source` answering every relative import as foreign, as `pycc
/// build --ext --foreign-relative-imports` does, plus every absolute import
/// of a module in `foreign`.
fn lower_with_foreign(source: &str, foreign: &[&str]) -> Result<LoweredModule, Vec<Diagnostic>> {
    let parsed = parse(source);
    let mut resolved = ResolvedImports::default();
    for request in project_import_requests(&parsed) {
        if request.level > 0
            || request
                .module
                .as_deref()
                .is_some_and(|module| foreign.contains(&module))
        {
            resolved.insert(request.span, ResolvedImport::Foreign);
        }
    }
    lower_module(&parsed, &resolved, None)
}

fn class<'a>(hir: &'a crate::HirModule, name: &str) -> &'a HirClassDef {
    hir.class_defs
        .iter()
        .find(|(class, _)| class == name)
        .map(|(_, def)| def)
        .expect("class is defined")
}

/// Asserts `source` is refused with a `C0001` whose message is `expected`.
fn assert_refused(source: &str, expected: &str) {
    let diagnostic = lower_err(source);
    assert_eq!(diagnostic.code, "C0001", "{source:?}");
    assert_eq!(diagnostic.message, expected, "{source:?}");
}

const BARE_NAME: &str = "a base class must be a bare name (e.g. `class C(Base):`), not an \
                         attribute access or other expression";

#[test]
fn a_generic_base_over_a_local_type_var_is_erased() {
    let hir = lower_ok(&format!(
        "{TYPING}class C(Generic[T]):\n    def __init__(self, x: int) -> None:\n        \
         self.x = x\n    def get(self, t: T) -> T:\n        return t\n"
    ));
    let c = class(&hir, "C");
    assert!(c.bases.is_empty(), "{:?}", c.bases);
    assert_eq!(c.mro, vec!["C".to_string()]);
    // The declaration is compile-time only: no item, and its `object` alias
    // never leaves the module.
    assert!(hir.type_aliases.iter().all(|(name, _)| name != "T"));
    assert!(
        !hir.items
            .iter()
            .any(|item| matches!(item, HirItem::TopLevelStmt(_)))
    );
    let get = hir
        .items
        .iter()
        .find_map(|item| match item {
            HirItem::Function {
                name,
                params,
                return_ty,
                ..
            } if name == "C.get" => Some((params.clone(), return_ty.clone())),
            _ => None,
        })
        .expect("C.get is lowered");
    assert_eq!(get.0[1], ("t".to_string(), Ty::Object));
    assert_eq!(get.1, Ty::Object);
}

#[test]
fn several_type_variables_and_a_real_base_keep_only_the_real_base() {
    let hir = lower_ok(
        "from typing import Generic, TypeVar\nK = TypeVar(\"K\")\nV = TypeVar(\"V\")\n\
         class Base:\n    def __init__(self) -> None:\n        return\n\
         class Pair(Base, Generic[K, V]):\n    pass\n",
    );
    let pair = class(&hir, "Pair");
    assert_eq!(pair.bases, vec!["Base".to_string()]);
    assert_eq!(pair.mro, vec!["Pair".to_string(), "Base".to_string()]);
}

#[test]
fn the_typing_attribute_spellings_are_recognized() {
    let hir = lower_ok(
        "import typing\nT = typing.TypeVar(\"T\")\n\
         class C(typing.Generic[T]):\n    pass\n",
    );
    assert!(class(&hir, "C").bases.is_empty());
}

#[test]
fn a_generic_protocol_stays_a_protocol() {
    let hir = lower_ok(&format!(
        "from typing import Protocol\n{TYPING}\
         class P(Protocol, Generic[T]):\n    def get(self) -> int: ...\n"
    ));
    assert!(class(&hir, "P").is_protocol);
}

#[test]
fn a_foreign_imported_type_variable_is_admitted() {
    // The subject module's shape: `StateT` comes from a sibling module as a
    // CPython object, which pycc takes on trust as a type variable.
    let lowered = lower_with_foreign(
        "from typing import Generic\nfrom .lalr_analysis import StateT\n\
         class ParseConf(Generic[StateT]):\n    def __init__(self, start: str) -> None:\n        \
         self.start = start\n",
        &[],
    )
    .expect("fixture should lower");
    assert!(class(&lowered.hir, "ParseConf").bases.is_empty());
    // An absolute foreign import is taken on trust too: pycc cannot see
    // that `Fraction` is not a `TypeVar`, so this compiles, where CPython
    // raises a `TypeError` at class creation (the documented deviation).
    lower_with_foreign(
        "from typing import Generic\nfrom fractions import Fraction\n\
         class C(Generic[Fraction]):\n    pass\n",
        &["fractions"],
    )
    .expect("fixture should lower");
}

#[test]
fn a_plain_generic_base_is_refused() {
    assert_refused(
        "from typing import Generic\nclass C(Generic):\n    pass\n",
        "class `C` lists a plain `Generic` base -- write `Generic[T]` with the class's type \
         variables",
    );
    assert_refused(
        "import typing\nclass C(typing.Generic):\n    pass\n",
        "class `C` lists a plain `Generic` base -- write `Generic[T]` with the class's type \
         variables",
    );
}

#[test]
fn an_argument_that_is_not_a_type_variable_is_refused() {
    assert_refused(
        &format!("{TYPING}class C(Generic[int]):\n    pass\n"),
        "`int` in a `Generic[...]` base is not a type variable -- declare it at module level as \
         `int = TypeVar(\"int\")` or import it",
    );
    assert_refused(
        &format!("{TYPING}class C(Generic[list[T]]):\n    pass\n"),
        "an argument of a `Generic[...]` base must be the bare name of a type variable (a \
         module-level `T = TypeVar(\"T\")` or a foreign-imported name)",
    );
    assert_refused(
        &format!("{TYPING}class C(Generic[()]):\n    pass\n"),
        "a `Generic[...]` base must name at least one type variable",
    );
    assert_refused(
        &format!("{TYPING}class C(Generic[T, T]):\n    pass\n"),
        "type variable `T` appears more than once in a `Generic[...]` base",
    );
}

#[test]
fn a_second_generic_base_and_a_pep_695_class_are_refused() {
    assert_refused(
        &format!("{TYPING}U = TypeVar(\"U\")\nclass C(Generic[T], Generic[U]):\n    pass\n"),
        "class `C` lists `Generic[...]` more than once -- a class may inherit from \
         `Generic[...]` only once",
    );
    assert_refused(
        &format!("{TYPING}class C[U](Generic[T]):\n    pass\n"),
        "class `C` declares type parameters (`class C[T]:`) and also lists a `Generic[...]` \
         base -- a PEP 695 generic class is already generic, so drop the `Generic[...]` base",
    );
}

#[test]
fn every_other_non_name_base_keeps_the_bare_name_refusal() {
    for source in [
        // Not imported: `Generic` is never resolved by spelling.
        "class C(Generic[T]):\n    pass\n".to_string(),
        // A user class's subscript is still #886.
        format!("{TYPING}class B:\n    pass\nclass C(B[T]):\n    pass\n"),
        // An attribute chain ending in `Generic` that is not `typing`'s.
        format!("{TYPING}class C(a.b.Generic[T]):\n    pass\n"),
        "class C(f()):\n    pass\n".to_string(),
    ] {
        assert_refused(&source, BARE_NAME);
    }
}

#[test]
fn a_type_var_declaration_outside_the_plain_form_is_refused() {
    let typing = "from typing import TypeVar\n";
    assert_refused(
        &format!("{typing}T = TypeVar(\"U\")\n"),
        "`TypeVar(\"U\")` is bound to `T` -- a type variable's name must match the variable it \
         is assigned to",
    );
    let plain = "only the plain `T = TypeVar(\"T\")` form is supported -- a constrained or \
                 bounded `TypeVar` (further arguments or keywords) is not supported yet";
    for declaration in [
        "T = TypeVar(\"T\", int, str)\n",
        "T = TypeVar(\"T\", bound=int)\n",
        "T = TypeVar(name)\n",
    ] {
        assert_refused(&format!("{typing}{declaration}"), plain);
    }
    assert_refused(
        &format!("{typing}T = U = TypeVar(\"T\")\n"),
        "a `TypeVar(...)` must be bound to a single name (`T = TypeVar(\"T\")`)",
    );
    assert_refused(
        &format!("{typing}type T = int\nT = TypeVar(\"T\")\n"),
        "type variable `T` rebinds a name already bound as a type in this module",
    );
    assert_refused(
        &format!("{typing}class T:\n    pass\nT = TypeVar(\"T\")\n"),
        "type variable `T` collides with a class of the same name already defined in this module",
    );
}

/// #1388: a `Generic[T]` class is not a PEP 695 generic class -- its `T`
/// resolves to `object`, never `Ty::Param` -- so its declarations admit any
/// establishing expression, which is what lets `declared_slot_ty` keep the
/// shape gate to `type_param.is_some()` classes alone.
#[test]
fn a_generic_base_class_declares_object_slots_from_any_expression() {
    let hir = lower_ok(&format!(
        "{TYPING}class C(Generic[T]):\n    x: T\n    n: int\n\
         \x20   def __init__(self, v: T, k: int) -> None:\n        self.x = v\n        \
         self.n = k + 1\n"
    ));
    let c = class(&hir, "C");
    assert!(c.type_param.is_none());
    assert_eq!(
        c.attrs,
        vec![("x".to_string(), Ty::Object), ("n".to_string(), Ty::Int)]
    );
}
