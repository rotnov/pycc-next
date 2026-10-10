//! Unit tests for the type-checking crate root.
//!
//! Relocated verbatim from `lib.rs`'s inline `#[cfg(test)] mod tests` under
//! AGENTS.md's decomposability rule (#544 Part 1). As a child module it still
//! sees the crate root's private items directly, so nothing needed widened
//! visibility; only the module's own location changed.

use super::*;

// `lib.rs` used to import this for its own inference code; that code now
// lives in `expr.rs`, which imports it directly, so the crate root no longer
// carries the import these tests inherited through `use super::*`.
use crate::binop::numeric_result_type;

// Cohesive test clusters live in their own child files so this
// already-oversized module does not grow (AGENTS.md decomposability rule):
// #934 for the protocol-return re-cover tests, and #695 for the
// constraint-collection, enum-loop-unrolling, exception-handling,
// generic-class-instantiation, generic-class-substitution,
// generic-method-dedup, generic-method-instantiation,
// generic-monomorphization-arms, import-alias, `__init__`-rank,
// `Optional[T]`-narrowing, pattern-matching, protocol-argument,
// `typing.TYPE_CHECKING`-marker and `typing.cast` clusters.
mod boolop;
mod buffer_producer;
mod compare_chain;
mod comprehension_expr;
mod constraints;
mod constructor_call;
mod empty_container_registry;
mod enum_unrolling;
mod exception_handling;
mod foreign_float_tuple;
mod generic_class_instantiation;
mod generic_class_substitution;
mod generic_method_dedup;
mod generic_method_instantiation;
mod generic_monomorphization_arms;
mod if_exp;
mod import_alias;
mod init_rank;
mod isinstance_narrowing;
mod method_call_return;
mod not_implemented;
mod object_none_return;
mod optional_narrowing;
mod pattern_matching;
mod protocol_argument;
mod protocol_return;
mod try_definite;
mod type_checking_marker;
mod typing_cast;
mod typing_legacy_aliases;
mod while_true_return;

// Topic submodules split out of this file.
mod basics;
mod memoryview;
mod walrus;
mod solver_fast_path;
mod annotated_private_locals;
mod assignment_and_declarations;
mod control_flow_binding;
mod comprehensions;
mod expr_inference;
mod container_methods;
mod builtin_calls;
mod dict_set_tuple_literals;
mod stmt_helpers;
mod scoping;
mod shadowing;
mod class_attributes;
mod function_checks;
mod function_scope_flow;
mod comprehensions_in_functions;
mod private_inference_binops;
mod private_signature_inference;
mod generics;
mod monomorphize_rewrite;
mod monomorphize_errors;
mod generic_recursion;
mod redefinition_and_mro;
mod solver_definite_assignment;
mod final_bindings;
mod protocols;
mod match_patterns;
mod try_raise_bindings;
mod walrus_solver;
mod try_raise_solver;
mod class_getitem;
mod unary_and_optional;

fn generic_identity_fn(param_ty: Ty, return_ty: Ty) -> HirItem {
    HirItem::Function {
        name: "identity".to_string(),
        params: vec![("x".to_string(), param_ty)],
        return_ty,
        body: vec![HirStmt::Return(Some(HirExpr::Name("x".to_string())))],
    }
}

// PR-13 Task 3 (D-133/D-134): full pipeline wiring -- `check`/
// `check_and_resolve` dispatching a generic call site through
// `instantiate_generic_call`, and `check_and_resolve`'s own
// `monomorphize` pass rewriting call sites and dropping/appending
// items so `pycc_mir::build` only ever sees ordinary concrete
// functions.

fn find_function<'a>(module: &'a HirModule, name: &str) -> Option<&'a HirItem> {
    module
        .items
        .iter()
        .find(|item| matches!(item, HirItem::Function { name: n, .. } if n == name))
}

fn count_function(module: &HirModule, name: &str) -> usize {
    module
        .items
        .iter()
        .filter(|item| matches!(item, HirItem::Function { name: n, .. } if n == name))
        .count()
}

// -- shared PEP 695 generic-class fixture helpers, used through
// `super::` only by the `generic_*` child modules (#695) --------------

/// Helper: builds a `HirModule` with a generic class `C[T]` whose
/// `__init__` takes `x: T` and stores `self.x = x`, plus a single
/// `C[int](1)` call site at module scope. This is the minimal module
/// that exercises the full `check_and_resolve` → `monomorphize` →
/// `instantiate_generic_class_methods` pipeline.
fn generic_class_module_with_call() -> HirModule {
    let param = Ty::Param(Box::new("T".to_string()));
    let init = HirItem::Function {
        name: "C.__init__".to_string(),
        params: vec![
            ("self".to_string(), Ty::Instance(Box::new("C".to_string()))),
            ("x".to_string(), param),
        ],
        return_ty: Ty::None,
        body: vec![HirStmt::AttrSet {
            base: HirExpr::Name("self".to_string()),
            attr: "x".to_string(),
            value: HirExpr::Name("x".to_string()),
        }],
    };
    let class_def = HirClassDef {
        class_attrs: Vec::new(),
        exception_type_tag: None,
        name: "C".to_string(),
        bases: Vec::new(),
        mro: vec!["C".to_string()],
        attrs: vec![("x".to_string(), Ty::Param(Box::new("T".to_string())))],
        methods: vec![("__init__".to_string(), "C.__init__".to_string())],
        type_param: Some("T".to_string()),
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
    HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            init,
            HirItem::TopLevelStmt(HirStmt::ExprStmt(HirExpr::GenericClassInstantiate {
                class: "C".to_string(),
                type_arg: Ty::Int,
                args: vec![HirExpr::IntLiteral(1)],
            })),
        ],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: vec![("C".to_string(), class_def)],
    }
}

/// Helper: a `GenericClassInstantiate` expression for `C[int](1)`.
fn gci_expr() -> HirExpr {
    HirExpr::GenericClassInstantiate {
        class: "C".to_string(),
        type_arg: Ty::Int,
        args: vec![HirExpr::IntLiteral(1)],
    }
}

// -- #380: Protocol monomorphization and marker value tests ----------

/// Parses, lowers, and type-checks source code, returning the
/// resolved HIR or the diagnostic.
fn check_source(source: &str) -> Result<HirModule, pycc_diag::Diagnostic> {
    let module = pycc_parser::parse(source).expect("test fixture must parse");
    let hir = pycc_hir::lower_checked(&module).expect("test fixture must lower");
    check_and_resolve(&hir)
}

// -- #382 parse/check helpers, shared by the tests that stayed here (the
// `#382 coverage tests` run below) and, through `super::`, by the
// `exception_handling` and `enum_unrolling` child modules (#695) --

fn parse_check_resolve(src: &str) -> Result<HirModule, Diagnostic> {
    let module = pycc_parser::parse(src).expect("test fixture must parse");
    let hir = pycc_hir::lower_checked(&module).expect("lowering must succeed");
    check_and_resolve(&hir)
}

fn parse_check(src: &str) -> Result<(), Diagnostic> {
    let module = pycc_parser::parse(src).expect("test fixture must parse");
    let hir = pycc_hir::lower_checked(&module).expect("lowering must succeed");
    check(&hir)
}

// ------------------------------------------------------------------
// PEP 560 (#610): value-position `C[x]` dispatches to
// `C.__class_getitem__(x)`.
// ------------------------------------------------------------------

const CLASS_GETITEM_STATIC: &str = "\
class C:
    def __init__(self) -> None:
        self.x = 1
    @staticmethod
    def __class_getitem__(key: int) -> int:
        return key + 1
";

// -- #974 round 5: a module-scope rebinding of a class's own name ----
//
// A function body is checked (D-041 late binding) against the module
// environment as it stands after *all* top-level code has run, so
// `binding_state` cannot tell a rebinding that executes before the call
// from one that executes after it. CPython answers those two differently
// (`1` versus `2`), so neither resolution is correct for both and the read
// is rejected with `C0001` instead -- see `expr::class_name_dispatch`.
//
// All four shapes below are rejected: rebind-before and rebind-after,
// crossed with a class attribute the class declares itself and one it
// inherits. The `Subscript` and `MethodCall` dispatch sites carry the same
// guard and get one case each. The parameter-shadow case that closes the
// set is the round-2/3 behavior this rejection must **not** widen to.

/// A class, a same-shaped stand-in to rebind its name to, and a function
/// reading the class attribute through the bare class name.
const REBIND_OWN_ATTR: &str = "\
class A:
    X: int = 2

class D:
    def __init__(self) -> None:
        self.X = 1

def f() -> int:
    return A.X
";

/// [`REBIND_OWN_ATTR`] with the read going through a *derived* class, so
/// the attribute resolves through the MRO walk #974 added.
const REBIND_INHERITED_ATTR: &str = "\
class A:
    X: int = 2

class B(A):
    pass

class D:
    def __init__(self) -> None:
        self.X = 1

def f() -> int:
    return B.X
";
