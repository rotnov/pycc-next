//! Typing of an attribute store and an attribute `del` on a CPython object
//! (Part 2 of #1443, #1457).

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

fn assert_admitted(source: &str) {
    if let Err(diagnostics) = check_foreign(source) {
        panic!("must type-check: {source}\n{diagnostics:?}");
    }
}

fn assert_refused(source: &str, code: &str, phrase: &str) -> pycc_diag::Diagnostic {
    let diagnostics = check_foreign(source).expect_err(source);
    assert_eq!(diagnostics.len(), 1, "{diagnostics:?} for {source}");
    assert_eq!(diagnostics[0].code, code, "{diagnostics:?} for {source}");
    assert!(
        diagnostics[0].message.contains(phrase),
        "{diagnostics:?} for {source}"
    );
    diagnostics[0].clone()
}

const BOX: &str = "class Box:\n    def __init__(self, n: int) -> None:\n        self.n = n\n\n\n";

/// Every value a method-call argument may be -- a packable scalar, another
/// object, `None` and a carriable class instance -- may be stored, in a
/// module body, a function body, an unannotated helper and a generic
/// function.
#[test]
fn an_object_attribute_store_admits_every_argument_type() {
    for source in [
        "numpy.pi.n = 1\n".to_string(),
        "numpy.pi.n = 1.5\n".to_string(),
        "numpy.pi.n = True\n".to_string(),
        "numpy.pi.n = 'a'\n".to_string(),
        "numpy.pi.n = None\n".to_string(),
        "numpy.pi.n = numpy.e\n".to_string(),
        "o = numpy.pi\no.n = o\n".to_string(),
        format!("{BOX}numpy.pi.box = Box(1)\n"),
        "def f(v: int) -> None:\n    o = numpy.pi\n    o.n = v\n".to_string(),
        "def _h(v):\n    numpy.pi.n = v\n\n\n_h(1)\n".to_string(),
        "def g[T](x: T) -> T:\n    numpy.pi.n = 1\n    return x\n\n\nn = g(1)\n".to_string(),
    ] {
        assert_admitted(&source);
    }
}

/// A value with no packer is refused with the method-call argument rule's
/// `I0404`, naming the attribute store.
#[test]
fn an_object_attribute_store_of_an_unpackable_value_is_refused() {
    assert_refused(
        "numpy.pi.xs = [1, 2]\n",
        "I0404",
        "passing a `list[int]` argument to a CPython object's attribute store",
    );
    assert_refused(
        "def f() -> None:\n    numpy.pi.d = [1]\n",
        "I0404",
        "passing a `list[int]` argument to a CPython object's attribute store",
    );
}

/// An empty display stored on an object has no element type to infer: it
/// keeps the empty literal's own `T0003` in a module body and in a function
/// body, rather than reaching an empty-container slot inference.
#[test]
fn an_empty_list_stored_on_an_object_is_refused() {
    for source in [
        "numpy.pi.xs = []\n",
        "def f() -> None:\n    numpy.pi.xs = []\n",
    ] {
        let diagnostics = check_foreign(source).expect_err(source);
        assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
        assert_eq!(diagnostics[0].code, "T0003", "{diagnostics:?}");
    }
}

/// A store through an object inside `__init__` gives the class no slot:
/// only a `self` base declares one, so reading `x` off an instance is still
/// an unknown attribute.
#[test]
fn an_object_store_in_init_declares_no_slot() {
    let source = "class C:\n    def __init__(self) -> None:\n        self.n = 1\n        \
                  numpy.pi.x = 2\n\n\nc = C()\nm = c.n\nk = c.x\n";
    let diagnostics = check_foreign(source).expect_err(source);
    assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
    assert!(diagnostics[0].message.contains("`x`"), "{diagnostics:?}");
}

/// A store on a value that is neither an object, a class instance nor a
/// protocol is a `T0043` rather than a field lookup on a non-class.
#[test]
fn a_store_on_a_builtin_value_is_t0043() {
    assert_refused(
        "x: int = 1\nx.n = 2\n",
        "T0043",
        "cannot assign an attribute on `int`",
    );
}

/// `del o.x` is admitted on an object in a module body, a function body, an
/// unannotated helper (the constraint solver's arm, which also walks the
/// base so a helper called from it is inferred) and a generic function, and
/// with a generic function call, a protocol-parameter call or a generic
/// class instantiation in the base (the monomorphizer's rewrites and
/// class-instantiation collector).
#[test]
fn an_object_attribute_delete_is_admitted() {
    for source in [
        "del numpy.pi.n\n",
        "o = numpy.pi\ndel o.n, o.m\n",
        "def f() -> None:\n    o = numpy.pi\n    del o.n\n",
        "def _h():\n    del numpy.pi.n\n\n\n_h()\n",
        "def _f(x):\n    return x\n\n\ndel _f(numpy.pi).n\n",
        "def g[T](x: T) -> T:\n    return x\n\n\ndel numpy.pi[g(1):].n\n",
        "def g[T](x: T) -> T:\n    return x\n\n\ndef f() -> None:\n    del numpy.pi[g(1):].n\n",
        "def g[T](x: T) -> T:\n    del numpy.pi.n\n    return x\n\n\nn = g(1)\n",
        "class Box[T]:\n    def __init__(self, x: T) -> None:\n        self.x = x\n\n    \
         def size(self) -> int:\n        return 1\n\n\n\
         del numpy.pi[Box(1).size():].n\n",
    ] {
        assert_admitted(source);
    }
}

/// Any other base keeps a `C0001` located at the attribute target, in a
/// module body and in a function body.
#[test]
fn a_native_attribute_delete_is_a_located_c0001() {
    let source = format!("{BOX}b = Box(1)\ndel b.n\n");
    let diagnostic = assert_refused(
        &source,
        "C0001",
        "a `del` of an attribute (`del obj.attr`) is supported only on a CPython object, \
         not on `Box`",
    );
    let start = u32::try_from(source.find("b.n").expect("target")).expect("small");
    assert_eq!(diagnostic.span, Some(Span::new(start, start + 3)));
    assert_refused(
        "def f() -> None:\n    xs = [1]\n    del xs.n\n",
        "C0001",
        "not on `list[int]`",
    );
}

/// A base the constraint solver rejects inside an unannotated helper
/// surfaces that rejection: the solver's `DeleteAttr` arm walks the base.
#[test]
fn a_solver_error_in_an_attribute_delete_base_is_reported() {
    // `_identity` is inferred `int -> int` from its first call; the nested
    // `str` call conflicts with it, which only the solver reports.
    let source = "def _identity(value):\n    return value\n\n\n\
                  def _sink(value: int) -> int:\n    return value\n\n\n\
                  _identity(1)\n\n\n\
                  def _probe():\n    del numpy.pi[_sink(_identity(\"wrong\")):].n\n";
    let diagnostics = check_foreign(source).expect_err(source);
    assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
    assert_eq!(diagnostics[0].code, "T0021", "{diagnostics:?}");
}

/// A call of a protocol-parameter function in a `del` base is rewritten to
/// its specialization, and an explicit generic class instantiation there
/// is collected and specialized: monomorphization drops the original `proto_fn` item, so an
/// unrewritten call would dangle.
#[test]
fn a_protocol_call_and_a_generic_class_in_an_attribute_delete_base_are_specialized() {
    let source = "from typing import Protocol\nclass P(Protocol):\n    def foo(self) -> int: ...\n\
                  class C:\n    def __init__(self) -> None:\n        self.x = 0\n    \
                  def foo(self) -> int:\n        return 1\n\
                  class Box[T]:\n    def __init__(self, x: T) -> None:\n        self.x = x\n\
                  def proto_fn(p: P) -> C:\n    return C()\n\
                  def caller() -> None:\n    del proto_fn(c).x\n\
                  c = C()\ndel proto_fn(c).x\ndel Box[int](1).x\ncaller()\n";
    // The rewrite is type-blind and runs before the `del` base check, so a
    // native base exercises it without seeding a foreign import.
    let hir =
        pycc_hir::lower_checked(&pycc_parser::parse(source).expect("parses")).expect("lowers");
    let rewritten = format!(
        "{:?}",
        crate::monomorphize::monomorphize(&hir).expect("monomorphizes")
    );
    assert!(
        !rewritten.contains("callee: \"proto_fn\""),
        "a dangling `proto_fn` call survived: {rewritten}"
    );
    assert!(
        rewritten.contains("DeleteAttr { base: Call { callee: \"0gen_Box__T_int\""),
        "the `Box[int]` instantiation was not specialized: {rewritten}"
    );
}
