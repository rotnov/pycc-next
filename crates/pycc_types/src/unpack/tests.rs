//! Typing of a tuple-unpacking assignment's value (Part 1 of #891).

use pycc_diag::Span;
use pycc_hir::ImportBinding;

fn check(source: &str) -> Result<(), Vec<pycc_diag::Diagnostic>> {
    let module = pycc_parser::parse(source).expect("test source must parse");
    let hir = pycc_hir::lower_checked(&module).expect("test source must lower");
    crate::check_all(&hir).map(|_| ())
}

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

fn single(result: Result<(), Vec<pycc_diag::Diagnostic>>, source: &str) -> pycc_diag::Diagnostic {
    let diagnostics = result.expect_err(source);
    assert_eq!(diagnostics.len(), 1, "{diagnostics:?} for {source}");
    diagnostics.into_iter().next().unwrap()
}

/// A native tuple of exactly the target count unpacks, and each name takes
/// its own element's type: `n + 1` needs an `int` and `f * 2.0` a `float`.
#[test]
fn a_tuple_of_matching_arity_unpacks_into_typed_names() {
    for source in [
        "t = (1, 2.5)\nn, f = t\nm = n + 1\ng = f * 2.0\n",
        "a, b = 1, 2\na, b = b, a\nc = a + b\n",
        "[a, b, c] = (1, 2, 3)\nd = a + c\n",
        "a, = (7,)\nb = a + 1\n",
        "def f(p: tuple[int, float]) -> float:\n    i, j = p\n    return i + j\n",
        "def g() -> tuple[int, int]:\n    return 1, 2\nx = a, b = g()\nc = a + b\n",
    ] {
        if let Err(diagnostics) = check(source) {
            panic!("must type-check: {source}\n{diagnostics:?}");
        }
    }
}

/// A tuple of the wrong length is `T0055`, in CPython's own wording.
#[test]
fn a_tuple_of_the_wrong_arity_is_t0055() {
    for (source, message, help) in [
        (
            "t = (1, 2, 3)\na, b = t\n",
            "too many values to unpack (expected 2, got 3)",
            "the tuple has 3 element(s); write exactly 3 target name(s)",
        ),
        (
            "t = (1,)\na, b = t\n",
            "not enough values to unpack (expected 2, got 1)",
            "the tuple has 1 element(s); write exactly 1 target name(s)",
        ),
    ] {
        let diagnostic = single(check(source), source);
        assert_eq!(diagnostic.code, "T0055", "{diagnostic:?}");
        assert_eq!(diagnostic.message, message);
        assert_eq!(diagnostic.help.as_deref(), Some(help));
    }
}

/// A CPython object unpacks into object names, at module level, in a
/// function, through a chain, and in an unannotated helper -- which only
/// the constraint solver types, through its `Unpack` arm passing the
/// value's term through.
#[test]
fn an_object_unpacks_into_object_names() {
    for source in [
        "a, b = numpy.pi\nn = len(a)\n",
        "def f() -> int:\n    o = numpy.pi\n    x, y, z = o\n    return len(z)\n",
        "w = a, b = numpy.pi\n",
        "def _h():\n    a, b = numpy.pi\n    return b\n\n\nn = len(_h())\n",
    ] {
        if let Err(diagnostics) = check_foreign(source) {
            panic!("must type-check: {source}\n{diagnostics:?}");
        }
    }
}

/// Any other value type is refused with `C0001` naming the type.
#[test]
fn a_non_tuple_non_object_value_is_refused() {
    for (source, ty) in [
        ("xs = [1, 2]\na, b = xs\n", "list[int]"),
        ("a, b = 'ab'\n", "str"),
        ("a, b = 3\n", "int"),
        (
            "class P:\n    def __init__(self) -> None:\n        self.x = 1\na, b = P()\n",
            "P",
        ),
    ] {
        let diagnostic = single(check(source), source);
        assert_eq!(diagnostic.code, "C0001", "{diagnostic:?}");
        assert_eq!(
            diagnostic.message,
            format!("unpacking a `{ty}` value into names is not supported yet"),
            "{source}"
        );
        assert!(
            diagnostic
                .help
                .as_deref()
                .is_some_and(|help| help.contains("fixed-length `tuple[...]`")),
            "{diagnostic:?}"
        );
    }
}

/// A tuple display mixing `int` and `str` is not a native tuple at all, so
/// it is refused before the unpack is typed (`T0039`), deliberately.
#[test]
fn a_tuple_display_with_a_str_element_keeps_its_own_refusal() {
    let source = "a, b = 1, 'x'\n";
    let diagnostic = single(check(source), source);
    assert_ne!(diagnostic.code, "T0055", "{diagnostic:?}");
    assert_ne!(diagnostic.code, "C0001", "{diagnostic:?}");
}
