//! Class headers, name collisions, type parameters and bases.

use super::*;

// -- lower_class: class-level shape checks -----------------------------

#[test]
fn a_decorated_class_is_unsupported() {
    assert_c0001(
        "@some_decorator\nclass C:\n    def __init__(self) -> None:\n        return\n",
    );
}

#[test]
fn a_generic_class_with_one_type_param_is_supported() {
    // PEP 695 (#387): `class C[T]:` with exactly one type parameter is
    // now supported. The type parameter `T` is recorded in
    // `HirClassDef::type_param` for later monomorphization.
    let hir =
        lower_ok("class C[T]:\n    def __init__(self, x: T) -> None:\n        self.x = x\n");
    assert_eq!(hir.class_defs.len(), 1);
    assert_eq!(hir.class_defs[0].1.type_param, Some("T".to_string()));
}

#[test]
fn a_generic_class_with_two_type_params_is_unsupported() {
    assert_c0001("class C[T, U]:\n    def __init__(self) -> None:\n        return\n");
}

#[test]
fn a_class_with_a_keyword_argument_is_unsupported() {
    assert_c0001("class C(metaclass=Meta):\n    def __init__(self) -> None:\n        return\n");
}

#[test]
fn a_class_with_empty_parens_and_no_bases_is_supported() {
    // `class C():` is syntactically distinct from `class C:` (upstream
    // parses `arguments: Some(Arguments { args: [], keywords: [] })`)
    // but semantically equivalent -- no inheritance -- so it must not
    // be rejected merely because `arguments` is `Some(_)`.
    let hir = lower_ok("class C():\n    def __init__(self) -> None:\n        return\n");
    assert_eq!(hir.class_defs.len(), 1);
}

#[test]
fn redefining_a_class_name_at_module_scope_is_unsupported() {
    // D-154 Part 1's own post-merge review finding: two module-level
    // classes sharing a name would each lower their own `__init__` to
    // the identical mangled `<Name>.__init__` function name, colliding
    // silently in `pycc_types`'/`pycc_mir`'s own `HashMap`-collected
    // class tables downstream rather than producing a clean diagnostic.
    // Mirrors `redefining_init_in_one_class_body_is_unsupported`
    // above, one level up (module scope rather than one class body).
    assert_c0001(
        "class C:\n    def __init__(self) -> None:\n        return\nclass C:\n    def __init__(self) -> None:\n        return\n",
    );
}

#[test]
fn a_class_name_colliding_with_an_earlier_function_name_is_unsupported() {
    // D-068 review finding on #385: without this check, `class Foo`
    // below would silently, permanently shadow the earlier `def Foo()`
    // at every call site -- `pycc_types::Environment` checks
    // `env.lookup_class(callee)` before the ordinary function lookup
    // (`crates/pycc_types/src/lib.rs`), so `Foo()` would always resolve
    // to the class instantiation and the function would become
    // unreachable, with no diagnostic ever produced.
    let diagnostic = lower_checked(&crate::pycc_parser_test_helper::parse(
        "def Foo() -> None:\n    return\nclass Foo:\n    def __init__(self) -> None:\n        return\n",
    ))
    .unwrap_err();
    assert_eq!(diagnostic.code, "C0001");
    assert!(
        diagnostic
            .message
            .contains("class `Foo` collides with a function of the same name"),
        "unexpected message: {}",
        diagnostic.message
    );
}

#[test]
fn a_function_name_colliding_with_an_earlier_class_name_is_unsupported() {
    // The reverse order of
    // `a_class_name_colliding_with_an_earlier_function_name_is_unsupported`
    // above: the class comes first, the function second.
    let diagnostic = lower_checked(&crate::pycc_parser_test_helper::parse(
        "class Foo:\n    def __init__(self) -> None:\n        return\ndef Foo() -> None:\n    return\n",
    ))
    .unwrap_err();
    assert_eq!(diagnostic.code, "C0001");
    assert!(
        diagnostic
            .message
            .contains("function `Foo` collides with a class of the same name"),
        "unexpected message: {}",
        diagnostic.message
    );
}

#[test]
fn a_class_name_colliding_with_a_type_alias_is_unsupported() {
    let diagnostic = lower_checked(&crate::pycc_parser_test_helper::parse(
        "type Foo = int\nclass Foo:\n    def __init__(self) -> None:\n        return\n",
    ))
    .unwrap_err();
    assert_eq!(diagnostic.code, "C0001");
    assert!(
        diagnostic
            .message
            .contains("class `Foo` collides with a type alias of the same name"),
        "unexpected message: {}",
        diagnostic.message
    );
}

#[test]
fn a_class_name_colliding_with_an_earlier_value_binding_is_unsupported() {
    // #974 round 6: a class statement has no `HirItem` of its own, so
    // the type checker's sequential top-level pass never re-binds the
    // name back to the class -- a later read would resolve through the
    // stale value. Reject the collision here instead.
    let diagnostic = lower_checked(&crate::pycc_parser_test_helper::parse(
        "Foo = 1\nclass Foo:\n    def __init__(self) -> None:\n        return\n",
    ))
    .unwrap_err();
    assert_eq!(diagnostic.code, "C0001");
    assert!(
        diagnostic
            .message
            .contains("class `Foo` collides with a value of the same name"),
        "unexpected message: {}",
        diagnostic.message
    );
}

#[test]
fn a_value_binding_of_another_name_does_not_block_a_later_class() {
    // The negative direction of the check above: the earlier top-level
    // statement binds a *different* name, so the class lowers normally.
    let module = lower_checked(&crate::pycc_parser_test_helper::parse(
        "Bar = 1\nclass Foo:\n    def __init__(self) -> None:\n        return\n",
    ))
    .expect("an unrelated value binding must not block a later class");
    assert!(module.class_defs.iter().any(|(name, _)| name == "Foo"));
}

#[test]
fn a_class_name_colliding_with_a_module_import_is_unsupported() {
    let diagnostic = lower_checked(&crate::pycc_parser_test_helper::parse(
        "import math\nclass math:\n    def __init__(self) -> None:\n        return\n",
    ))
    .unwrap_err();
    assert_eq!(diagnostic.code, "C0001");
    assert!(
        diagnostic
            .message
            .contains("class `math` collides with an import of the same name"),
        "unexpected message: {}",
        diagnostic.message
    );
}

#[test]
fn a_class_name_colliding_with_a_symbol_import_is_unsupported() {
    // Exercises `import_local_name`'s other `ImportBinding` variant
    // (`Symbol`, from `from <module> import <name>`) -- the test above
    // only ever reaches the `Module` variant (`import math`), leaving
    // the `Symbol` arm of `import_local_name`'s own or-pattern
    // structurally unreachable under D-014's 100%-region coverage gate.
    let diagnostic = lower_checked(&crate::pycc_parser_test_helper::parse(
        "from math import sqrt\nclass sqrt:\n    def __init__(self) -> None:\n        return\n",
    ))
    .unwrap_err();
    assert_eq!(diagnostic.code, "C0001");
    assert!(
        diagnostic
            .message
            .contains("class `sqrt` collides with an import of the same name"),
        "unexpected message: {}",
        diagnostic.message
    );
}

#[test]
fn a_type_alias_colliding_with_an_earlier_class_name_is_unsupported() {
    // The reverse order of
    // `a_class_name_colliding_with_a_type_alias_is_unsupported` above:
    // the class comes first, the `type X = ...` alias second -- D-068
    // review finding on #385's second round: without this check, `type
    // Foo = int` below would silently establish a second, alias-shaped
    // `Foo` binding alongside the class, with no diagnostic.
    let diagnostic = lower_checked(&crate::pycc_parser_test_helper::parse(
        "class Foo:\n    def __init__(self) -> None:\n        return\ntype Foo = int\n",
    ))
    .unwrap_err();
    assert_eq!(diagnostic.code, "C0001");
    assert!(
        diagnostic
            .message
            .contains("type alias `Foo` collides with a class of the same name"),
        "unexpected message: {}",
        diagnostic.message
    );
}

#[test]
fn a_legacy_type_alias_colliding_with_an_earlier_class_name_is_unsupported() {
    // Same reverse-direction collision as
    // `a_type_alias_colliding_with_an_earlier_class_name_is_unsupported`
    // above, exercised through the legacy `X: TypeAlias = <expr>`
    // spelling (`lower_legacy_type_alias_ann_assign`) instead of `type X
    // = <expr>` (`lower_type_alias_stmt`) -- the two are lowered by
    // independent functions in `import.rs`, each needing its own check and
    // its own regression test.
    let diagnostic = lower_checked(&crate::pycc_parser_test_helper::parse(
        "class Foo:\n    def __init__(self) -> None:\n        return\nFoo: TypeAlias = int\n",
    ))
    .unwrap_err();
    assert_eq!(diagnostic.code, "C0001");
    assert!(
        diagnostic
            .message
            .contains("type alias `Foo` collides with a class of the same name"),
        "unexpected message: {}",
        diagnostic.message
    );
}

#[test]
fn a_module_import_colliding_with_an_earlier_class_name_is_unsupported() {
    // The reverse order of
    // `a_class_name_colliding_with_a_module_import_is_unsupported`
    // above: the class comes first, `import math` second.
    let diagnostic = lower_checked(&crate::pycc_parser_test_helper::parse(
        "class math:\n    def __init__(self) -> None:\n        return\nimport math\n",
    ))
    .unwrap_err();
    assert_eq!(diagnostic.code, "C0001");
    assert!(
        diagnostic
            .message
            .contains("import `math` collides with a class of the same name"),
        "unexpected message: {}",
        diagnostic.message
    );
}

#[test]
fn a_symbol_import_colliding_with_an_earlier_class_name_is_unsupported() {
    // The reverse order of
    // `a_class_name_colliding_with_a_symbol_import_is_unsupported`
    // above: the class comes first, `from math import sqrt` second.
    // Also exercises `bound.iter().map(import_local_name).find(..)`'s
    // own multi-binding search (`from math import pi, sqrt` binds two
    // names in one statement) finding the colliding name when it is not
    // the first one bound.
    let diagnostic = lower_checked(&crate::pycc_parser_test_helper::parse(
        "class sqrt:\n    def __init__(self) -> None:\n        return\nfrom math import pi, sqrt\n",
    ))
    .unwrap_err();
    assert_eq!(diagnostic.code, "C0001");
    assert!(
        diagnostic
            .message
            .contains("import `sqrt` collides with a class of the same name"),
        "unexpected message: {}",
        diagnostic.message
    );
}

// PEP 695 (#387 Part 3): a generic class's __init__ parameter typed `T`
// seeds an attribute slot with `Ty::Param("T")`, which is substituted
// at monomorphization time.
#[test]
fn generic_class_init_param_seeds_param_typed_slot() {
    let hir =
        lower_ok("class C[T]:\n    def __init__(self, x: T) -> None:\n        self.x = x\n");
    assert_eq!(hir.class_defs[0].1.type_param, Some("T".to_string()));
    assert_eq!(
        hir.class_defs[0].1.attrs,
        vec![("x".to_string(), Ty::Param(Box::new("T".to_string())))]
    );
}

// PEP 695 (#387 Part 3): `TypeVarTuple` (`*Ts`) and `ParamSpec` (`**P`)
// type parameters are not supported — `type_param_name`'s `?` error path
// (the uncovered region at line 120) is exercised by both forms.
#[test]
fn a_generic_class_with_a_type_var_tuple_is_unsupported() {
    assert_c0001("class C[*Ts]:\n    def __init__(self) -> None:\n        return\n");
}

#[test]
fn a_generic_class_with_a_param_spec_is_unsupported() {
    assert_c0001("class C[**P]:\n    def __init__(self) -> None:\n        return\n");
}

#[test]
fn duplicate_bases_are_rejected() {
    let module = crate::pycc_parser_test_helper::parse(
        "class A:\n    def __init__(self) -> None:\n        return\nclass B(A, A):\n    def __init__(self) -> None:\n        return\n",
    );
    let diagnostic = lower_checked(&module).unwrap_err();
    assert_eq!(diagnostic.code, "C0001");
    assert!(
        diagnostic.message.contains("lists base `A` more than once"),
        "unexpected message: {}",
        diagnostic.message
    );
}

#[test]
fn a_generic_class_with_base_classes_is_rejected() {
    let module = crate::pycc_parser_test_helper::parse(
        "class Base:\n    def __init__(self) -> None:\n        return\nclass C[T](Base):\n    def __init__(self, x: T) -> None:\n        self.x = x\n",
    );
    let diagnostic = lower_checked(&module).unwrap_err();
    assert_eq!(diagnostic.code, "C0001");
    assert!(
        diagnostic
            .message
            .contains("generic class `C` with base classes"),
        "unexpected message: {}",
        diagnostic.message
    );
}
