//! D-258 (#1397): in a module compiled into an `ext` artifact, `Any`,
//! `object`, a bare `list`/`dict`/`tuple`/`set`, and a container with an
//! object-typed argument all lower to the opaque `Ty::Object`. The same
//! source lowered for a `native` artifact keeps today's diagnostics, byte
//! for byte.

use super::*;
use crate::{HirItem, HirStmt, Ty};
use pycc_diag::Diagnostic;

/// Lowers `source` as an `ext` module (`ext == true`) or a `native` one,
/// with every import of a module named in `foreign` bound to a CPython
/// object.
fn lower(source: &str, foreign: &[&str], ext: bool) -> Result<LoweredModule, Vec<Diagnostic>> {
    let parsed = parse(source);
    let mut resolved = ResolvedImports::default();
    resolved.set_ext_module(ext);
    for request in project_import_requests(&parsed) {
        if request
            .module
            .as_deref()
            .is_some_and(|module| foreign.contains(&module))
        {
            resolved.insert(request.span, ResolvedImport::Foreign);
        }
    }
    lower_module(&parsed, &resolved, None)
}

fn lower_ext(source: &str) -> LoweredModule {
    lower(source, &["fractions"], true)
        .unwrap_or_else(|diagnostics| panic!("{source:?} must lower: {diagnostics:#?}"))
}

fn only_error(source: &str, ext: bool) -> Diagnostic {
    let diagnostics = lower(source, &["fractions"], ext).expect_err("fixture must fail to lower");
    assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
    diagnostics.into_iter().next().expect("one diagnostic")
}

/// The `(param types, return_ty)` of the module-level function `name`.
fn signature(module: &LoweredModule, name: &str) -> (Vec<Ty>, Ty) {
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
            } if n == name => Some((
                params.iter().map(|(_, ty)| ty.clone()).collect(),
                return_ty.clone(),
            )),
            _ => None,
        })
        .unwrap_or_else(|| panic!("no function `{name}`"))
}

const TYPING: &str = "from typing import Any, Dict, List, Tuple\n";

#[test]
fn any_and_object_lower_to_the_object_in_a_parameter_a_return_and_a_local() {
    let module = lower_ext(&format!(
        "{TYPING}def _f(x: Any, y: object) -> Any:\n    z: Any = x\n    w: object = y\n    return z\n"
    ));
    let (params, return_ty) = signature(&module, "_f");
    assert_eq!(params, vec![Ty::Object, Ty::Object]);
    assert_eq!(return_ty, Ty::Object);
    let HirItem::Function { body, .. } = module
        .hir
        .items
        .iter()
        .find(|item| matches!(item, HirItem::Function { .. }))
        .expect("the function")
    else {
        unreachable!()
    };
    for (index, name) in [(0, "z"), (1, "w")] {
        assert!(
            matches!(&body[index], HirStmt::AnnAssign { target, annotation: Ty::Object, .. } if target == name),
            "{body:#?}"
        );
    }
}

#[test]
fn the_four_bare_containers_lower_to_the_object() {
    let module = lower_ext("def _f(a: list, b: dict, c: tuple, d: set) -> list:\n    return a\n");
    let (params, return_ty) = signature(&module, "_f");
    assert_eq!(params, vec![Ty::Object; 4]);
    assert_eq!(return_ty, Ty::Object);
}

#[test]
fn a_container_with_an_object_argument_collapses_to_the_object() {
    let module = lower_ext(&format!(
        "{TYPING}from fractions import Fraction\n\
         def _f(a: List[Fraction], b: Dict[str, Any], c: tuple[int, Any], \
         d: dict[str, Dict[str, tuple]], e: set[object]) -> List[Any]:\n    return a\n"
    ));
    let (params, return_ty) = signature(&module, "_f");
    assert_eq!(params, vec![Ty::Object; 5]);
    assert_eq!(return_ty, Ty::Object);
}

#[test]
fn a_container_of_native_types_is_unchanged_in_an_ext_module() {
    let module = lower_ext(&format!(
        "{TYPING}def _f(a: List[int], b: Dict[str, int]) -> Tuple[int, float]:\n    return (1, 1.0)\n"
    ));
    let (params, return_ty) = signature(&module, "_f");
    assert_eq!(
        params,
        vec![
            Ty::List(Box::new(Ty::Int)),
            Ty::Dict(Box::new((Ty::Str, Ty::Int)))
        ]
    );
    assert_eq!(return_ty, Ty::Tuple(Box::new(vec![Ty::Int, Ty::Float])));
}

#[test]
fn a_class_body_declaration_and_its_init_assignment_are_the_object() {
    let module = lower_ext(&format!(
        "{TYPING}class _C:\n    v: Any\n    w: list\n\
         \x20   def __init__(self, v: Any, w: list) -> None:\n\
         \x20       self.v = v\n        self.w = w\n"
    ));
    let attrs = module
        .hir
        .class_defs
        .iter()
        .find(|(name, _)| name == "_C")
        .map(|(_, def)| def.attrs.clone())
        .expect("the class");
    assert_eq!(
        attrs,
        vec![("v".to_string(), Ty::Object), ("w".to_string(), Ty::Object)]
    );
}

/// D-258 rule 4 names exactly four bare spellings: a bare `frozenset` and
/// the bare legacy aliases keep their `C0001`, as does a type parameter
/// inside a container (`T0042` precedes the collapse).
#[test]
fn the_spellings_outside_rule_4_keep_their_refusals_in_an_ext_module() {
    for (source, code, needle) in [
        (
            "def _f(a: frozenset) -> int:\n    return 1\n".to_string(),
            "C0001",
            "a bare `frozenset`",
        ),
        (
            format!("{TYPING}def _f(a: List) -> int:\n    return 1\n"),
            "C0001",
            "a bare `List`",
        ),
        (
            "def _f[T](a: list[T], b: Any) -> int:\n    return 1\n"
                .replace("def _f", "from typing import Any\ndef _f"),
            "T0042",
            "type parameter `T` used inside a container position",
        ),
    ] {
        let error = only_error(&source, true);
        assert_eq!(error.code, code, "{source}: {error:#?}");
        assert!(
            error.message.contains(needle),
            "{source}: {}",
            error.message
        );
    }
}

/// `Any` and `object` are not generic, so a subscript on either is the
/// `T0044` every other non-subscriptable base gets, not an erased subscript
/// like a foreign class's.
#[test]
fn a_subscripted_any_or_object_is_t0044_in_an_ext_module() {
    for (annotation, noun) in [
        ("Any[int]", "`Any`"),
        ("object[int]", "builtin type `object`"),
    ] {
        let error = only_error(
            &format!("{TYPING}def _f(a: {annotation}) -> int:\n    return 1\n"),
            true,
        );
        assert_eq!(error.code, "T0044", "{annotation}: {error:#?}");
        assert!(
            error
                .message
                .starts_with(&format!("{noun} is not subscriptable")),
            "{annotation}: {}",
            error.message
        );
    }
}

/// A subscript on an *alias* of `Any` or `object` (`type Foo = Any`,
/// `Bar: TypeAlias = object`) is erased to the object, not refused with
/// `T0044`. The alias table records only the resolved `Ty::Object`, the
/// same entry a foreign class's name gets, so the subscript arm cannot tell
/// the two apart. This pins the current behaviour; it is not a miscompile.
/// CPython 3.14 never evaluates the annotation (PEP 649), so `Foo[int]`
/// accepts any object there too, which is exactly what the erased
/// `Ty::Object` admits.
#[test]
fn a_subscripted_alias_of_any_or_object_is_erased_to_the_object() {
    let module = lower_ext(&format!(
        "{TYPING}type Foo = Any\nBar: TypeAlias = object\n\
         def _f(a: Foo[int], b: Bar[str]) -> Foo[int]:\n    return a\n"
    ));
    let (params, return_ty) = signature(&module, "_f");
    assert_eq!(params, vec![Ty::Object, Ty::Object]);
    assert_eq!(return_ty, Ty::Object);
}

/// A module's own `object` or `list` shadows the builtin, as in Python:
/// the builtin spelling is resolved after the class table and the aliases.
#[test]
fn a_module_s_own_object_or_list_shadows_the_builtin_in_an_ext_module() {
    let module = lower_ext(
        "type object = int\nclass list:\n    pass\n\
         def _f(a: object, b: list) -> object:\n    return a\n",
    );
    let (params, return_ty) = signature(&module, "_f");
    assert_eq!(params[0], Ty::Int);
    assert!(
        matches!(&params[1], Ty::Instance(name) if name.as_str() == "list"),
        "{params:?}"
    );
    assert_eq!(return_ty, Ty::Int);
}

/// The marker entry never reaches the lowered module's alias table.
#[test]
fn the_ext_marker_is_not_a_type_alias_of_the_module() {
    let module = lower_ext("type Num = int\ndef _f(a: Num) -> int:\n    return a\n");
    let names: Vec<&str> = module
        .hir
        .type_aliases
        .iter()
        .map(|(name, _)| name.as_str())
        .collect();
    assert_eq!(names, vec!["Num"]);
}

/// The same sources lowered for a `native` artifact keep today's refusals.
#[test]
fn a_native_module_keeps_every_refusal() {
    for (source, code, message) in [
        (
            format!("{TYPING}def _f(a: Any) -> int:\n    return 1\n"),
            "T0002",
            "`Any` is not permitted in pycc code outside a declared interop boundary",
        ),
        (
            format!("{TYPING}def _f(a: Any[int]) -> int:\n    return 1\n"),
            "T0002",
            "`Any` is not permitted in pycc code outside a declared interop boundary",
        ),
        (
            "def _f(a: object) -> int:\n    return 1\n".to_string(),
            "C0001",
            "type annotation `object` is not supported yet",
        ),
        (
            "def _f(a: list) -> int:\n    return 1\n".to_string(),
            "C0001",
            "a bare `list` type annotation is not supported yet -- write the parameterized \
             form, e.g. `list[int]`",
        ),
    ] {
        let error = only_error(&source, false);
        assert_eq!(error.code, code, "{source}: {error:#?}");
        assert_eq!(error.message, message, "{source}");
    }
    // `List[Fraction]` is not collapsed natively: it keeps the D-105 gate's
    // `T0034`.
    let error = only_error(
        &format!(
            "{TYPING}from fractions import Fraction\ndef _f(a: List[Fraction]) -> int:\n    return 1\n"
        ),
        false,
    );
    assert_eq!(error.code, "T0034", "{error:#?}");
    // A subscript on an `object` alias keeps its `native` noun in both modes.
    for ext in [false, true] {
        let error = only_error(
            "type object = int\ndef _f(a: object[int]) -> int:\n    return 1\n",
            ext,
        );
        assert_eq!(error.code, "T0044", "{error:#?}");
        assert!(
            error
                .message
                .starts_with("type alias `object` is not subscriptable"),
            "{}",
            error.message
        );
    }
}
