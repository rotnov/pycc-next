//! #953: a concrete conforming argument is accepted for a protocol-typed
//! parameter of an instance-method call, and the call lowers.
//!
//! Two seams meet here, and both need in-crate coverage because the
//! integration tests under `tests/` do not count toward this crate's own
//! regions (D-014, `docs/TESTING.md`):
//!
//! - `class::check_call_args`' `structural` discriminator: `Some(env)` runs
//!   the environment-aware conformance predicate for the two
//!   instance-method-call sites, `None` keeps plain nominal assignability
//!   for the static-method, class-method, `super()`, constructor and
//!   exception-constructor sites.
//! - `monomorphize::specialize_protocol_method_call`: the method whose
//!   signature carries the protocol parameter is dropped from the module
//!   by `monomorphize_protocol_params`, so without a per-call-site
//!   specialization the accepted call reaches `pycc_mir` with no
//!   `$fn:<mangled>` binding at all.

use super::check_source;
use crate::monomorphize::specialize_protocol_method_call;
use crate::{Environment, check_and_resolve};
use pycc_hir::{HirClassDef, HirExpr, HirItem, HirModule, Ty};
use std::collections::{HashMap, HashSet};

/// A protocol whose member takes the protocol itself, and a conforming
/// class spelling the parameter the same way (#948's accepted shape).
const PRELUDE: &str = "from typing import Protocol\nclass P(Protocol):\n    def same(self, other: P) -> int: ...\nclass C:\n    def same(self, other: P) -> int:\n        return 5\n";

fn resolved_function_names(source: &str) -> Vec<String> {
    let module = pycc_parser::parse(source).expect("test fixture must parse");
    let hir = pycc_hir::lower_checked(&module).expect("test fixture must lower");
    let resolved = check_and_resolve(&hir).expect("fixture must type-check");
    resolved
        .items
        .iter()
        .filter_map(|item| match item {
            HirItem::Function { name, .. } => Some(name.clone()),
            _ => None,
        })
        .collect()
}

#[test]
fn a_conforming_argument_through_a_protocol_typed_receiver_is_accepted_and_specialized() {
    let names = resolved_function_names(&format!(
        "{PRELUDE}def main() -> None:\n    p: P = C()\n    print(p.same(C()))\nmain()\n"
    ));
    assert!(
        names.iter().any(|name| name == "0gen_C.same__P_C"),
        "the call site mints exactly one specialization: {names:?}"
    );
    assert!(
        !names.iter().any(|name| name == "C.same"),
        "the protocol-parameter original is dropped: {names:?}"
    );
}

#[test]
fn a_conforming_argument_through_a_concrete_receiver_is_accepted_and_specialized() {
    let names = resolved_function_names(&format!(
        "{PRELUDE}def main() -> None:\n    c = C()\n    print(c.same(C()))\nmain()\n"
    ));
    assert!(
        names.iter().any(|name| name == "0gen_C.same__P_C"),
        "{names:?}"
    );
}

#[test]
fn a_module_level_receiver_is_specialized_too() {
    // The `TopLevelStmt` arm has to grow its own environment as it walks:
    // without that, `p` is untyped when `p.same(C())` is reached and no
    // specialization is minted at all.
    let names = resolved_function_names(&format!("{PRELUDE}p: P = C()\nprint(p.same(C()))\n"));
    assert!(
        names.iter().any(|name| name == "0gen_C.same__P_C"),
        "{names:?}"
    );
}

#[test]
fn an_inherited_protocol_parameter_method_resolves_through_the_mro() {
    let names = resolved_function_names(
        "from typing import Protocol\nclass P(Protocol):\n    def val(self) -> int: ...\nclass Base:\n    def take(self, p: P) -> int:\n        return p.val()\nclass C(Base):\n    def val(self) -> int:\n        return 7\ndef main() -> None:\n    c = C()\n    print(c.take(C()))\nmain()\n",
    );
    assert!(
        names.iter().any(|name| name == "0gen_Base.take__P_C"),
        "{names:?}"
    );
}

#[test]
fn two_calls_with_the_same_concrete_type_share_one_specialization() {
    // The `seen.insert` false branch: the second call site rewrites to the
    // already-minted specialization instead of pushing a duplicate item.
    let names = resolved_function_names(&format!(
        "{PRELUDE}def main() -> None:\n    c = C()\n    print(c.same(C()))\n    print(c.same(C()))\nmain()\n"
    ));
    assert_eq!(
        names
            .iter()
            .filter(|name| *name == "0gen_C.same__P_C")
            .count(),
        1,
        "{names:?}"
    );
}

#[test]
fn a_method_without_protocol_parameters_is_left_alone() {
    // `protocol_funcs.get(&mangled)` misses: the ordinary method call is
    // not rewritten even though the module has a protocol parameter
    // elsewhere (which is what makes this pass run at all).
    let names = resolved_function_names(&format!(
        "{PRELUDE}class D:\n    def plain(self, n: int) -> int:\n        return n\ndef main() -> None:\n    d = D()\n    print(d.plain(1))\nmain()\n"
    ));
    assert!(names.iter().any(|name| name == "D.plain"), "{names:?}");
    assert!(
        !names.iter().any(|name| name.starts_with("0gen_")),
        "{names:?}"
    );
}

#[test]
fn a_receiver_that_is_not_a_class_instance_is_left_alone() {
    // `infer_expr_in` returns a non-`Instance` type for a container
    // receiver, so the arm bails before any MRO walk.
    let names = resolved_function_names(&format!(
        "{PRELUDE}def main() -> None:\n    xs = [1]\n    xs.append(2)\n    print(len(xs))\nmain()\n"
    ));
    assert!(
        !names.iter().any(|name| name.starts_with("0gen_")),
        "{names:?}"
    );
}

#[test]
fn a_non_conforming_argument_keeps_the_existing_t0021() {
    let err = check_source(&format!(
        "{PRELUDE}class D:\n    def other(self) -> int:\n        return 1\ndef main() -> None:\n    c = C()\n    print(c.same(D()))\nmain()\n"
    ))
    .unwrap_err();
    assert_eq!(err.code, "T0021");
    assert_eq!(err.message, "argument 1 of `same` expects `P`, got `D`");
    assert_eq!(err.help.as_deref(), Some("pass a `P` value"));
}

#[test]
fn a_constructor_still_uses_nominal_assignability() {
    // `check_call_args`' `structural: None` arm: the constructor site is
    // deliberately unchanged, because accepting a conforming concrete
    // argument there needs monomorphization support that does not exist
    // for the constructor lowering shape yet.
    let err = check_source(&format!(
        "{PRELUDE}class Holder:\n    def __init__(self, p: P, n: int) -> None:\n        self.n = n\ndef main() -> None:\n    h = Holder(C(), 2)\n    print(h.n)\nmain()\n"
    ))
    .unwrap_err();
    assert_eq!(err.code, "T0021");
    assert_eq!(err.message, "argument 1 of `Holder` expects `P`, got `C`");
}

#[test]
fn an_arity_mismatch_is_unchanged_on_both_predicates() {
    let err = check_source(&format!(
        "{PRELUDE}def main() -> None:\n    c = C()\n    print(c.same(C(), C()))\nmain()\n"
    ))
    .unwrap_err();
    assert_eq!(err.code, "T0021");
    assert_eq!(err.message, "`same` expects 1 argument(s), got 2");
}

// -- direct tests for the branches no source program can reach -----------

/// A class with one method entry, registered in a fresh environment under
/// `name`, with `receiver` bound to an instance of it.
fn env_with_class(name: &str, methods: Vec<(String, String)>) -> Environment {
    let mut env = Environment::new();
    env.bind_class(
        name.to_string(),
        HirClassDef {
            class_attrs: Vec::new(),
            exception_type_tag: None,
            name: name.to_string(),
            bases: Vec::new(),
            mro: vec![name.to_string()],
            attrs: Vec::new(),
            methods,
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
        },
    );
    env.bind("c".to_string(), Ty::Instance(Box::new(name.to_string())));
    env
}

fn call(
    env: &Environment,
    method: &str,
    args: Vec<HirExpr>,
    protocol_funcs: &HashMap<String, HirItem>,
) -> Option<HirExpr> {
    let mut specializations: Vec<HirItem> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    specialize_protocol_method_call(
        &HirExpr::Name("c".to_string()),
        method,
        &args,
        protocol_funcs,
        env,
        &[],
        &mut specializations,
        &mut seen,
    )
}

#[test]
fn a_receiver_whose_class_is_not_registered_is_left_alone() {
    let mut env = Environment::new();
    env.bind("c".to_string(), Ty::Instance(Box::new("Ghost".to_string())));
    assert!(call(&env, "same", Vec::new(), &HashMap::new()).is_none());
}

#[test]
fn a_method_missing_from_the_mro_is_left_alone() {
    let env = env_with_class("C", Vec::new());
    assert!(call(&env, "same", Vec::new(), &HashMap::new()).is_none());
}

#[test]
fn a_protocol_parameter_with_no_concrete_argument_is_left_alone() {
    // The resolved method *is* a protocol-parameter function, but the
    // argument's type is not a class instance, so the substitution list
    // stays empty and the call is left as a `MethodCall`.
    let env = env_with_class("C", vec![("same".to_string(), "C.same".to_string())]);
    let mut protocol_funcs = HashMap::new();
    protocol_funcs.insert(
        "C.same".to_string(),
        HirItem::Function {
            name: "C.same".to_string(),
            params: vec![
                ("self".to_string(), Ty::Instance(Box::new("C".to_string()))),
                ("other".to_string(), Ty::Protocol(Box::new("P".to_string()))),
            ],
            return_ty: Ty::Int,
            body: Vec::new(),
        },
    );
    assert!(call(&env, "same", vec![HirExpr::IntLiteral(1)], &protocol_funcs).is_none());
}

#[test]
fn a_non_function_protocol_entry_is_left_alone() {
    // Defense in depth: `protocol_funcs` only ever holds
    // `HirItem::Function`s, so this arm is unreachable from any module.
    let env = env_with_class("C", vec![("same".to_string(), "C.same".to_string())]);
    let mut protocol_funcs = HashMap::new();
    protocol_funcs.insert(
        "C.same".to_string(),
        HirItem::TopLevelStmt(pycc_hir::HirStmt::ExprStmt(HirExpr::IntLiteral(0))),
    );
    assert!(call(&env, "same", Vec::new(), &protocol_funcs).is_none());
}

#[test]
fn an_empty_module_still_resolves() {
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: Vec::new(),
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    assert!(check_and_resolve(&hir).is_ok());
}

#[test]
fn a_non_protocol_parameter_and_a_missing_argument_are_both_skipped() {
    // Covers the two remaining conjuncts of the substitution filter: a
    // parameter whose type is not `Ty::Protocol`, and a protocol-typed
    // parameter with no argument at that index (an arity mismatch the
    // checker rejects separately, so monomorphization must not index
    // past the argument list).
    let env = env_with_class("C", vec![("same".to_string(), "C.same".to_string())]);
    let mut protocol_funcs = HashMap::new();
    protocol_funcs.insert(
        "C.same".to_string(),
        HirItem::Function {
            name: "C.same".to_string(),
            params: vec![
                ("self".to_string(), Ty::Instance(Box::new("C".to_string()))),
                ("n".to_string(), Ty::Int),
                ("other".to_string(), Ty::Protocol(Box::new("P".to_string()))),
            ],
            return_ty: Ty::Int,
            body: Vec::new(),
        },
    );
    assert!(call(&env, "same", vec![HirExpr::IntLiteral(1)], &protocol_funcs).is_none());
}

#[test]
fn a_static_method_still_uses_nominal_assignability() {
    // `check_call_args`' `structural: None` arm, reached through the
    // static-method call site (#954).
    let err = check_source(&format!(
        "{PRELUDE}class H:\n    @staticmethod\n    def take(p: P) -> int:\n        return 1\ndef main() -> None:\n    print(H.take(C()))\nmain()\n"
    ))
    .unwrap_err();
    assert_eq!(err.code, "T0021");
    assert_eq!(err.message, "argument 1 of `take` expects `P`, got `C`");
}

#[test]
fn a_class_method_still_uses_nominal_assignability() {
    let err = check_source(&format!(
        "{PRELUDE}class H:\n    @classmethod\n    def take(cls, p: P) -> int:\n        return 1\ndef main() -> None:\n    print(H.take(C()))\nmain()\n"
    ))
    .unwrap_err();
    assert_eq!(err.code, "T0021");
    assert_eq!(err.message, "argument 1 of `take` expects `P`, got `C`");
}

#[test]
fn a_super_forwarding_call_still_uses_nominal_assignability() {
    // `resolve_super_method_call`'s own `check_call_args` site keeps the
    // nominal predicate too, so a concrete argument forwarded through
    // `super()` is still `T0021` (#954).
    let err = check_source(&format!(
        "{PRELUDE}class Base:\n    def take(self, p: P) -> int:\n        return 1\nclass Sub(Base):\n    def go(self) -> int:\n        return super().take(C())\ndef main() -> None:\n    print(Sub().go())\nmain()\n"
    ))
    .unwrap_err();
    assert_eq!(err.code, "T0021");
    assert_eq!(err.message, "argument 1 of `take` expects `P`, got `C`");
}
