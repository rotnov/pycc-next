//! `__class_getitem__` dispatch and class-name rebinding.

use super::*;

#[test]
fn class_getitem_dispatches_a_static_hook_in_value_position() {
    let src = format!("{CLASS_GETITEM_STATIC}\nvalue: int = C[3]\n");
    assert!(
        parse_check(&src).is_ok(),
        "`C[3]` must resolve through the class's own `__class_getitem__`"
    );
}

#[test]
fn class_getitem_dispatch_survives_the_generic_rewrite_pass() {
    // `check_and_resolve` runs `rewrite_generic_calls_in_expr`, whose
    // `Subscript` arm must not recurse into the bare class-name base:
    // doing so would infer it as a value and fail with T0021.
    //
    // The generic function is load-bearing: `monomorphize` returns early
    // for a module that declares no generic function and no protocol
    // parameter, and the rewrite pass would never run at all without it.
    let src = format!(
        "{CLASS_GETITEM_STATIC}
def ident[T](x: T) -> T:
    return x

echoed: int = ident(1)
value: int = C[3]
"
    );
    assert!(parse_check_resolve(&src).is_ok());
}

#[test]
fn a_class_name_attribute_read_survives_the_generic_rewrite_pass() {
    // The `AttrGet` arm of `rewrite_generic_calls_in_expr` shares the
    // `Subscript` arm's problem: `A.X` names the class in a position that is
    // not a value expression, so recursing into the base infers a bare class
    // name and fails with T0021. As above, the generic function is
    // load-bearing -- without it `monomorphize` returns before the rewrite
    // pass runs.
    let src = "\
class A:
    X: int = 2

def ident[T](x: T) -> T:
    return x

echoed: int = ident(1)
value: int = A.X
";
    assert!(parse_check_resolve(src).is_ok());
}

#[test]
fn a_class_name_static_method_call_survives_the_generic_rewrite_pass() {
    // The `MethodCall` arm's own spelling of the same skip: `A.m()` calls a
    // `@staticmethod` through the class name, so its base is not a value
    // expression either.
    let src = "\
class A:
    @staticmethod
    def m() -> int:
        return 2

def ident[T](x: T) -> T:
    return x

echoed: int = ident(1)
value: int = A.m()
";
    assert!(parse_check_resolve(src).is_ok());
}

#[test]
fn class_getitem_index_error_propagates_through_the_generic_rewrite_pass() {
    // The rewrite pass still descends into the *index* of a class-name
    // subscript, so an instantiation error there must surface rather than
    // being skipped along with the base.
    let src = format!(
        "{CLASS_GETITEM_STATIC}
def ident[T](x: T) -> T:
    return x

value: int = C[ident(1, 2)]
"
    );
    assert_eq!(parse_check_resolve(&src).unwrap_err().code, "T0021");
}

#[test]
fn monomorphize_propagates_an_instantiation_error_from_a_subscript_index() {
    // The rewrite pass descends into a subscript's index regardless of
    // whether its base is a bare class name, so an instantiation error
    // there must propagate. `check_and_resolve` rejects the bad call
    // before `monomorphize` ever sees it, so the module is built by hand
    // and handed to `monomorphize` directly, as the sibling
    // error-propagation tests above already do.
    let param = Ty::Param(Box::new("T".to_string()));
    let identity = generic_identity_fn(param.clone(), param);
    let top = HirItem::TopLevelStmt(HirStmt::ExprStmt(HirExpr::Subscript {
        base: Box::new(HirExpr::ListLiteral(vec![HirExpr::IntLiteral(1)])),
        index: Box::new(HirExpr::Call {
            callee: "identity".to_string(),
            args: vec![HirExpr::IntLiteral(1), HirExpr::IntLiteral(2)],
        }),
    }));
    let hir = HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![identity, top],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: Vec::new(),
    };
    assert_eq!(monomorphize(&hir).unwrap_err().code, "T0021");
}

#[test]
fn class_getitem_propagates_an_index_inference_error() {
    // The index is inferred before the hook is resolved, so an undefined
    // name there is reported as such instead of reaching the class hook.
    let src = format!("{CLASS_GETITEM_STATIC}\nvalue: int = C[missing]\n");
    assert_eq!(parse_check(&src).unwrap_err().code, "T0021");
}

#[test]
fn class_getitem_dispatches_a_classmethod_hook_in_value_position() {
    // CPython makes `__class_getitem__` an implicit classmethod, so the
    // explicit `@classmethod` spelling must resolve too.
    let src = "\
class C:
    def __init__(self) -> None:
        self.x = 1
    @classmethod
    def __class_getitem__(cls, key: int) -> int:
        return key + 1

value: int = C[3]
";
    assert!(parse_check(src).is_ok());
}

#[test]
fn class_getitem_result_type_is_the_hook_s_return_type() {
    // Binding the result to a mismatched annotation must fail, proving
    // the subscript's type comes from the hook rather than from a
    // container-element fallback.
    let src = format!("{CLASS_GETITEM_STATIC}\nvalue: str = C[3]\n");
    assert!(parse_check(&src).is_err());
}

#[test]
fn class_getitem_argument_type_is_checked_against_the_hook() {
    let src = format!("{CLASS_GETITEM_STATIC}\nvalue: int = C[\"k\"]\n");
    assert!(parse_check(&src).is_err());
}

#[test]
fn class_getitem_value_position_prefers_a_base_s_staticmethod_hook_over_a_derived_classmethod_override()
 {
    // Issue #693 deep-review, Finding 1: `resolve_static_or_class_method_call`
    // walks the full MRO checking `static_methods` first, and only if none
    // of the MRO's classes declare the hook there does it walk the full MRO
    // a second time checking `class_methods` -- so a base class's
    // `@staticmethod` hook always outranks a derived class's `@classmethod`
    // override, regardless of MRO order. `pycc_hir`'s annotation-position
    // resolver (`class_getitem_return_ty`) must replicate this exact
    // two-pass order rather than combining both tables into one MRO pass;
    // see `pycc_hir::tests::an_annotation_subscript_prefers_a_base_s_staticmethod_hook_over_a_derived_classmethod_override`
    // for the annotation-position half of this regression pair -- both
    // must resolve `D[3]` to `int` (the base's hook), never `str` (the
    // derived override).
    let src = "\
class C:
    @staticmethod
    def __class_getitem__(key: int) -> int:
        return key
    def __init__(self) -> None:
        self.x = 1

class D(C):
    @classmethod
    def __class_getitem__(cls, key: int) -> str:
        return \"overridden\"

value: int = D[3]
";
    assert!(
        parse_check(src).is_ok(),
        "`D[3]` must resolve through the base class `C`'s `@staticmethod` \
         hook (return type `int`), not `D`'s own `@classmethod` override \
         (return type `str`)"
    );
}

#[test]
fn subscripting_a_class_without_the_hook_is_rejected() {
    // CPython raises `TypeError: type 'C' is not subscriptable` here.
    let src = "\
class C:
    def __init__(self) -> None:
        self.x = 1

value: int = C[3]
";
    let err = parse_check(src).unwrap_err();
    assert_eq!(err.code, "T0044");
}

#[test]
fn a_value_binding_shadowing_a_class_name_indexes_as_a_value() {
    // Only *type aliases* collide with a class name in `pycc_hir`, so a
    // plain rebinding is accepted and `C[0]` must then read the list
    // element rather than dispatching a class hook.
    let src = format!("{CLASS_GETITEM_STATIC}\nC = [1, 2, 3]\nvalue: int = C[0]\n");
    assert!(
        parse_check(&src).is_ok(),
        "a rebound class name must index as the value it now holds"
    );
}

#[test]
fn a_local_shadowing_a_class_name_indexes_as_a_value() {
    let src = format!(
        "{CLASS_GETITEM_STATIC}
def f() -> int:
    C = [1, 2, 3]
    return C[0]

value: int = f()
"
    );
    assert!(parse_check(&src).is_ok());
}

#[test]
fn rebinding_a_class_name_after_the_read_is_rejected() {
    // CPython prints `2` here: `f()` runs before `A = D()` executes.
    let src = format!("{REBIND_OWN_ATTR}\nprint(f())\nA = D()\n");
    let err = parse_check(&src).unwrap_err();
    assert_eq!(err.code, "C0001");
}

#[test]
fn rebinding_a_class_name_before_the_read_is_rejected() {
    // CPython prints `1` here: `A` is the `D` instance by the time `f()`
    // runs. The environment pass 3 checks against is identical to the case
    // above, which is exactly why neither can be resolved.
    let src = format!("{REBIND_OWN_ATTR}\nA = D()\nprint(f())\n");
    let err = parse_check(&src).unwrap_err();
    assert_eq!(err.code, "C0001");
}

#[test]
fn rebinding_a_derived_class_name_after_the_read_is_rejected() {
    let src = format!("{REBIND_INHERITED_ATTR}\nprint(f())\nB = D()\n");
    let err = parse_check(&src).unwrap_err();
    assert_eq!(err.code, "C0001");
}

#[test]
fn rebinding_a_derived_class_name_before_the_read_is_rejected() {
    let src = format!("{REBIND_INHERITED_ATTR}\nB = D()\nprint(f())\n");
    let err = parse_check(&src).unwrap_err();
    assert_eq!(err.code, "C0001");
}

#[test]
fn rebinding_a_class_name_read_through_class_getitem_is_rejected() {
    // The `Subscript` dispatch site carries the same guard as the
    // `AttrGet` one above; inside a function body it rejects too.
    let src = format!(
        "{CLASS_GETITEM_STATIC}
def f() -> int:
    return C[0]

C = [1, 2, 3]
print(f())
"
    );
    let err = parse_check(&src).unwrap_err();
    assert_eq!(err.code, "C0001");
}

#[test]
fn rebinding_a_class_name_called_as_a_static_method_is_rejected() {
    // The `MethodCall` dispatch site, third of the three guards.
    let src = "\
class A:
    @staticmethod
    def m() -> int:
        return 2

class D:
    def __init__(self) -> None:
        self.x = 1

def f() -> int:
    return A.m()

A = D()
print(f())
";
    let err = parse_check(src).unwrap_err();
    assert_eq!(err.code, "C0001");
}

#[test]
fn a_parameter_shadowing_a_class_name_still_reads_the_parameter() {
    // The round-2/3 behavior the rejection above must not widen to: a
    // *parameter* (like a function-local) shadows the class without any
    // ordering ambiguity, because it is bound at the call, not by module
    // top-level code.
    let src = "\
class B:
    X: int = 2

class D:
    def __init__(self) -> None:
        self.X = 1

def f(B: D) -> int:
    return B.X

print(f(D()))
";
    assert!(
        parse_check(src).is_ok(),
        "a parameter shadowing a class name must still read the parameter"
    );
}
