//! #1418: `return NotImplemented` in a comparison method of an `ext`
//! module, from the type checker's side.
//!
//! `pycc_hir` widens such a method's return type to the CPython object and
//! lowers the admitted `return NotImplemented` to `HirExpr::NotImplemented`;
//! these tests pin what the checker makes of that: an object-valued body
//! passes, a native scalar return is boxed (#1475) while a container
//! return stays `T0022` with the widened-return help, the
//! widened method's result flows as an object, a native `==` on two
//! instances stays refused, and a `native` build keeps its `T0021`.

/// Lowers `source` as an `ext` module (`ext == true`) or a `native` one.
fn lower(source: &str, ext: bool) -> pycc_hir::HirModule {
    let parsed = pycc_parser::parse(source).expect("test fixture must parse");
    let mut resolved = pycc_hir::ResolvedImports::default();
    resolved.set_ext_module(ext);
    pycc_hir::lower_module(&parsed, &resolved, None)
        .unwrap_or_else(|diagnostics| panic!("{source:?} must lower: {diagnostics:#?}"))
        .hir
}

/// The first diagnostic `source` is refused with.
fn refusal(source: &str, ext: bool) -> pycc_diag::Diagnostic {
    crate::check_all(&lower(source, ext))
        .expect_err(source)
        .swap_remove(0)
}

/// A class `C` holding an object `v`, whose `__eq__` returns
/// `NotImplemented` for a `None` operand and `tail` otherwise.
fn widened(tail: &str) -> String {
    format!(
        "class C:\n\
         \x20   def __init__(self, v: object) -> None:\n        self.v = v\n\
         \x20   def __eq__(self, other: object) -> bool:\n\
         \x20       if other is None:\n            return NotImplemented\n\
         \x20       return {tail}\n"
    )
}

#[test]
fn an_object_valued_widened_method_is_admitted() {
    let source = format!(
        "{}\
         \x20   def eq(self, o: object) -> object:\n        return self.__eq__(o)\n",
        widened("self.v == other")
    );
    crate::check_all(&lower(&source, true))
        .unwrap_or_else(|diagnostics| panic!("{source:?}: {diagnostics:#?}"));
}

#[test]
fn a_native_scalar_return_in_a_widened_method_is_boxed() {
    // #1475: a `bool` is boxed into the widened `object` return.
    crate::check_all(&lower(&widened("True"), true))
        .unwrap_or_else(|diagnostics| panic!("{diagnostics:#?}"));
}

#[test]
fn a_container_return_in_a_widened_method_is_t0022_with_the_help() {
    let diagnostic = refusal(&widened("[1]"), true);
    assert_eq!(diagnostic.code, "T0022", "{diagnostic:#?}");
    // The check phase's wording: the solver defers a concrete value
    // returned into a declared `object` to it (#1475).
    assert_eq!(
        diagnostic.message,
        "expected return type `object`, got `list[int]`"
    );
    assert_eq!(
        diagnostic.help.as_deref(),
        Some(pycc_hir::WIDENED_RETURN_HELP)
    );
}

#[test]
fn a_widened_method_falling_off_its_end_keeps_its_own_help() {
    let source = "class C:\n\
                  \x20   def __eq__(self, other: object) -> bool:\n\
                  \x20       if other is None:\n            return NotImplemented\n";
    let diagnostic = refusal(source, true);
    assert_eq!(diagnostic.code, "T0022", "{diagnostic:#?}");
    assert!(
        diagnostic
            .message
            .contains("can exit without returning `object`"),
        "{diagnostic:#?}"
    );
    assert_ne!(
        diagnostic.help.as_deref(),
        Some(pycc_hir::WIDENED_RETURN_HELP)
    );
}

#[test]
fn the_widened_method_result_flows_as_the_object() {
    let source = format!(
        "{}\
         \x20   def eq(self, o: object) -> bool:\n        return self.__eq__(o)\n",
        widened("self.v == other")
    );
    let diagnostic = refusal(&source, true);
    assert_eq!(diagnostic.code, "T0022", "{diagnostic:#?}");
    // Since #1420 the solver links `eq`'s declared `bool` to the method
    // call's widened `object` return, so the refusal is the solver's
    // wording rather than the check phase's.
    assert_eq!(
        diagnostic.message,
        "return type mismatch: expected `bool`, found `object`"
    );
    // `eq` itself returns no `NotImplemented`, so the help is not its.
    assert_ne!(
        diagnostic.help.as_deref(),
        Some(pycc_hir::WIDENED_RETURN_HELP)
    );
}

#[test]
fn a_native_compare_of_two_instances_stays_refused() {
    let source = format!(
        "{}\ndef _same(a: C, b: C) -> bool:\n    return a == b\n",
        widened("self.v == other")
    );
    let diagnostic = refusal(&source, true);
    assert_eq!(diagnostic.code, "T0021", "{diagnostic:#?}");
    assert_eq!(diagnostic.message, "cannot compare `C` and `C`");
}

#[test]
fn a_native_build_keeps_its_t0021() {
    let source = "class C:\n    def __init__(self, v: int) -> None:\n        self.v = v\n\
                  \x20   def __eq__(self, other: C) -> bool:\n        return NotImplemented\n";
    let diagnostic = refusal(source, false);
    assert_eq!(diagnostic.code, "T0021", "{diagnostic:#?}");
    assert_eq!(diagnostic.message, "name `NotImplemented` is not defined");
}

#[test]
fn a_set_element_refusal_names_the_widened_return() {
    let source = "class C:\n    def __init__(self, v: int) -> None:\n        self.v = v\n\
                  \x20   def __hash__(self) -> int:\n        return self.v\n\
                  \x20   def __eq__(self, other: C) -> bool:\n        return NotImplemented\n\
                  \ndef _f() -> int:\n    s = {C(1), C(2)}\n    return len(s)\n";
    let diagnostic = refusal(source, true);
    assert_eq!(diagnostic.code, "C0001", "{diagnostic:#?}");
    let help = diagnostic.help.expect("the set-element refusal has a help");
    assert!(
        help.ends_with(
            "bases; its return is the CPython object (an `-> object` annotation, or a \
             `return NotImplemented`, which makes it so whatever the annotation says, #1418)"
        ),
        "{help}"
    );
}
