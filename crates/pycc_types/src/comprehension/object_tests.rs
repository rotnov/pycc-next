//! Typing of a list or set comprehension whose iterable is a CPython object
//! (Part 1 of #1255), with `numpy` bound as a foreign import.

use pycc_diag::Span;
use pycc_hir::ImportBinding;

fn check_foreign(source: &str) -> Result<(), Vec<pycc_diag::Diagnostic>> {
    let module = pycc_parser::parse(source).expect("test source must parse");
    let mut hir = pycc_hir::lower_checked(&module).expect("test source must lower");
    hir.imports.push(ImportBinding::Foreign {
        local_name: "numpy".to_string(),
        module_path: "numpy".to_string(),
        from: None,
        site: pycc_hir::ForeignImportSite::Item(0),
        span: Span::new(0, 0),
    });
    crate::check_all(&hir).map(|_| ())
}

fn assert_refused(source: &str, code: &str, phrase: &str) {
    let diagnostics = check_foreign(source).expect_err(source);
    assert_eq!(diagnostics.len(), 1, "{diagnostics:?} for {source}");
    assert_eq!(diagnostics[0].code, code, "{diagnostics:?} for {source}");
    assert!(
        diagnostics[0].message.contains(phrase),
        "{diagnostics:?} for {source}"
    );
}

/// Every admitted shape: the statement and expression forms, a filter, each
/// packable element type, a module body and a function body (the lark
/// shape: `try`/`except KeyError` around `d[k].keys()` with a method-call
/// filter), and an unannotated helper returning one, which only the
/// constraint solver types. The result is an object: `len()` of it and
/// passing it to a CPython call both type-check.
#[test]
fn a_comprehension_over_an_object_is_an_object() {
    for source in [
        "xs = [s for s in numpy.pi]\n",
        "xs = [s for s in numpy.pi.keys() if s.isupper()]\nn = len(xs)\n",
        "e = {s for s in numpy.pi.keys() if s}\nnumpy.array(e)\n",
        "xs = [1 for s in numpy.pi]\n",
        "xs = [1.5 for s in numpy.pi]\n",
        "xs = [True for s in numpy.pi]\n",
        "xs = {'a' for s in numpy.pi}\n",
        "print(len([s for s in numpy.pi]))\n",
        "def f(k: int) -> int:\n    \
         t = numpy.pi\n    \
         try:\n        e = {s for s in t[k].keys() if s.isupper()}\n    \
         except KeyError:\n        e = numpy.pi\n    \
         numpy.array(e)\n    \
         return len(e)\n",
        "def g() -> int:\n    t = numpy.pi\n    return len([k for k in t.keys()])\n",
        "def _h():\n    return [s for s in numpy.pi.keys()]\n\n\nn = len(_h())\n",
    ] {
        if let Err(diagnostics) = check_foreign(source) {
            panic!("must type-check: {source}\n{diagnostics:?}");
        }
    }
}

/// An iterable expression that is not a CPython object keeps a `C0001`: a
/// native container must still be bound to a name first.
#[test]
fn a_native_iterable_expression_is_refused() {
    for (source, ty) in [
        ("xs = [k for k in [1, 2, 3]]\n", "list[int]"),
        ("xs = {k for k in 'abc'}\n", "str"),
        (
            "def f() -> int:\n    return len([k for k in [1]])\n",
            "list[int]",
        ),
    ] {
        assert_refused(
            source,
            "C0001",
            &format!(
                "a CPython object is supported so far as a comprehension's iterable, got an expression of type `{ty}`"
            ),
        );
    }
}

/// A dict comprehension over an object, and an element with no packer, are
/// refused with `I0404`.
#[test]
fn a_dict_or_unpackable_comprehension_over_an_object_is_refused() {
    assert_refused(
        "d = {s: 1 for s in numpy.pi}\n",
        "I0404",
        "a dict comprehension over a CPython object",
    );
    assert_refused(
        "xs = [[1] for s in numpy.pi]\n",
        "I0404",
        "collecting a `list[int]` element into a CPython list",
    );
    assert_refused(
        "xs = {None for s in numpy.pi}\n",
        "I0404",
        "collecting a `None` element into a CPython set",
    );
}

/// The iterable's own type error is reported, not masked by the
/// comprehension.
#[test]
fn an_ill_typed_iterable_reports_its_own_error() {
    let diagnostics = check_foreign("xs = [k for k in numpy.pi + [1]]\n").expect_err("ill-typed");
    assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
    assert_ne!(diagnostics[0].code, "C0001", "{diagnostics:?}");
}

fn resolve_foreign(source: &str) -> Result<pycc_hir::HirModule, Vec<pycc_diag::Diagnostic>> {
    let module = pycc_parser::parse(source).expect("test source must parse");
    let mut hir = pycc_hir::lower_checked(&module).expect("test source must lower");
    hir.imports.push(ImportBinding::Foreign {
        local_name: "numpy".to_string(),
        module_path: "numpy".to_string(),
        from: None,
        site: pycc_hir::ForeignImportSite::Item(0),
        span: Span::new(0, 0),
    });
    crate::check_and_resolve_all(&hir)
}

/// An object comprehension: `[_v0 for _v0 in <iterable>]`.
fn object_comprehension(iterable: pycc_hir::HirExpr) -> pycc_hir::HirExpr {
    pycc_hir::HirExpr::Comprehension(Box::new(pycc_hir::HirComprehension {
        var: "_v0".to_string(),
        iter: pycc_hir::CompIter::Iterable(Box::new(iterable)),
        cond: None,
        elt: pycc_hir::CompElt::List(pycc_hir::HirExpr::Name("_v0".to_string())),
    }))
}

/// `monomorphize`'s generic-call rewrite types an object comprehension as an
/// object, walking its iterable. No source reaches this arm today: a module
/// that both uses a foreign name and defines a generic function is refused by
/// `monomorphize`'s foreign-less environment (#1105) before the walk, so the
/// arm is pinned directly.
#[test]
fn the_generic_rewrite_types_an_object_comprehension_as_an_object() {
    let mut env = crate::Environment::new();
    let mut expr = object_comprehension(pycc_hir::HirExpr::IntLiteral(1));
    let mut instantiations = Vec::new();
    let mut seen = std::collections::HashSet::new();
    let ty = crate::rewrite_generic_calls_in_expr(
        &mut env,
        &[],
        &mut expr,
        &mut instantiations,
        &mut seen,
    );
    assert_eq!(ty, Ok(pycc_hir::Ty::Object));
    assert!(instantiations.is_empty());
    assert_eq!(expr, object_comprehension(pycc_hir::HirExpr::IntLiteral(1)));
}

/// The protocol-call rewrite walks an object comprehension's iterable too,
/// so a protocol call there could not escape it; unreachable from source for
/// the same #1105 reason, so pinned directly.
#[test]
fn the_protocol_rewrite_walks_an_object_comprehension() {
    let env = crate::Environment::new();
    let mut item = pycc_hir::HirItem::Function {
        name: "f".to_string(),
        params: Vec::new(),
        return_ty: pycc_hir::Ty::None,
        body: vec![pycc_hir::HirStmt::ExprStmt(object_comprehension(
            pycc_hir::HirExpr::IntLiteral(1),
        ))],
    };
    let mut specializations = Vec::new();
    let mut seen = std::collections::HashSet::new();
    crate::rewrite_protocol_calls_in_specialization(
        &env,
        &mut item,
        &std::collections::HashMap::new(),
        &mut specializations,
        &mut seen,
    );
    assert!(specializations.is_empty());
    assert!(matches!(
        item,
        pycc_hir::HirItem::Function { ref body, .. }
            if body == &[pycc_hir::HirStmt::ExprStmt(object_comprehension(
                pycc_hir::HirExpr::IntLiteral(1),
            ))]
    ));
}

/// A generic call over a native iterable expression keeps the iterable's own
/// `C0001`: the check phase refuses it before `monomorphize` could type the
/// loop variable.
#[test]
fn a_generic_call_over_a_native_iterable_expression_keeps_its_c0001() {
    let diagnostics =
        resolve_foreign("def g[T](x: T) -> T:\n    return x\n\n\nxs = [g(k) for k in [1, 2]]\n")
            .expect_err("a native iterable expression is refused");
    assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
    assert_eq!(diagnostics[0].code, "C0001", "{diagnostics:?}");
    assert!(
        diagnostics[0]
            .message
            .contains("got an expression of type `list[int]`"),
        "{diagnostics:?}"
    );
}
